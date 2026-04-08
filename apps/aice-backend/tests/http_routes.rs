use aice_backend::{
    spawn_server, spawn_server_with_audio, AudioIngressConfig, AudioTranscriber, BackendEngine,
    BackendEngineDecision,
};
use async_trait::async_trait;
use base64::Engine;
use core_config::WakeWordConfig;
use core_runtime_protocol::{
    AudioChunkRequest, AudioFinalizeRequest, DoneReason, FrontendActivateRequest,
    FrontendSkillIntent, FrontendSkillResultRequest, TurnRequest,
};
use std::sync::{Arc, Mutex};

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

fn encode_pcm(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
async fn finalize_endpoint_streams_token() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let client = reqwest::Client::new();
    let url = format!(
        "http://{}/v1/turns/{}/frontend-skill-result",
        handle.bind, "turn-123"
    );
    let response = client
        .post(url)
        .json(&FrontendSkillResultRequest {
            status: "success".to_string(),
            user_text: "send alex hi".to_string(),
            structured_result_context: Some("Message sent to Alex".to_string()),
            error: None,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(body.contains("done from finalize"));

    handle.shutdown().await;
}

#[tokio::test]
async fn audio_turn_executes_only_after_finalize() {
    let engine_impl = Arc::new(DeterministicEngine::default());
    let engine: Arc<dyn BackendEngine> = engine_impl.clone();
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(StaticTranscriber {
        transcript: "i want to buy strawberries".to_string(),
    });
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let client = reqwest::Client::new();
    let chunk_url = format!("http://{}/v1/turns/audio-chunks", handle.bind);
    let chunk_response = client
        .post(chunk_url)
        .json(&AudioChunkRequest {
            session_id: "s1".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-client-2".to_string(),
            seq: 0,
            pcm_s16le_base64: encode_pcm(&[1, 2, 3, 4]),
            sample_rate_hz: 16_000,
            channels: 1,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert_eq!(chunk_response.status(), reqwest::StatusCode::ACCEPTED);
    let seen_after_chunk = engine_impl
        .seen_transcripts
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    assert!(seen_after_chunk.is_empty());

    let finalize_url = format!("http://{}/v1/turns/audio-finalize", handle.bind);
    let response = client
        .post(finalize_url)
        .json(&AudioFinalizeRequest {
            session_id: "s1".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-client-2".to_string(),
            done_reason: DoneReason::VadEnd,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let seen_after_finalize = engine_impl
        .seen_transcripts
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    assert_eq!(seen_after_finalize, vec!["i want to buy strawberries"]);

    handle.shutdown().await;
}

#[tokio::test]
async fn audio_chunks_reject_out_of_order_sequence() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
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

    let client = reqwest::Client::new();
    let chunk_url = format!("http://{}/v1/turns/audio-chunks", handle.bind);
    let response = client
        .post(chunk_url)
        .json(&AudioChunkRequest {
            session_id: "s1".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-oos".to_string(),
            seq: 1,
            pcm_s16le_base64: encode_pcm(&[1, 2]),
            sample_rate_hz: 16_000,
            channels: 1,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.shutdown().await;
}

#[tokio::test]
async fn audio_finalize_honors_frontend_capability_gate() {
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
    let client = reqwest::Client::new();
    let activate_url = format!("http://{}/v1/frontends/activate", handle.bind);
    let activate = FrontendActivateRequest {
        device_id: "device-a".to_string(),
        session_id: "session-a".to_string(),
        platform: "macos".to_string(),
        frontend_version: "0.1.0".to_string(),
        supported_frontend_intents: vec!["skill_timer".to_string()],
        expires_in_seconds: Some(60),
        protocol_version: Some(2),
    };
    let activate_response = client
        .post(activate_url)
        .json(&activate)
        .send()
        .await
        .unwrap_or_else(|error| panic!("activation failed: {error}"));
    assert_eq!(activate_response.status(), reqwest::StatusCode::ACCEPTED);

    let chunk_url = format!("http://{}/v1/turns/audio-chunks", handle.bind);
    let _ = client
        .post(chunk_url)
        .json(&AudioChunkRequest {
            session_id: "session-a".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-capability".to_string(),
            seq: 0,
            pcm_s16le_base64: encode_pcm(&[100, 200]),
            sample_rate_hz: 16_000,
            channels: 1,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));

    let finalize_url = format!("http://{}/v1/turns/audio-finalize", handle.bind);
    let response = client
        .post(finalize_url)
        .json(&AudioFinalizeRequest {
            session_id: "session-a".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-capability".to_string(),
            done_reason: DoneReason::VadEnd,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(!body.contains("frontend_skill_intent"));
    assert!(body.contains("not available on this active frontend"));

    handle.shutdown().await;
}

#[tokio::test]
async fn wake_word_enabled_in_backend_drops_non_matching_transcript() {
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
        ..AudioIngressConfig::default()
    };
    let handle = spawn_server_with_audio("127.0.0.1:0", engine, transcriber, audio_config)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));
    let client = reqwest::Client::new();
    let chunk_url = format!("http://{}/v1/turns/audio-chunks", handle.bind);
    let _ = client
        .post(chunk_url)
        .json(&AudioChunkRequest {
            session_id: "wake-session".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-wake".to_string(),
            seq: 0,
            pcm_s16le_base64: encode_pcm(&[100, 100]),
            sample_rate_hz: 16_000,
            channels: 1,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));

    let finalize_url = format!("http://{}/v1/turns/audio-finalize", handle.bind);
    let response = client
        .post(finalize_url)
        .json(&AudioFinalizeRequest {
            session_id: "wake-session".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: "turn-wake".to_string(),
            done_reason: DoneReason::VadEnd,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let seen = engine_impl
        .seen_transcripts
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    assert!(seen.is_empty(), "turn should be dropped by wake-word gate");
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(body.contains("\"type\":\"done\""));

    handle.shutdown().await;
}
