use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use property_facilitator::{serve, serve_fixed_mcp, Pack, Settings};
use serde_json::json;

fn temp_db(label: &str) -> PathBuf {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let mut path = std::env::temp_dir();
    path.push(format!(
        "aice-property-{label}-{}-{millis}.sqlite",
        std::process::id()
    ));
    path
}

fn hotels(db: PathBuf, upstream: Option<String>, delegated: Vec<String>) -> Settings {
    Settings {
        pack: Pack::Hotels,
        bind: "127.0.0.1:0".to_string(),
        database_path: db,
        property_mcp_url: upstream,
        delegated_tools: delegated,
        extensions: [("101".to_string(), "204".to_string())]
            .into_iter()
            .collect(),
    }
}

async fn post_mcp(base: &str, body: serde_json::Value) -> serde_json::Value {
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(error) => panic!("http client: {error}"),
    };
    let response = match client.post(format!("{base}/mcp")).json(&body).send().await {
        Ok(response) => response,
        Err(error) => panic!("mcp post failed: {error}"),
    };
    match response.json().await {
        Ok(value) => value,
        Err(error) => panic!("mcp json failed: {error}"),
    }
}

#[tokio::test]
async fn hotel_tool_creates_an_open_ticket() {
    let db = temp_db("open");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "request_extra_towels",
                "arguments": { "room": "204", "count": 2 }
            }
        }),
    )
    .await;
    let spoken = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(spoken.contains("request_extra_towels"));
    assert!(spoken.contains("204"));
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["structuredContent"]["status"], "open");

    let client = reqwest::Client::new();
    let tickets: Vec<serde_json::Value> = match client
        .get(format!("{}/api/tickets", running.url))
        .send()
        .await
    {
        Ok(response) => match response.json().await {
            Ok(value) => value,
            Err(error) => panic!("tickets json: {error}"),
        },
        Err(error) => panic!("tickets get: {error}"),
    };
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

    let response = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "set_room_temperature",
                "arguments": { "room": "204", "celsius": 22 }
            }
        }),
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
    let response = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "request_extra_towels",
                "arguments": { "room": "18" }
            }
        }),
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
    let created = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "request_extra_towels",
                "arguments": { "room": "204" }
            }
        }),
    )
    .await;
    let ticket_id = created["result"]["structuredContent"]["ticket_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let client = reqwest::Client::new();
    let desk = match client.get(format!("{}/desk", running.url)).send().await {
        Ok(response) => match response.text().await {
            Ok(text) => text,
            Err(error) => panic!("desk text: {error}"),
        },
        Err(error) => panic!("desk get: {error}"),
    };
    assert!(desk.contains("204"));
    assert!(desk.contains("request_extra_towels"));
    assert!(desk.contains("aice-hotels desk"));
    let updated: serde_json::Value = match client
        .post(format!(
            "{}/api/tickets/{ticket_id}/acknowledge",
            running.url
        ))
        .send()
        .await
    {
        Ok(response) => match response.json().await {
            Ok(value) => value,
            Err(error) => panic!("ack json: {error}"),
        },
        Err(error) => panic!("ack post: {error}"),
    };
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
    let running = match serve(Settings {
        pack: Pack::Care,
        bind: "127.0.0.1:0".to_string(),
        database_path: db.clone(),
        property_mcp_url: Some(upstream.url.clone()),
        delegated_tools: vec!["report_fall".to_string()],
        extensions: std::collections::HashMap::new(),
    })
    .await
    {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let response = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "report_fall",
                "arguments": { "room": "12" }
            }
        }),
    )
    .await;
    assert_eq!(upstream.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        response["result"]["structuredContent"]["status"],
        "escalated"
    );
    let desk = match reqwest::Client::new()
        .get(format!("{}/desk", running.url))
        .send()
        .await
    {
        Ok(response) => match response.text().await {
            Ok(text) => text,
            Err(error) => panic!("desk text: {error}"),
        },
        Err(error) => panic!("desk get: {error}"),
    };
    assert!(desk.contains("aice-care desk"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn ward_denies_medication_and_allows_wayfinding() {
    let db = temp_db("ward");
    let running = match serve(Settings {
        pack: Pack::Ward,
        bind: "127.0.0.1:0".to_string(),
        database_path: db.clone(),
        property_mcp_url: None,
        delegated_tools: Vec::new(),
        extensions: std::collections::HashMap::new(),
    })
    .await
    {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let denied = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "prescribe_medication",
                "arguments": { "room": "4B" }
            }
        }),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(denied["result"]["structuredContent"]["status"], "escalated");

    let allowed = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "wayfinding",
                "arguments": { "room": "4B", "destination": "radiology" }
            }
        }),
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
    let response = post_mcp(
        &running.url,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "request_housekeeping",
                "arguments": { "extension": "101" }
            }
        }),
    )
    .await;
    assert!(response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("204"));
    assert_eq!(response["result"]["structuredContent"]["status"], "open");
    let _ = std::fs::remove_file(db);
}
