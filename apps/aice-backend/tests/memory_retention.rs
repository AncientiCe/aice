//! Stay memory retention: the backend deletes a stay's wing when the
//! facilitator says it is due, and leaves every other wing alone.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aice_backend::{run_memory_retention_once, MemoryScope};
use palace::palace::Palace;
use property_facilitator::{
    close_stay, open_stay, serve, MemoryRetention, Pack, PropertyClient, Settings,
};

const SERVICE_TOKEN: &str = "retention-test-service-token-0123456789";

fn temp_db(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!(
        "aice-retention-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn drawers_in(palace: &Palace, wing: &str) -> i64 {
    palace
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM drawers WHERE wing = ?1",
            [wing],
            |row| row.get(0),
        )
        .unwrap_or(-1)
}

fn add_drawer(palace: &Palace, id: &str, wing: &str) {
    palace
        .conn()
        .execute(
            "INSERT INTO drawers (id, wing, room, content) VALUES (?1, ?2, 'conversation', 'guest said hello')",
            [id, wing],
        )
        .unwrap_or_else(|error| panic!("insert drawer: {error}"));
}

#[tokio::test]
async fn a_wiped_stay_loses_its_drawers_and_facts_and_nothing_else() {
    let db = temp_db("wipe");
    let mut settings = Settings::default_for(Pack::Ward);
    settings.bind = "127.0.0.1:0".to_string();
    settings.database_path = db.clone();
    settings.service_token = SERVICE_TOKEN.to_string();
    let running = serve(settings)
        .await
        .unwrap_or_else(|error| panic!("facilitator failed: {error}"));
    let client = PropertyClient::new(format!("{}/mcp", running.url), SERVICE_TOKEN)
        .unwrap_or_else(|error| panic!("client failed: {error}"));

    let leaving = open_stay(&db, "4B", None, true).unwrap_or_else(|error| panic!("open: {error}"));
    let staying = open_stay(&db, "5C", None, true).unwrap_or_else(|error| panic!("open: {error}"));

    let palace = Palace::open_in_memory().unwrap_or_else(|error| panic!("palace: {error}"));
    add_drawer(&palace, "d1", &leaving.memory_wing);
    add_drawer(&palace, "d2", &leaving.memory_wing);
    add_drawer(&palace, "d3", &staying.memory_wing);
    add_drawer(&palace, "d4", "journal");
    for wing in [&leaving.memory_wing, &staying.memory_wing] {
        palace::store::ensure_wing_registered(palace.conn(), wing)
            .unwrap_or_else(|error| panic!("register wing: {error}"));
        palace
            .conn()
            .execute(
                "INSERT INTO usage_events (ts, session_id, project, tool, wing, outcome)
                 VALUES ('2026-09-23T00:00:00Z', 's', 'aice', 'search', ?1, 'hit')",
                [wing.as_str()],
            )
            .unwrap_or_else(|error| panic!("usage row: {error}"));
    }
    palace
        .conn()
        .execute(
            "INSERT INTO gain_feedback (query_id, drawer_id, verdict, source) VALUES ('q', 'd1', 'useful', 'test')",
            [],
        )
        .unwrap_or_else(|error| panic!("feedback row: {error}"));
    let leaving_scope = MemoryScope::Stay(leaving.memory_wing.clone());
    let staying_scope = MemoryScope::Stay(staying.memory_wing.clone());
    for scope in [&leaving_scope, &staying_scope] {
        let subject = scope.entity("Alice").unwrap_or_default();
        let object = scope.entity("vegetarian").unwrap_or_default();
        palace
            .kg_add_triple(&subject, "is", &object, 0.9)
            .unwrap_or_else(|error| panic!("kg add: {error}"));
    }
    let palace = Arc::new(Mutex::new(palace));

    assert_eq!(
        run_memory_retention_once(&palace, &client)
            .await
            .unwrap_or_else(|error| panic!("retention: {error}")),
        0,
        "open stays are never purged"
    );

    close_stay(&db, &leaving.id, MemoryRetention::WipeOnClose)
        .unwrap_or_else(|error| panic!("close: {error}"));
    let purged = run_memory_retention_once(&palace, &client)
        .await
        .unwrap_or_else(|error| panic!("retention: {error}"));
    assert_eq!(purged, 1);

    {
        let guard = palace
            .lock()
            .unwrap_or_else(|error| panic!("lock: {error}"));
        assert_eq!(drawers_in(&guard, &leaving.memory_wing), 0);
        assert_eq!(drawers_in(&guard, &staying.memory_wing), 1);
        assert_eq!(drawers_in(&guard, "journal"), 1);
        let gone = guard
            .kg_query(&leaving_scope.entity("Alice").unwrap_or_default())
            .unwrap_or_default();
        assert!(gone.is_empty(), "the leaving guest's facts are deleted");
        let kept = guard
            .kg_query(&staying_scope.entity("Alice").unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(kept.len(), 1, "other guests' facts stay");
        let count = |sql: &str, arg: &str| -> i64 {
            guard
                .conn()
                .query_row(sql, [arg], |row| row.get(0))
                .unwrap_or(-1)
        };
        let wings = "SELECT COUNT(*) FROM wings WHERE name = ?1";
        let usage = "SELECT COUNT(*) FROM usage_events WHERE wing = ?1";
        assert_eq!(
            count(wings, &leaving.memory_wing),
            0,
            "wing registry entry removed"
        );
        assert_eq!(count(usage, &leaving.memory_wing), 0, "usage trail removed");
        assert_eq!(
            count(
                "SELECT COUNT(*) FROM gain_feedback WHERE drawer_id = ?1",
                "d1"
            ),
            0,
            "feedback on deleted drawers removed"
        );
        assert_eq!(count(wings, &staying.memory_wing), 1);
        assert_eq!(count(usage, &staying.memory_wing), 1);
    }

    assert_eq!(
        run_memory_retention_once(&palace, &client)
            .await
            .unwrap_or_else(|error| panic!("retention: {error}")),
        0,
        "the facilitator recorded the purge"
    );
    let _ = std::fs::remove_file(db);
}
