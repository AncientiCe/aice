use property_facilitator::{add_user, assign_device, serve, Pack, Role};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::common::{client, login, settings, temp_db, Desk, PASSWORD, TOKEN};

async fn token_for(base: &str, db: &std::path::Path, device_id: &str, room: &str) -> String {
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

async fn wing(base: &str, token: &str) -> Value {
    match client()
        .post(format!("{base}/api/devices/verify"))
        .bearer_auth(TOKEN)
        .json(&json!({ "token": token }))
        .send()
        .await
    {
        Ok(response) => {
            response.json::<Value>().await.unwrap_or(Value::Null)["memory_wing"].clone()
        }
        Err(error) => panic!("verify: {error}"),
    }
}

async fn post(desk: &Desk, path: &str, fields: &[(&str, &str)]) -> StatusCode {
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

#[tokio::test]
async fn a_care_home_remembers_nothing_until_the_resident_consents() {
    let db = temp_db("consent-care");
    let running = match serve(settings(Pack::Care, db.clone())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = token_for(&running.url, &db, "pod-c1", "12").await;
    if let Err(error) = add_user(&db, "sam", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "sam", PASSWORD).await;

    assert_eq!(
        post(&desk, "/api/stays", &[("room", "12")]).await,
        StatusCode::SEE_OTHER
    );
    assert!(
        wing(&running.url, &token).await.is_null(),
        "no consent, no memory"
    );

    let stays = desk.get_json("/api/stays").await;
    let id = stays[0]["id"].as_str().unwrap_or("").to_string();
    assert_eq!(stays[0]["memory_consent"], false);
    assert_eq!(
        post(
            &desk,
            &format!("/api/stays/{id}/consent"),
            &[("memory_consent", "yes")]
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert!(
        wing(&running.url, &token).await.is_string(),
        "consent turns memory on"
    );

    assert_eq!(
        post(
            &desk,
            &format!("/api/stays/{id}/consent"),
            &[("memory_consent", "no")]
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert!(
        wing(&running.url, &token).await.is_null(),
        "withdrawn consent turns it off"
    );

    if let Err(error) = add_user(&db, "lee", PASSWORD, Role::Supervisor) {
        panic!("add user: {error}");
    }
    let supervisor = login(&running.url, "lee", PASSWORD).await;
    let audit = supervisor.get_json("/api/audit").await;
    let consent_events = audit
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter(|e| e["action"] == "stay_consent")
                .count()
        })
        .unwrap_or(0);
    assert_eq!(consent_events, 2);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hotels_record_consent_at_check_in() {
    let db = temp_db("consent-hotel");
    let running = match serve(settings(Pack::Hotels, db.clone())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = token_for(&running.url, &db, "pod-h1", "204").await;
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    assert_eq!(
        post(
            &desk,
            "/api/stays",
            &[("room", "204"), ("memory_consent", "yes")]
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert!(wing(&running.url, &token).await.is_string());
    let html = desk.page("/desk").await;
    assert!(
        html.contains("name=\"memory_consent\""),
        "the check-in form asks"
    );
    let _ = std::fs::remove_file(db);
}
