use property_facilitator::{add_user, serve, Role};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::common::{client, hotels, login, temp_db, Desk, PASSWORD, TOKEN};

async fn enroll(base: &str, device_id: &str, nonce: &str) -> (StatusCode, Value) {
    match client()
        .post(format!("{base}/api/devices/enroll"))
        .json(&json!({ "device_id": device_id, "nonce": nonce, "firmware": "1.4.0" }))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            (status, response.json().await.unwrap_or(Value::Null))
        }
        Err(error) => panic!("enroll: {error}"),
    }
}

async fn verify(base: &str, service_token: &str, device_token: &str) -> (StatusCode, Value) {
    match client()
        .post(format!("{base}/api/devices/verify"))
        .bearer_auth(service_token)
        .json(&json!({ "token": device_token }))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            (status, response.json().await.unwrap_or(Value::Null))
        }
        Err(error) => panic!("verify: {error}"),
    }
}

async fn assign(desk: &Desk, device_id: &str, room: &str) -> StatusCode {
    match client()
        .post(format!("{}/api/devices/{device_id}/assign", desk.base))
        .header(reqwest::header::COOKIE, &desk.cookie)
        .form(&[("csrf", desk.csrf.as_str()), ("room", room)])
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("assign: {error}"),
    }
}

async fn supervisor(base: &str, db: &std::path::Path) -> Desk {
    if let Err(error) = add_user(db, "lee", PASSWORD, Role::Supervisor) {
        panic!("add user: {error}");
    }
    login(base, "lee", PASSWORD).await
}

#[tokio::test]
async fn enrolled_pod_gets_a_token_once_a_supervisor_assigns_a_room() {
    let db = temp_db("enroll");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let (status, body) = enroll(&running.url, "pod-aa01", "nonce-one-0123456789").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "pending");

    let desk = supervisor(&running.url, &db).await;
    let html = desk.page("/desk").await;
    assert!(
        html.contains("pod-aa01"),
        "pending pod is shown on the desk"
    );
    assert_eq!(
        assign(&desk, "pod-aa01", "204").await,
        StatusCode::SEE_OTHER
    );

    let (status, body) = enroll(&running.url, "pod-aa01", "nonce-one-0123456789").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "active");
    assert_eq!(body["room"], "204");
    let token = body["token"].as_str().unwrap_or("").to_string();
    assert!(token.len() >= 32);

    let (status, again) = enroll(&running.url, "pod-aa01", "nonce-one-0123456789").await;
    assert_eq!(status, StatusCode::OK);
    assert!(again["token"].is_null(), "token is delivered only once");

    let (status, identity) = verify(&running.url, TOKEN, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["device_id"], "pod-aa01");
    assert_eq!(identity["room"], "204");

    let devices = desk.get_json("/api/devices").await;
    let pod = devices
        .as_array()
        .and_then(|list| list.iter().find(|device| device["device_id"] == "pod-aa01"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(pod["status"], "active");
    assert_eq!(pod["firmware"], "1.4.0");
    assert!(pod["last_seen_millis"].as_i64().unwrap_or(0) > 0);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn enrollment_needs_the_original_nonce() {
    let db = temp_db("nonce");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let (status, _) = enroll(&running.url, "pod-bb02", "real-nonce-0123456789").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let desk = supervisor(&running.url, &db).await;
    assert_eq!(assign(&desk, "pod-bb02", "12").await, StatusCode::SEE_OTHER);
    let (status, body) = enroll(&running.url, "pod-bb02", "stolen-nonce-012345678").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["token"].is_null());
    let (status, _) = enroll(&running.url, "bad id!", "real-nonce-0123456789").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = enroll(&running.url, "pod-cc03", "short").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn revoked_and_unknown_tokens_do_not_verify() {
    let db = temp_db("revoke");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let _ = enroll(&running.url, "pod-dd04", "nonce-four-0123456789").await;
    let desk = supervisor(&running.url, &db).await;
    assert_eq!(assign(&desk, "pod-dd04", "7").await, StatusCode::SEE_OTHER);
    let (_, body) = enroll(&running.url, "pod-dd04", "nonce-four-0123456789").await;
    let token = body["token"].as_str().unwrap_or("").to_string();

    let (status, _) = verify(&running.url, "wrong-service-token", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = verify(&running.url, TOKEN, "not-a-device-token").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert_eq!(
        desk.form("/api/devices/pod-dd04/revoke", Some(&desk.csrf))
            .await,
        StatusCode::SEE_OTHER
    );
    let (status, _) = verify(&running.url, TOKEN, &token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn reassigning_a_pod_moves_it_to_the_new_room() {
    let db = temp_db("reassign");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let _ = enroll(&running.url, "pod-ee05", "nonce-five-0123456789").await;
    let desk = supervisor(&running.url, &db).await;
    assert_eq!(
        assign(&desk, "pod-ee05", "101").await,
        StatusCode::SEE_OTHER
    );
    let (_, body) = enroll(&running.url, "pod-ee05", "nonce-five-0123456789").await;
    let token = body["token"].as_str().unwrap_or("").to_string();
    assert_eq!(
        assign(&desk, "pod-ee05", "305").await,
        StatusCode::SEE_OTHER
    );
    let (status, identity) = verify(&running.url, TOKEN, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["room"], "305");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn only_supervisors_manage_devices() {
    let db = temp_db("device-roles");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let _ = enroll(&running.url, "pod-ff06", "nonce-six-01234567890").await;
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let staff = login(&running.url, "maria", PASSWORD).await;
    assert_eq!(assign(&staff, "pod-ff06", "1").await, StatusCode::FORBIDDEN);
    let unauthenticated = match client()
        .get(format!("{}/api/devices", running.url))
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("devices get: {error}"),
    };
    assert_eq!(unauthenticated, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}
