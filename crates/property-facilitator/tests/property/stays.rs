use property_facilitator::{add_user, assign_device, serve, MemoryRetention, Pack, Role};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::common::{client, hotels, login, settings, temp_db, Desk, PASSWORD, TOKEN};

async fn pod_token(base: &str, db: &std::path::Path, device_id: &str, room: &str) -> String {
    let enroll = || async {
        match client()
            .post(format!("{base}/api/devices/enroll"))
            .json(&json!({ "device_id": device_id, "nonce": format!("{device_id}-nonce-0123456789"), "firmware": "t" }))
            .send()
            .await
        {
            Ok(response) => response.json::<Value>().await.unwrap_or(Value::Null),
            Err(error) => panic!("enroll: {error}"),
        }
    };
    let _ = enroll().await;
    if let Err(error) = assign_device(db, device_id, room) {
        panic!("assign: {error}");
    }
    enroll().await["token"].as_str().unwrap_or("").to_string()
}

async fn verify(base: &str, token: &str) -> Value {
    match client()
        .post(format!("{base}/api/devices/verify"))
        .bearer_auth(TOKEN)
        .json(&json!({ "token": token }))
        .send()
        .await
    {
        Ok(response) => response.json().await.unwrap_or(Value::Null),
        Err(error) => panic!("verify: {error}"),
    }
}

async fn stay_action(desk: &Desk, path: &str, fields: &[(&str, &str)]) -> StatusCode {
    let mut form: Vec<(&str, &str)> = vec![("csrf", desk.csrf.as_str())];
    form.extend_from_slice(fields);
    match client()
        .post(format!("{}{path}", desk.base))
        .header(reqwest::header::COOKIE, &desk.cookie)
        .form(&form)
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("POST {path}: {error}"),
    }
}

async fn purge_due(base: &str) -> Vec<Value> {
    match client()
        .get(format!("{base}/api/stays/purge-due"))
        .bearer_auth(TOKEN)
        .send()
        .await
    {
        Ok(response) => {
            assert_eq!(response.status(), StatusCode::OK);
            response
                .json::<Value>()
                .await
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
        }
        Err(error) => panic!("purge-due: {error}"),
    }
}

async fn staff(base: &str, db: &std::path::Path) -> Desk {
    if let Err(error) = add_user(db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    login(base, "maria", PASSWORD).await
}

#[tokio::test]
async fn a_room_has_memory_only_while_a_stay_is_open() {
    let db = temp_db("stay-open");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = pod_token(&running.url, &db, "pod-s1", "204").await;
    assert!(verify(&running.url, &token).await["memory_wing"].is_null());

    let desk = staff(&running.url, &db).await;
    assert_eq!(
        stay_action(&desk, "/api/stays", &[("room", "204")]).await,
        StatusCode::SEE_OTHER
    );
    let first = verify(&running.url, &token).await;
    let wing = first["memory_wing"].as_str().unwrap_or("").to_string();
    assert!(wing.starts_with("stay-"), "stay wing: {first}");
    assert_eq!(
        stay_action(&desk, "/api/stays", &[("room", "204")]).await,
        StatusCode::CONFLICT,
        "one open stay per room"
    );

    let stays = desk.get_json("/api/stays").await;
    let stay_id = stays[0]["id"].as_str().unwrap_or("").to_string();
    assert_eq!(
        stay_action(&desk, &format!("/api/stays/{stay_id}/close"), &[]).await,
        StatusCode::SEE_OTHER
    );
    assert!(verify(&running.url, &token).await["memory_wing"].is_null());

    assert_eq!(
        stay_action(&desk, "/api/stays", &[("room", "204")]).await,
        StatusCode::SEE_OTHER
    );
    let next = verify(&running.url, &token).await;
    assert_ne!(
        next["memory_wing"], first["memory_wing"],
        "the next guest never sees the last guest's memory"
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn keep_retention_never_purges_and_a_returning_guest_can_reuse_memory() {
    let db = temp_db("stay-keep");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = pod_token(&running.url, &db, "pod-s2", "12").await;
    let desk = staff(&running.url, &db).await;
    let _ = stay_action(&desk, "/api/stays", &[("room", "12")]).await;
    let original = verify(&running.url, &token).await["memory_wing"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let stays = desk.get_json("/api/stays").await;
    let stay_id = stays[0]["id"].as_str().unwrap_or("").to_string();
    let _ = stay_action(&desk, &format!("/api/stays/{stay_id}/close"), &[]).await;
    assert!(
        purge_due(&running.url).await.is_empty(),
        "keep never purges"
    );

    assert_eq!(
        stay_action(
            &desk,
            "/api/stays",
            &[("room", "12"), ("continue_from", stay_id.as_str())]
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        verify(&running.url, &token).await["memory_wing"],
        original.as_str()
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn wipe_on_close_queues_the_wing_for_purge_until_confirmed() {
    let db = temp_db("stay-wipe");
    let mut ward = settings(Pack::Ward, db.clone());
    ward.memory_retention = MemoryRetention::WipeOnClose;
    let running = match serve(ward).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = pod_token(&running.url, &db, "pod-s3", "4B").await;
    let desk = staff(&running.url, &db).await;
    let _ = stay_action(&desk, "/api/stays", &[("room", "4B")]).await;
    let wing = verify(&running.url, &token).await["memory_wing"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let stays = desk.get_json("/api/stays").await;
    let stay_id = stays[0]["id"].as_str().unwrap_or("").to_string();
    assert!(
        purge_due(&running.url).await.is_empty(),
        "open stays are never purged"
    );
    let _ = stay_action(&desk, &format!("/api/stays/{stay_id}/close"), &[]).await;

    let due = purge_due(&running.url).await;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0]["memory_wing"], wing.as_str());
    assert_eq!(
        stay_action(
            &desk,
            "/api/stays",
            &[("room", "4B"), ("continue_from", stay_id.as_str())]
        )
        .await,
        StatusCode::CONFLICT,
        "a wiped stay's memory cannot be continued"
    );

    let confirmed = match client()
        .post(format!("{}/api/stays/{stay_id}/purged", running.url))
        .bearer_auth(TOKEN)
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("purged: {error}"),
    };
    assert_eq!(confirmed, StatusCode::OK);
    assert!(purge_due(&running.url).await.is_empty());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn archive_retention_purges_only_after_the_window() {
    let db = temp_db("stay-archive");
    let mut hotel = hotels(db.clone(), None, Vec::new());
    hotel.memory_retention = MemoryRetention::ArchiveAfterDays(30);
    let running = match serve(hotel).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let desk = staff(&running.url, &db).await;
    let _ = stay_action(&desk, "/api/stays", &[("room", "7")]).await;
    let stays = desk.get_json("/api/stays").await;
    let stay_id = stays[0]["id"].as_str().unwrap_or("").to_string();
    let _ = stay_action(&desk, &format!("/api/stays/{stay_id}/close"), &[]).await;
    assert!(
        purge_due(&running.url).await.is_empty(),
        "not due for 30 days"
    );
    let closed = desk.get_json("/api/stays").await;
    let purge_after = closed[0]["purge_after_millis"].as_i64().unwrap_or(0);
    let closed_at = closed[0]["closed_millis"].as_i64().unwrap_or(0);
    assert_eq!(purge_after - closed_at, 30 * 24 * 60 * 60 * 1000);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn purge_api_needs_the_service_token() {
    let db = temp_db("stay-auth");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let status = match client()
        .get(format!("{}/api/stays/purge-due", running.url))
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("purge-due: {error}"),
    };
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}

#[test]
fn retention_is_read_from_property_json() {
    let dir = temp_db("retention").with_extension("d");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("dir: {error}");
    }
    let config = dir.join("property.json");
    let cases = [
        (json!({"pack": "hotels"}), MemoryRetention::Keep),
        (
            json!({"pack": "hotels", "memory_retention": {"mode": "archive", "archive_after_days": 14}}),
            MemoryRetention::ArchiveAfterDays(14),
        ),
        (
            json!({"pack": "hotels", "memory_retention": {"mode": "wipe_on_close"}}),
            MemoryRetention::WipeOnClose,
        ),
    ];
    for (body, expected) in cases {
        if let Err(error) = std::fs::write(&config, body.to_string()) {
            panic!("write: {error}");
        }
        match property_facilitator::Settings::load_or_default(Pack::Hotels, &config) {
            Ok(loaded) => assert_eq!(loaded.memory_retention, expected),
            Err(error) => panic!("load: {error}"),
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
