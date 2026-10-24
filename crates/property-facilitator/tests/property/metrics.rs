use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use property_facilitator::{add_user, serve, Role};
use reqwest::StatusCode;
use serde_json::json;

use crate::common::{call_tool, hotels, login, temp_db, ticket_id, PASSWORD};

fn free_bind() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| panic!("reserve a local port: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("local addr: {error}"));
    drop(listener);
    addr.to_string()
}

fn scrape(bind: &str) -> String {
    let Ok(mut stream) = TcpStream::connect(bind) else {
        return String::new();
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ =
        stream.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let mut body = Vec::new();
    let mut chunk = [0_u8; 4096];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8_lossy(&body).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ticket_that_is_done_records_how_long_it_took_to_resolve() {
    let bind = free_bind();
    core_observability::init_prometheus_exporter(&bind)
        .unwrap_or_else(|error| panic!("exporter: {error}"));
    core_observability::register_metrics();

    let db = temp_db("resolve-metric");
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
    let (status, _) = desk
        .api_post(&format!("/api/tickets/{id}/acknowledge"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = desk.api_post(&format!("/api/tickets/{id}/done")).await;
    assert_eq!(status, StatusCode::OK);

    let deadline = Instant::now() + Duration::from_secs(5);
    let payload = loop {
        let payload = scrape(&bind);
        if payload.contains("property_ticket_resolve_duration_seconds_count")
            || Instant::now() >= deadline
        {
            break payload;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        payload.contains("property_ticket_resolve_duration_seconds_count{pack=\"hotels\",tool=\"request_extra_towels\"}"),
        "{payload}"
    );
    assert!(
        payload.contains("property_ticket_ack_duration_seconds_count{pack=\"hotels\",tool=\"request_extra_towels\"}"),
        "{payload}"
    );
    let _ = std::fs::remove_file(db);
}
