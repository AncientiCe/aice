//! Room bridge: a pod talks to the gateway, the gateway runs turns on the
//! real backend with the pod's device token and speaks the answer back.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aice_backend::{
    spawn_server_with_options, AudioIngressConfig, AudioTranscriber, BackendEngine,
    BackendEngineDecision, DeviceAuth, ServerHandle, ServerOptions,
};
use async_trait::async_trait;
use core_runtime_protocol::{FrontendSkillResultRequest, TurnRequest};
use futures_util::{SinkExt, StreamExt};
use pod_gateway::{spawn_bridge, BridgeSettings, Speech, VadSettings};
use pod_protocol::{AudioPayload, GatewayToPod, LedState, PodToGateway};
use property_facilitator::{assign_device, serve, Pack, PropertyClient, Running, Settings};
use serde_json::json;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const SERVICE_TOKEN: &str = "bridge-test-service-token-0123456789";
const ANSWER: &str = "Towels are on the way.";

struct Answering {
    rooms: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl BackendEngine for Answering {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        let room = request
            .context
            .as_ref()
            .and_then(|context| context.get("room"))
            .and_then(|room| room.as_str())
            .map(str::to_string);
        if let Ok(mut rooms) = self.rooms.lock() {
            rooms.push(room);
        }
        Ok(BackendEngineDecision::Chat(ANSWER.to_string()))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(String::new())
    }
}

struct HeardTowels;

#[async_trait]
impl AudioTranscriber for HeardTowels {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("towels please".to_string())
    }
}

/// Explicit test double: 64 bytes of PCM per character, so the pod can check
/// it received exactly the synthesized answer.
struct CountingSpeech;

impl Speech for CountingSpeech {
    fn synthesize(&self, text: &str) -> Result<Vec<u8>, String> {
        Ok(vec![7u8; text.chars().count() * 64])
    }
}

fn temp_db(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!(
        "aice-bridge-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

struct Stack {
    _facilitator: Running,
    backend: ServerHandle,
    engine: Arc<Answering>,
    db: PathBuf,
    token: String,
}

async fn stack(label: &str) -> Stack {
    let db = temp_db(label);
    let mut settings = Settings::default_for(Pack::Hotels);
    settings.bind = "127.0.0.1:0".to_string();
    settings.database_path = db.clone();
    settings.service_token = SERVICE_TOKEN.to_string();
    let facilitator = serve(settings)
        .await
        .unwrap_or_else(|error| panic!("facilitator: {error}"));
    let enroll = || async {
        reqwest::Client::new()
            .post(format!("{}/api/devices/enroll", facilitator.url))
            .json(&json!({"device_id": "pod-204", "nonce": "pod-204-nonce-0123456789", "firmware": "t"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("enroll: {error}"))
            .json::<serde_json::Value>()
            .await
            .unwrap_or_default()
    };
    let _ = enroll().await;
    assign_device(&db, "pod-204", "204").unwrap_or_else(|error| panic!("assign: {error}"));
    let token = enroll().await["token"].as_str().unwrap_or("").to_string();
    let client = PropertyClient::new(format!("{}/mcp", facilitator.url), SERVICE_TOKEN)
        .unwrap_or_else(|error| panic!("client: {error}"));
    let engine = Arc::new(Answering {
        rooms: Mutex::new(Vec::new()),
    });
    let engine_dyn: Arc<dyn BackendEngine> = engine.clone();
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(HeardTowels);
    let backend = spawn_server_with_options(
        "127.0.0.1:0",
        engine_dyn,
        transcriber,
        AudioIngressConfig::default(),
        ServerOptions {
            device_auth: Some(Arc::new(DeviceAuth::new(client))),
            tls: None,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("backend: {error}"));
    Stack {
        _facilitator: facilitator,
        backend,
        engine,
        db,
        token,
    }
}

fn fast_vad() -> VadSettings {
    VadSettings {
        start_level: 1_000,
        start_frames: 2,
        end_silence: Duration::from_millis(300),
        max_turn: Duration::from_secs(5),
        preroll_frames: 3,
    }
}

async fn bridge(backend: &ServerHandle) -> pod_gateway::BridgeHandle {
    spawn_bridge(
        "127.0.0.1:0",
        BridgeSettings {
            backend_url: format!("ws://{}/turns/stream", backend.bind),
            backend_tls: None,
            pod_tls: None,
            vad: fast_vad(),
        },
        Arc::new(CountingSpeech),
    )
    .await
    .unwrap_or_else(|error| panic!("bridge: {error}"))
}

type PodSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn pod(bind: &str, token: &str) -> PodSocket {
    let mut request = format!("ws://{bind}/")
        .into_client_request()
        .unwrap_or_else(|error| panic!("request: {error}"));
    let header = format!("Bearer {token}")
        .parse()
        .unwrap_or_else(|error| panic!("header: {error}"));
    request.headers_mut().insert("authorization", header);
    let (socket, _) = connect_async(request)
        .await
        .unwrap_or_else(|error| panic!("pod connect: {error}"));
    socket
}

async fn send(socket: &mut PodSocket, message: &PodToGateway) {
    let text = serde_json::to_string(message).unwrap_or_default();
    socket
        .send(Message::Text(text))
        .await
        .unwrap_or_else(|error| panic!("pod send: {error}"));
}

async fn next(socket: &mut PodSocket) -> Option<GatewayToPod> {
    loop {
        match timeout(Duration::from_secs(5), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).ok(),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => return None,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) => return None,
            Err(_) => panic!("timed out waiting for the gateway"),
        }
    }
}

fn frame(level: i16) -> PodToGateway {
    let samples: Vec<u8> = (0..640)
        .flat_map(|i| {
            let value = if i % 2 == 0 { level } else { -level };
            value.to_le_bytes()
        })
        .collect();
    PodToGateway::Audio {
        payload: AudioPayload(samples),
    }
}

async fn hello(socket: &mut PodSocket) {
    send(
        socket,
        &PodToGateway::Hello {
            protocol_version: 1,
            device_id: "pod-204".to_string(),
            room: Some("ignored-by-server".to_string()),
        },
    )
    .await;
}

#[tokio::test]
async fn a_spoken_request_is_answered_through_the_pod_speaker() {
    let stack = stack("answer").await;
    let bridge = bridge(&stack.backend).await;
    let mut socket = pod(&bridge.bind, &stack.token).await;
    hello(&mut socket).await;
    assert!(matches!(
        next(&mut socket).await,
        Some(GatewayToPod::HelloAck { .. })
    ));
    assert_eq!(
        next(&mut socket).await,
        Some(GatewayToPod::Led {
            state: LedState::Listening
        })
    );

    for _ in 0..8 {
        send(&mut socket, &frame(8_000)).await;
    }
    for _ in 0..12 {
        send(&mut socket, &frame(0)).await;
    }

    let mut audio_bytes = 0usize;
    let mut saw_thinking = false;
    let mut saw_speaking = false;
    loop {
        match next(&mut socket).await {
            Some(GatewayToPod::Led {
                state: LedState::Thinking,
            }) => saw_thinking = true,
            Some(GatewayToPod::Led {
                state: LedState::Speaking,
            }) => saw_speaking = true,
            Some(GatewayToPod::Audio { payload }) => {
                assert!(
                    payload.0.len() <= 2048,
                    "chunks fit the pod's playback slots"
                );
                audio_bytes += payload.0.len();
            }
            Some(GatewayToPod::Led {
                state: LedState::Listening,
            }) if saw_speaking => break,
            Some(other) => panic!("unexpected message: {other:?}"),
            None => panic!("gateway closed the pod connection"),
        }
    }
    assert!(saw_thinking && saw_speaking);
    assert_eq!(audio_bytes, ANSWER.chars().count() * 64);
    let rooms = stack
        .engine
        .rooms
        .lock()
        .map(|rooms| rooms.clone())
        .unwrap_or_default();
    assert_eq!(
        rooms,
        vec![Some("204".to_string())],
        "room came from the token"
    );

    stack.backend.shutdown().await;
    let _ = std::fs::remove_file(stack.db);
}

#[tokio::test]
async fn a_pod_with_a_bad_token_is_turned_away() {
    let stack = stack("reject").await;
    let bridge = bridge(&stack.backend).await;
    let mut socket = pod(&bridge.bind, "not-a-device-token").await;
    hello(&mut socket).await;
    match next(&mut socket).await {
        Some(GatewayToPod::Error { code, .. }) => assert_eq!(code, "unauthorized"),
        other => panic!("expected an unauthorized error, got {other:?}"),
    }
    assert_eq!(next(&mut socket).await, None, "connection closed");
    stack.backend.shutdown().await;
    let _ = std::fs::remove_file(stack.db);
}

#[tokio::test]
async fn bad_frames_are_reported_without_dropping_the_pod() {
    let stack = stack("frames").await;
    let bridge = bridge(&stack.backend).await;
    let mut socket = pod(&bridge.bind, &stack.token).await;
    hello(&mut socket).await;
    assert!(matches!(
        next(&mut socket).await,
        Some(GatewayToPod::HelloAck { .. })
    ));
    let _listening = next(&mut socket).await;

    socket
        .send(Message::Text("{not json".to_string()))
        .await
        .unwrap_or_else(|error| panic!("send: {error}"));
    match next(&mut socket).await {
        Some(GatewayToPod::Error { code, .. }) => assert_eq!(code, "invalid_message"),
        other => panic!("expected invalid_message, got {other:?}"),
    }
    send(
        &mut socket,
        &PodToGateway::Audio {
            payload: AudioPayload(vec![0; 70 * 1024]),
        },
    )
    .await;
    match next(&mut socket).await {
        Some(GatewayToPod::Error { code, .. }) => assert_eq!(code, "payload_too_large"),
        other => panic!("expected payload_too_large, got {other:?}"),
    }
    send(&mut socket, &PodToGateway::Ping { seq: 9 }).await;
    assert_eq!(next(&mut socket).await, Some(GatewayToPod::Pong { seq: 9 }));
    stack.backend.shutdown().await;
    let _ = std::fs::remove_file(stack.db);
}
