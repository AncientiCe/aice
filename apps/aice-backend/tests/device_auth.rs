//! Device tokens on `/turns/stream`, verified against a real facilitator.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aice_backend::{
    spawn_server_with_options, AudioIngressConfig, AudioTranscriber, BackendEngine,
    BackendEngineDecision, DeviceAuth, ServerOptions,
};
use async_trait::async_trait;
use core_runtime_protocol::{
    FrontendSkillResultRequest, TurnRequest, TurnStreamClientMessage, TurnStreamServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use property_facilitator::{assign_device, serve, Pack, PropertyClient, Running, Settings};
use serde_json::json;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const SERVICE_TOKEN: &str = "backend-test-service-token-0123456789";

#[derive(Default)]
struct RecordingEngine {
    requests: Mutex<Vec<TurnRequest>>,
}

#[async_trait]
impl BackendEngine for RecordingEngine {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut seen) = self.requests.lock() {
            seen.push(request);
        }
        Ok(BackendEngineDecision::Chat("ok".to_string()))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("done".to_string())
    }
}

struct StaticTranscriber;

#[async_trait]
impl AudioTranscriber for StaticTranscriber {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("extra towels please".to_string())
    }
}

fn temp_db(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!(
        "aice-backend-auth-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

/// Start a facilitator and provision one pod in `room`; returns its device token.
async fn facilitator_with_pod(label: &str, room: &str) -> (Running, PathBuf, String) {
    let db = temp_db(label);
    let mut settings = Settings::default_for(Pack::Hotels);
    settings.bind = "127.0.0.1:0".to_string();
    settings.database_path = db.clone();
    settings.service_token = SERVICE_TOKEN.to_string();
    let running = serve(settings)
        .await
        .unwrap_or_else(|error| panic!("facilitator failed: {error}"));
    let enroll = |base: String| async move {
        reqwest::Client::new()
            .post(format!("{base}/api/devices/enroll"))
            .json(&json!({"device_id": "pod-204", "nonce": "pod-204-nonce-0123456789", "firmware": "test"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("enroll failed: {error}"))
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|error| panic!("enroll json failed: {error}"))
    };
    let _ = enroll(running.url.clone()).await;
    assign_device(&db, "pod-204", room).unwrap_or_else(|error| panic!("assign failed: {error}"));
    let body = enroll(running.url.clone()).await;
    let token = body["token"].as_str().unwrap_or("").to_string();
    assert!(!token.is_empty(), "pod did not receive a token: {body}");
    (running, db, token)
}

async fn backend(running: &Running, engine: Arc<RecordingEngine>) -> aice_backend::ServerHandle {
    let client = PropertyClient::new(format!("{}/mcp", running.url), SERVICE_TOKEN)
        .unwrap_or_else(|error| panic!("client failed: {error}"));
    let engine: Arc<dyn BackendEngine> = engine;
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber);
    spawn_server_with_options(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
        ServerOptions {
            device_auth: Some(Arc::new(DeviceAuth::new(client))),
            tls: None,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("backend failed: {error}"))
}

fn ws_request(
    bind: &str,
    token: Option<&str>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("ws://{bind}/turns/stream")
        .into_client_request()
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    if let Some(token) = token {
        let value = format!("Bearer {token}")
            .parse()
            .unwrap_or_else(|error| panic!("header failed: {error}"));
        request.headers_mut().insert("authorization", value);
    }
    request
}

fn http_status(error: tokio_tungstenite::tungstenite::Error) -> u16 {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
        other => panic!("expected an HTTP rejection, got {other}"),
    }
}

#[tokio::test]
async fn turn_stream_rejects_missing_and_unknown_device_tokens() {
    let (running, db, _token) = facilitator_with_pod("reject", "204").await;
    let engine = Arc::new(RecordingEngine::default());
    let handle = backend(&running, Arc::clone(&engine)).await;

    let missing = connect_async(ws_request(&handle.bind, None)).await;
    assert_eq!(missing.map(|_| ()).map_err(http_status), Err(401));
    let unknown = connect_async(ws_request(&handle.bind, Some("forged-device-token"))).await;
    assert_eq!(unknown.map(|_| ()).map_err(http_status), Err(401));

    handle.shutdown().await;
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn turn_uses_the_device_and_room_from_the_token_not_the_client() {
    let (running, db, token) = facilitator_with_pod("room", "204").await;
    let engine = Arc::new(RecordingEngine::default());
    let handle = backend(&running, Arc::clone(&engine)).await;

    let (ws, _) = connect_async(ws_request(&handle.bind, Some(&token)))
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws.split();
    let start = TurnStreamClientMessage::TurnStart {
        session_id: "s1".to_string(),
        device_id: Some("spoofed-device".to_string()),
        turn_id: "turn-1".to_string(),
        supported_frontend_intents: vec![],
        schema_version: None,
    };
    for message in [
        Message::Text(serde_json::to_string(&start).unwrap_or_default()),
        Message::Binary(vec![1, 0, 2, 0, 3, 0, 4, 0]),
        Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone).unwrap_or_default(),
        ),
    ] {
        write
            .send(message)
            .await
            .unwrap_or_else(|error| panic!("send failed: {error}"));
    }
    loop {
        let next = timeout(Duration::from_secs(3), read.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for the turn to finish"));
        let Some(Ok(Message::Text(text))) = next else {
            panic!("websocket closed before the turn finished");
        };
        let event: TurnStreamServerEvent =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("decode failed: {error}"));
        if matches!(event, TurnStreamServerEvent::Done { .. }) {
            break;
        }
    }

    let requests = engine
        .requests
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let request = requests
        .last()
        .unwrap_or_else(|| panic!("engine was never called"));
    assert_eq!(request.device_id.as_deref(), Some("pod-204"));
    assert_eq!(
        request
            .context
            .as_ref()
            .and_then(|context| context.get("room"))
            .and_then(|room| room.as_str()),
        Some("204")
    );

    handle.shutdown().await;
    let _ = std::fs::remove_file(db);
}

#[test]
fn required_device_tokens_need_a_facilitator() {
    let mut config = core_config::Config::default();
    assert!(
        aice_backend::server_options_from_config(&config, "127.0.0.1:0")
            .map(|options| options.device_auth.is_none())
            .unwrap_or(false)
    );
    config.property.require_device_token = Some(true);
    assert!(aice_backend::server_options_from_config(&config, "127.0.0.1:0").is_err());
    config.property.facilitator_url = Some("http://127.0.0.1:8791/mcp".to_string());
    config.property.service_token_file = None;
    std::env::remove_var(property_facilitator::SERVICE_TOKEN_ENV);
    assert!(
        aice_backend::server_options_from_config(&config, "127.0.0.1:0").is_err(),
        "no service token means no verification"
    );
}
