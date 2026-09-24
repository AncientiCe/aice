use std::time::Duration;

use property_facilitator::{add_user, assign_device, list_devices, serve, Role};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::common::{client, hotels, login, temp_db, PASSWORD, TOKEN};

async fn provision(base: &str, db: &std::path::Path, device_id: &str, room: &str) {
    let enroll = || async {
        let _ = client()
            .post(format!("{base}/api/devices/enroll"))
            .json(&json!({ "device_id": device_id, "nonce": format!("{device_id}-nonce-0123456789"), "firmware": "1.0" }))
            .send()
            .await;
    };
    enroll().await;
    if let Err(error) = assign_device(db, device_id, room) {
        panic!("assign: {error}");
    }
    enroll().await;
}

async fn heartbeat(base: &str, token: &str, device_ids: &[&str]) -> StatusCode {
    match client()
        .post(format!("{base}/api/devices/heartbeat"))
        .bearer_auth(token)
        .json(&json!({ "device_ids": device_ids }))
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("heartbeat: {error}"),
    }
}

fn offline_tickets(tickets: &Value) -> Vec<Value> {
    tickets
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|ticket| ticket["tool_name"] == "device_offline")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_silent_pod_raises_one_escalated_ticket_and_recovers() {
    let db = temp_db("fleet");
    let mut settings = hotels(db.clone(), None, Vec::new());
    settings.device_offline_after = Duration::from_millis(400);
    let running = match serve(settings).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    provision(&running.url, &db, "pod-quiet", "12").await;
    provision(&running.url, &db, "pod-chatty", "14").await;

    for _ in 0..8 {
        assert_eq!(
            heartbeat(&running.url, TOKEN, &["pod-chatty"]).await,
            StatusCode::OK
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let tickets = match client()
        .get(format!("{}/api/tickets", running.url))
        .bearer_auth(TOKEN)
        .send()
        .await
    {
        Ok(response) => response.json::<Value>().await.unwrap_or(Value::Null),
        Err(error) => panic!("tickets: {error}"),
    };
    let offline = offline_tickets(&tickets);
    assert_eq!(offline.len(), 1, "only the quiet pod: {tickets}");
    assert_eq!(offline[0]["room"], "12");
    assert_eq!(offline[0]["status"], "escalated");

    if let Err(error) = add_user(&db, "lee", PASSWORD, Role::Supervisor) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "lee", PASSWORD).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let quiet_room_tickets = offline_tickets(&desk.get_json("/api/tickets").await)
        .into_iter()
        .filter(|ticket| ticket["room"] == "12")
        .count();
    assert_eq!(
        quiet_room_tickets, 1,
        "no repeat ticket while it stays offline"
    );

    assert_eq!(
        heartbeat(&running.url, TOKEN, &["pod-quiet"]).await,
        StatusCode::OK
    );
    let devices = list_devices(&db).unwrap_or_default();
    let quiet = devices
        .iter()
        .find(|device| device.device_id == "pod-quiet")
        .map(|device| device.status.clone());
    assert_eq!(quiet.as_deref(), Some("active"));
    let audit = desk.get_json("/api/audit").await;
    assert!(audit
        .as_array()
        .map(|events| events
            .iter()
            .any(|event| event["action"] == "device_online"))
        .unwrap_or(false));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn heartbeats_need_the_service_token() {
    let db = temp_db("fleet-auth");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    assert_eq!(
        heartbeat(&running.url, "wrong", &["pod-x"]).await,
        StatusCode::UNAUTHORIZED
    );
    let _ = std::fs::remove_file(db);
}
