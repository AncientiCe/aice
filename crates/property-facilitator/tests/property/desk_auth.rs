use property_facilitator::{add_user, serve, Pack, Role};
use reqwest::StatusCode;
use serde_json::json;

use crate::common::{
    call_tool, client, hotels, login, post_mcp_with, settings, temp_db, ticket_id, try_login,
    PASSWORD, TOKEN,
};

fn tools_list() -> serde_json::Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})
}

#[tokio::test]
async fn mcp_rejects_missing_or_wrong_service_token() {
    let db = temp_db("mcp-token");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let (missing, _) = post_mcp_with(&running.url, None, tools_list()).await;
    assert_eq!(missing, StatusCode::UNAUTHORIZED);
    let (wrong, _) = post_mcp_with(&running.url, Some("not-the-token"), tools_list()).await;
    assert_eq!(wrong, StatusCode::UNAUTHORIZED);
    let (right, body) = post_mcp_with(&running.url, Some(TOKEN), tools_list()).await;
    assert_eq!(right, StatusCode::OK);
    assert!(body["result"]["tools"].is_array());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn desk_and_ticket_api_require_a_staff_session() {
    let db = temp_db("desk-login");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204" }),
    )
    .await;
    let id = ticket_id(&created);

    let desk = match client().get(format!("{}/desk", running.url)).send().await {
        Ok(response) => response,
        Err(error) => panic!("desk get: {error}"),
    };
    assert_eq!(desk.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        desk.headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/login")
    );

    for path in ["/api/tickets", "/api/audit"] {
        let response = match client().get(format!("{}{path}", running.url)).send().await {
            Ok(response) => response,
            Err(error) => panic!("GET {path}: {error}"),
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "GET {path}");
    }
    let ack = match client()
        .post(format!("{}/api/tickets/{id}/acknowledge", running.url))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("ack post: {error}"),
    };
    assert_eq!(ack.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn service_token_can_read_tickets_but_not_change_them() {
    let db = temp_db("token-read");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204" }),
    )
    .await;
    let id = ticket_id(&created);
    let read = match client()
        .get(format!("{}/api/tickets", running.url))
        .bearer_auth(TOKEN)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("tickets get: {error}"),
    };
    assert_eq!(read.status(), StatusCode::OK);
    let write = match client()
        .post(format!("{}/api/tickets/{id}/done", running.url))
        .bearer_auth(TOKEN)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("done post: {error}"),
    };
    assert_eq!(write.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn wrong_password_is_rejected_and_repeated_failures_lock_the_account() {
    let db = temp_db("lockout");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    for _ in 0..5 {
        let response = try_login(&running.url, "maria", "wrong password here").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .is_none());
    }
    let locked = try_login(&running.url, "maria", PASSWORD).await;
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);
    let unknown = try_login(&running.url, "nobody", PASSWORD).await;
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn short_passwords_and_duplicate_users_are_refused() {
    let db = temp_db("users");
    assert!(add_user(&db, "maria", "short", Role::Staff).is_err());
    assert!(add_user(&db, "maria", PASSWORD, Role::Staff).is_ok());
    assert!(add_user(&db, "maria", PASSWORD, Role::Supervisor).is_err());
    assert!(add_user(&db, " ", PASSWORD, Role::Staff).is_err());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn desk_forms_need_the_csrf_token() {
    let db = temp_db("csrf");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204" }),
    )
    .await;
    let id = ticket_id(&created);
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let path = format!("/api/tickets/{id}/acknowledge");
    assert_eq!(desk.form(&path, None).await, StatusCode::FORBIDDEN);
    assert_eq!(
        desk.form(&path, Some("forged")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        desk.form(&path, Some(&desk.csrf)).await,
        StatusCode::SEE_OTHER
    );
    let tickets = desk.get_json("/api/tickets").await;
    assert_eq!(tickets[0]["status"], "acknowledged");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn every_status_change_is_audited_with_the_staff_member() {
    let db = temp_db("audit");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204" }),
    )
    .await;
    let id = ticket_id(&created);
    for (name, role) in [("maria", Role::Staff), ("lee", Role::Supervisor)] {
        if let Err(error) = add_user(&db, name, PASSWORD, role) {
            panic!("add user: {error}");
        }
    }
    let staff = login(&running.url, "maria", PASSWORD).await;
    let (status, _) = staff
        .api_post(&format!("/api/tickets/{id}/acknowledge"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = staff.api_post(&format!("/api/tickets/{id}/done")).await;
    assert_eq!(status, StatusCode::OK);

    let forbidden = match client()
        .get(format!("{}/api/audit", running.url))
        .header(reqwest::header::COOKIE, &staff.cookie)
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("audit get: {error}"),
    };
    assert_eq!(forbidden, StatusCode::FORBIDDEN);

    let supervisor = login(&running.url, "lee", PASSWORD).await;
    let audit = supervisor.get_json("/api/audit").await;
    let events = audit.as_array().cloned().unwrap_or_default();
    let transitions: Vec<(String, String, String)> = events
        .iter()
        .filter(|event| event["action"] == "ticket_status")
        .map(|event| {
            (
                event["actor"].as_str().unwrap_or("").to_string(),
                event["from_status"].as_str().unwrap_or("").to_string(),
                event["to_status"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert!(transitions.contains(&(
        "maria".to_string(),
        "open".to_string(),
        "acknowledged".to_string()
    )));
    assert!(transitions.contains(&(
        "maria".to_string(),
        "acknowledged".to_string(),
        "done".to_string()
    )));
    assert!(events
        .iter()
        .any(|event| event["action"] == "login" && event["actor"] == "lee"));
    assert!(events
        .iter()
        .all(|event| event["ticket_id"].is_null() || event["ticket_id"] == id.as_str()));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn only_supervisors_can_reopen_a_done_ticket() {
    let db = temp_db("reopen");
    let running = match serve(settings(Pack::Care, db.clone())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(&running.url, "request_drink", json!({ "room": "3" })).await;
    let id = ticket_id(&created);
    for (name, role) in [("sam", Role::Staff), ("lee", Role::Supervisor)] {
        if let Err(error) = add_user(&db, name, PASSWORD, role) {
            panic!("add user: {error}");
        }
    }
    let staff = login(&running.url, "sam", PASSWORD).await;
    let (status, _) = staff.api_post(&format!("/api/tickets/{id}/done")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = staff.api_post(&format!("/api/tickets/{id}/reopen")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = staff.api_post(&format!("/api/tickets/{id}/escalate")).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let supervisor = login(&running.url, "lee", PASSWORD).await;
    let (status, reopened) = supervisor
        .api_post(&format!("/api/tickets/{id}/reopen"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reopened["status"], "open");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn logout_ends_the_session_and_desk_sends_security_headers() {
    let db = temp_db("logout");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let response = match client()
        .get(format!("{}/desk", running.url))
        .header(reqwest::header::COOKIE, &desk.cookie)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("desk get: {error}"),
    };
    let headers = response.headers();
    assert!(headers.get("content-security-policy").is_some());
    assert_eq!(
        headers
            .get("x-frame-options")
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        headers
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );

    assert_eq!(
        desk.form("/logout", Some(&desk.csrf)).await,
        StatusCode::SEE_OTHER
    );
    let after = match client()
        .get(format!("{}/desk", running.url))
        .header(reqwest::header::COOKIE, &desk.cookie)
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("desk get: {error}"),
    };
    assert_eq!(after, StatusCode::SEE_OTHER);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn oversized_request_bodies_are_rejected() {
    let db = temp_db("big-body");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let huge = "x".repeat(300 * 1024);
    let response = match client()
        .post(format!("{}/mcp", running.url))
        .bearer_auth(TOKEN)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(huge)
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("mcp post: {error}"),
    };
    assert_eq!(response, StatusCode::PAYLOAD_TOO_LARGE);
    let _ = std::fs::remove_file(db);
}
