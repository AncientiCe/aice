use std::sync::atomic::Ordering;

use property_facilitator::{
    add_user, client_service_token, serve, serve_fixed_mcp, Pack, PropertyClient, Role,
};
use serde_json::json;

use crate::common::{
    call_tool, hotels, login, post_mcp, settings, temp_db, ticket_id, PASSWORD, TOKEN,
};

#[tokio::test]
async fn hotel_tool_creates_an_open_ticket() {
    let db = temp_db("open");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204", "count": 2 }),
    )
    .await;
    let spoken = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(spoken.contains("request_extra_towels"));
    assert!(spoken.contains("204"));
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["structuredContent"]["status"], "open");

    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let tickets = desk.get_json("/api/tickets").await;
    let tickets = tickets.as_array().cloned().unwrap_or_default();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["room"], "204");
    assert_eq!(tickets[0]["tool_name"], "request_extra_towels");
    assert_eq!(tickets[0]["status"], "open");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn delegated_tool_calls_property_mcp_once() {
    let upstream = match serve_fixed_mcp(
        vec![
            "set_room_temperature".to_string(),
            "custom_minibar".to_string(),
        ],
        "thermostat set".to_string(),
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) => panic!("upstream failed: {error}"),
    };
    let db = temp_db("delegated");
    let running = match serve(hotels(
        db.clone(),
        Some(upstream.url.clone()),
        vec!["set_room_temperature".to_string()],
    ))
    .await
    {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let listed = post_mcp(
        &running.url,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let names = listed["result"]["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(names.iter().any(|name| name == "request_extra_towels"));
    assert!(names.iter().any(|name| name == "custom_minibar"));

    let response = call_tool(
        &running.url,
        "set_room_temperature",
        json!({ "room": "204", "celsius": 22 }),
    )
    .await;
    assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response["result"]["content"][0]["text"], "thermostat set");
    assert_eq!(response["result"]["structuredContent"]["status"], "open");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn property_mcp_outage_leaves_an_escalated_ticket() {
    let db = temp_db("outage");
    let running = match serve(hotels(
        db.clone(),
        Some("http://127.0.0.1:1/mcp".to_string()),
        vec!["request_extra_towels".to_string()],
    ))
    .await
    {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "18" }),
    )
    .await;
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"]["status"],
        "escalated"
    );
    assert!(response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("18"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn staff_desk_acknowledges_a_ticket() {
    let db = temp_db("desk");
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
    let ticket_id = ticket_id(&created);
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let html = desk.page("/desk").await;
    assert!(html.contains("204"));
    assert!(html.contains("request_extra_towels"));
    assert!(html.contains("aice-hotels desk"));
    let (status, updated) = desk
        .api_post(&format!("/api/tickets/{ticket_id}/acknowledge"))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(updated["status"], "acknowledged");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn care_fall_escalates_without_calling_upstream() {
    let upstream = match serve_fixed_mcp(Vec::new(), "should not run".to_string()).await {
        Ok(upstream) => upstream,
        Err(error) => panic!("upstream failed: {error}"),
    };
    let db = temp_db("care");
    let mut care = settings(Pack::Care, db.clone());
    care.property_mcp_url = Some(upstream.url.clone());
    care.delegated_tools = vec!["report_fall".to_string()];
    let running = match serve(care).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = call_tool(&running.url, "report_fall", json!({ "room": "12" })).await;
    assert_eq!(upstream.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        response["result"]["structuredContent"]["status"],
        "escalated"
    );
    if let Err(error) = add_user(&db, "sam", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "sam", PASSWORD).await;
    assert!(desk.page("/desk").await.contains("aice-care desk"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn ward_denies_medication_and_allows_wayfinding() {
    let db = temp_db("ward");
    let running = match serve(settings(Pack::Ward, db.clone())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let denied = call_tool(
        &running.url,
        "prescribe_medication",
        json!({ "room": "4B" }),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(denied["result"]["structuredContent"]["status"], "escalated");

    let allowed = call_tool(
        &running.url,
        "wayfinding",
        json!({ "room": "4B", "destination": "radiology" }),
    )
    .await;
    assert_eq!(allowed["result"]["isError"], false);
    assert_eq!(allowed["result"]["structuredContent"]["status"], "open");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn phone_extension_maps_to_a_room() {
    let db = temp_db("phone");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = call_tool(
        &running.url,
        "request_housekeeping",
        json!({ "extension": "101" }),
    )
    .await;
    assert!(response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("204"));
    assert_eq!(response["result"]["structuredContent"]["status"], "open");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn property_client_needs_the_service_token() {
    let db = temp_db("client");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let mcp = format!("{}/mcp", running.url);
    let good = match PropertyClient::new(&mcp, TOKEN) {
        Ok(client) => client,
        Err(error) => panic!("client: {error}"),
    };
    let tools = match good.list_tools().await {
        Ok(tools) => tools,
        Err(error) => panic!("list tools: {error}"),
    };
    assert!(tools.iter().any(|tool| tool == "request_extra_towels"));
    let created = match good
        .call_tool("request_extra_towels", Some("204"), None, &json!({}))
        .await
    {
        Ok(created) => created,
        Err(error) => panic!("call tool: {error}"),
    };
    assert_eq!(created.status, "open");

    let bad = match PropertyClient::new(&mcp, "wrong-token") {
        Ok(client) => client,
        Err(error) => panic!("client: {error}"),
    };
    assert!(bad.list_tools().await.is_err());
    let _ = std::fs::remove_file(db);
}

#[test]
fn client_token_file_must_hold_a_real_token() {
    let path = temp_db("token").with_extension("token");
    if let Err(error) = std::fs::write(&path, "short") {
        panic!("write: {error}");
    }
    assert!(client_service_token(Some(&path)).is_err());
    if let Err(error) = std::fs::write(&path, format!("{TOKEN}\n")) {
        panic!("write: {error}");
    }
    match client_service_token(Some(&path)) {
        Ok(token) => assert_eq!(token, TOKEN),
        Err(error) => panic!("token: {error}"),
    }
    let _ = std::fs::remove_file(path);
}
