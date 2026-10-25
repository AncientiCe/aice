use property_facilitator::{add_user, serve, Role};
use serde_json::json;

use crate::common::{call_tool, hotels, login, temp_db, PASSWORD};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_desk_shows_what_the_guest_asked_for_and_when() {
    let db = temp_db("desk-page");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    call_tool(
        &running.url,
        "book_taxi",
        json!({ "room": "101", "destination": "the airport", "time": "06:00", "note": "<b>2 bags</b>" }),
    )
    .await;
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let html = desk.page("/desk").await;

    assert!(html.contains("Book taxi"), "{html}");
    assert!(html.contains("destination: the airport"), "{html}");
    assert!(html.contains("time: 06:00"), "{html}");
    assert!(
        html.contains("note: &lt;b&gt;2 bags&lt;/b&gt;"),
        "guest words must be escaped: {html}"
    );
    assert!(
        !html.contains("room: 101"),
        "the room is its own column: {html}"
    );
    assert!(html.contains("<time datetime=\""), "{html}");
    assert!(html.contains("just now"), "{html}");
    assert!(
        html.contains("<table class=\"stack\">"),
        "requests stack on a phone: {html}"
    );
    assert!(html.contains("<td class=\"actions\">"), "{html}");
    let _ = std::fs::remove_file(db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisors_see_readable_times_in_the_audit_log() {
    let db = temp_db("desk-audit-times");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    if let Err(error) = add_user(&db, "duty", PASSWORD, Role::Supervisor) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "duty", PASSWORD).await;
    let html = desk.page("/desk").await;

    assert!(!html.contains("At (ms)"), "{html}");
    let audit = html
        .split("<h2>Audit log</h2>")
        .nth(1)
        .unwrap_or_else(|| panic!("no audit log: {html}"));
    assert!(audit.contains("<time datetime=\""), "{audit}");
    let _ = std::fs::remove_file(db);
}
