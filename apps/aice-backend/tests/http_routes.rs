use aice_backend::{
    spawn_server, spawn_server_with_audio, AudioIngressConfig, AudioTranscriber, BackendEngine,
    BackendEngineDecision,
};
use async_trait::async_trait;
use core_config::WakeWordConfig;
use core_runtime_protocol::{
    FrontendSkillIntent, FrontendSkillResultRequest, TurnRequest, TurnStreamClientMessage,
    TurnStreamServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

async fn recv_ws<S>(read: &mut S) -> Message
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    match timeout(Duration::from_secs(2), read.next()).await {
        Ok(Some(Ok(msg))) => msg,
        Ok(Some(Err(e))) => panic!("ws read failed: {e}"),
        Ok(None) => panic!("ws stream ended unexpectedly"),
        Err(_) => panic!("timed out waiting for ws event"),
    }
}

#[derive(Default)]
struct DeterministicEngine {
    seen_transcripts: Mutex<Vec<String>>,
    seen_turn_ids: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl BackendEngine for DeterministicEngine {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut seen) = self.seen_transcripts.lock() {
            seen.push(request.transcript.clone());
        }
        if let Ok(mut seen) = self.seen_turn_ids.lock() {
            seen.push(request.turn_id.clone());
        }
        Ok(BackendEngineDecision::FrontendSkillIntent(
            FrontendSkillIntent {
                turn_id: "turn-123".to_string(),
                intent: "skill_message".to_string(),
                slots: serde_json::json!({"message_contact":"alex","message_text":"hi"}),
                user_text: "send alex hi".to_string(),
            },
        ))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("done from finalize".to_string())
    }
}

struct StaticTranscriber {
    transcript: String,
}

#[async_trait]
impl AudioTranscriber for StaticTranscriber {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.transcript.clone())
    }
}

struct SequencedTranscriber {
    transcripts: Mutex<Vec<String>>,
}

#[async_trait]
impl AudioTranscriber for SequencedTranscriber {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut guard) = self.transcripts.lock() {
            if guard.is_empty() {
                return Ok(String::new());
            }
            return Ok(guard.remove(0));
        }
        Ok(String::new())
    }
}

struct EchoEngine;

#[async_trait]
impl BackendEngine for EchoEngine {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(BackendEngineDecision::Chat(format!(
            "echo:{}",
            request.transcript.trim()
        )))
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

struct CountingEchoEngine {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl BackendEngine for CountingEchoEngine {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(BackendEngineDecision::Chat(format!(
            "echo:{}",
            request.transcript.trim()
        )))
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

fn pcm_binary_frame(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[tokio::test]
async fn healthz_returns_ok() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let client = reqwest::Client::new();
    let url = format!("http://{}/healthz", handle.bind);
    let response = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert_eq!(body, "ok");

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_binary_audio_does_not_emit_token_before_done() {
    let engine: Arc<dyn BackendEngine> = Arc::new(EchoEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "hello backend".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    let start = TurnStreamClientMessage::TurnStart {
        session_id: "s1".to_string(),
        device_id: Some("device-a".to_string()),
        turn_id: "turn-ws-1".to_string(),
        supported_frontend_intents: vec![],
        schema_version: None,
    };
    write
        .send(Message::Text(
            serde_json::to_string(&start).unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let audio = pcm_binary_frame(&[1, 2, 3, 4, 5, 6]);
    write
        .send(Message::Binary(audio))
        .await
        .unwrap_or_else(|error| panic!("send binary failed: {error}"));

    let first_message = recv_ws(&mut read).await;
    let Message::Text(text) = first_message else {
        panic!("expected text websocket message");
    };
    let event: TurnStreamServerEvent =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("decode event failed: {error}"));
    assert!(
        matches!(event, TurnStreamServerEvent::PartialTranscript { .. }),
        "expected first event to be partial transcript, got: {event:?}"
    );

    let maybe_next = timeout(Duration::from_millis(250), read.next()).await;
    if let Ok(Some(Ok(Message::Text(text)))) = maybe_next {
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        assert!(
            !matches!(ev, TurnStreamServerEvent::Token { .. }),
            "unexpected token before turn_done: {ev:?}"
        );
    }

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_transcript_divergence_only_emits_final_token_after_done() {
    let engine: Arc<dyn BackendEngine> = Arc::new(EchoEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(SequencedTranscriber {
        transcripts: Mutex::new(vec![
            "old hypothesis".to_string(),
            "final coherent".to_string(),
        ]),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "s1".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-ws-2".to_string(),
                supported_frontend_intents: vec![],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    for _ in 0..3_u64 {
        write
            .send(Message::Binary(pcm_binary_frame(&[
                100, 101, 102, 103, 104, 105,
            ])))
            .await
            .unwrap_or_else(|error| panic!("send failed: {error}"));
    }

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut tokens = Vec::new();
    for _ in 0..12 {
        let msg = recv_ws(&mut read).await;
        let Message::Text(text) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        match ev {
            TurnStreamServerEvent::Token { text, .. } => tokens.push(text),
            TurnStreamServerEvent::Done { .. } => break,
            _ => {}
        }
    }
    assert!(
        tokens.iter().any(|token| token.contains("final coherent")),
        "expected final transcript token in {:?}",
        tokens
    );
    assert_eq!(
        tokens.len(),
        1,
        "expected exactly one token response after turn_done, got {:?}",
        tokens
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_only_classifies_once_after_turn_done() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine: Arc<dyn BackendEngine> = Arc::new(CountingEchoEngine {
        calls: calls.clone(),
    });
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(SequencedTranscriber {
        transcripts: Mutex::new(vec![
            "old hypothesis".to_string(),
            "final coherent".to_string(),
        ]),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "s1".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-ws-count".to_string(),
                supported_frontend_intents: vec![],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    for _ in 0..3_u64 {
        write
            .send(Message::Binary(pcm_binary_frame(&[
                100, 101, 102, 103, 104, 105,
            ])))
            .await
            .unwrap_or_else(|error| panic!("send failed: {error}"));
    }

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    for _ in 0..12 {
        let msg = recv_ws(&mut read).await;
        let Message::Text(text) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        if matches!(ev, TurnStreamServerEvent::Done { .. }) {
            break;
        }
    }

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "expected one backend classification call after turn_done"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_cancel_aborts_active_turn() {
    let engine: Arc<dyn BackendEngine> = Arc::new(EchoEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "some transcript".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "s1".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-cancel".to_string(),
                supported_frontend_intents: vec![],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Binary(pcm_binary_frame(&[1, 2, 3, 4])))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnCancel)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut saw_done = false;
    for _ in 0..10 {
        let msg = timeout(Duration::from_secs(2), read.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        if matches!(ev, TurnStreamServerEvent::Done { .. }) {
            saw_done = true;
            break;
        }
    }
    assert!(saw_done, "expected Done event after cancel");

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_malformed_text_returns_error() {
    let engine: Arc<dyn BackendEngine> = Arc::new(EchoEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "ignored".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text("not-json".to_string()))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let msg = recv_ws(&mut read).await;
    let Message::Text(text) = msg else {
        panic!("expected text");
    };
    let ev: TurnStreamServerEvent =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert!(
        matches!(ev, TurnStreamServerEvent::Error { .. }),
        "expected error event for malformed message"
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_odd_binary_frame_returns_error() {
    let engine: Arc<dyn BackendEngine> = Arc::new(EchoEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "ignored".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "s1".to_string(),
                device_id: Some("d1".to_string()),
                turn_id: "turn-odd".to_string(),
                supported_frontend_intents: vec![],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Binary(vec![0x01, 0x02, 0x03]))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let msg = recv_ws(&mut read).await;
    let Message::Text(text) = msg else {
        panic!("expected text");
    };
    let ev: TurnStreamServerEvent =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("decode failed: {error}"));
    match ev {
        TurnStreamServerEvent::Error { message, .. } => {
            assert!(
                message.contains("even"),
                "expected even-byte error: {message}"
            );
        }
        other => panic!("expected error event, got {other:?}"),
    }

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_wake_word_drops_non_matching_transcript() {
    let engine_impl = Arc::new(DeterministicEngine::default());
    let engine: Arc<dyn BackendEngine> = engine_impl.clone();
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "turn on the lights".to_string(),
    });
    let audio_config = AudioIngressConfig {
        wake_word: WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 2,
        },
    };
    let handle = spawn_server_with_audio("127.0.0.1:0", engine, transcriber, audio_config)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "wake-session".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-wake".to_string(),
                supported_frontend_intents: vec![],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Binary(pcm_binary_frame(&[100, 100])))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut saw_done = false;
    for _ in 0..6 {
        let msg = timeout(Duration::from_secs(2), read.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = msg else {
            break;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        if matches!(ev, TurnStreamServerEvent::Done { .. }) {
            saw_done = true;
            break;
        }
    }
    assert!(saw_done, "expected Done without engine call");
    let seen = engine_impl
        .seen_transcripts
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    assert!(seen.is_empty(), "turn should be dropped by wake-word gate");

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_capability_gate_blocks_unregistered_skill() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "send alex hi".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "session-cap".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-cap".to_string(),
                supported_frontend_intents: vec!["skill_timer".to_string()],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Binary(pcm_binary_frame(&[100, 200])))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut saw_fallback = false;
    for _ in 0..10 {
        let msg = timeout(Duration::from_secs(2), read.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        if let TurnStreamServerEvent::Token { text, .. } = ev {
            if text.contains("not available on this active frontend") {
                saw_fallback = true;
                break;
            }
        }
    }
    assert!(
        saw_fallback,
        "expected fallback token when frontend lacks skill_message capability"
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn turn_stream_frontend_skill_round_trip() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "send alex hi".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let ws_url = format!("ws://{}/turns/stream", handle.bind);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws_stream.split();

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnStart {
                session_id: "session-skill".to_string(),
                device_id: Some("device-a".to_string()),
                turn_id: "turn-skill".to_string(),
                supported_frontend_intents: vec!["skill_message".to_string()],
                schema_version: None,
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Binary(pcm_binary_frame(&[10, 20, 30])))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::TurnDone)
                .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut saw_skill_intent = false;
    for _ in 0..10 {
        let msg = timeout(Duration::from_secs(2), read.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        if matches!(ev, TurnStreamServerEvent::FrontendSkillIntent(_)) {
            saw_skill_intent = true;
            break;
        }
    }
    assert!(
        saw_skill_intent,
        "expected FrontendSkillIntent when capability is registered"
    );

    write
        .send(Message::Text(
            serde_json::to_string(&TurnStreamClientMessage::FrontendSkillResult {
                turn_id: "turn-123".to_string(),
                intent_id: "skill_message".to_string(),
                result: FrontendSkillResultRequest {
                    status: "success".to_string(),
                    user_text: "send alex hi".to_string(),
                    structured_result_context: Some("Message sent to Alex".to_string()),
                    error: None,
                },
            })
            .unwrap_or_else(|error| panic!("encode failed: {error}")),
        ))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    let mut saw_token = false;
    let mut saw_done = false;
    for _ in 0..10 {
        let msg = timeout(Duration::from_secs(2), read.next()).await;
        let Ok(Some(Ok(Message::Text(text)))) = msg else {
            continue;
        };
        let ev: TurnStreamServerEvent = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("decode event failed: {error}"));
        match ev {
            TurnStreamServerEvent::Token { text, .. } if text.contains("done from finalize") => {
                saw_token = true;
            }
            TurnStreamServerEvent::Done { .. } => {
                saw_done = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_token, "expected finalize token");
    assert!(saw_done, "expected Done after skill round-trip");

    handle.shutdown().await;
}
