use aice_backend::{spawn_server, BackendEngine, BackendEngineDecision};
#[allow(deprecated)]
use aice_macos::{FrontendClient, FrontendSkillExecutor};
use async_trait::async_trait;
use core_runtime_protocol::{FrontendSkillIntent, FrontendSkillResultRequest, TurnRequest};
use std::sync::{Arc, Mutex};

struct BackendScenario;

#[async_trait]
impl BackendEngine for BackendScenario {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        if request.transcript.contains("weather") {
            return Ok(BackendEngineDecision::Chat(
                "The weather is sunny".to_string(),
            ));
        }
        Ok(BackendEngineDecision::FrontendSkillIntent(
            FrontendSkillIntent {
                turn_id: "turn-42".to_string(),
                intent: "skill_message".to_string(),
                slots: serde_json::json!({"message_contact":"alex","message_text":"hi"}),
                user_text: request.transcript,
            },
        ))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(format!(
            "finalized: {}",
            request
                .structured_result_context
                .unwrap_or_else(|| "missing".to_string())
        ))
    }
}

#[derive(Default)]
struct TurnIdCaptureScenario {
    seen_turn_ids: Mutex<Vec<Option<String>>>,
    seen_device_ids: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl BackendEngine for TurnIdCaptureScenario {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut seen) = self.seen_turn_ids.lock() {
            seen.push(request.turn_id.clone());
        }
        if let Ok(mut seen) = self.seen_device_ids.lock() {
            seen.push(request.device_id.clone());
        }
        Ok(BackendEngineDecision::Chat("ok".to_string()))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("unused".to_string())
    }
}

struct FailingBackendScenario;

#[async_trait]
impl BackendEngine for FailingBackendScenario {
    async fn process_turn(
        &self,
        _request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        Err("time skill failed: no default location configured".into())
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("unused".to_string())
    }
}

struct RecordingExecutor {
    called: Arc<Mutex<bool>>,
}

#[async_trait]
impl FrontendSkillExecutor for RecordingExecutor {
    async fn execute(
        &self,
        intent: &FrontendSkillIntent,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut guard) = self.called.lock() {
            *guard = true;
        }
        Ok(format!("executed {}", intent.intent))
    }
}

#[tokio::test]
async fn frontend_returns_chat_text_without_skill_callback() {
    let engine: Arc<dyn BackendEngine> = Arc::new(BackendScenario);
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let called = Arc::new(Mutex::new(false));
    let executor = RecordingExecutor {
        called: called.clone(),
    };
    let client = FrontendClient::new(format!("http://{}", handle.bind));

    let text = client
        .run_turn("session-1", "what's the weather", &executor)
        .await
        .unwrap_or_else(|error| panic!("run_turn failed: {error}"));
    assert_eq!(text, "The weather is sunny");
    let was_called = called.lock().map(|guard| *guard).unwrap_or(false);
    assert!(!was_called);

    handle.shutdown().await;
}

#[tokio::test]
async fn frontend_executes_skill_and_finalizes_reply() {
    let engine: Arc<dyn BackendEngine> = Arc::new(BackendScenario);
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let called = Arc::new(Mutex::new(false));
    let executor = RecordingExecutor {
        called: called.clone(),
    };
    let client = FrontendClient::new(format!("http://{}", handle.bind));

    let text = client
        .run_turn("session-1", "send alex hi", &executor)
        .await
        .unwrap_or_else(|error| panic!("run_turn failed: {error}"));
    assert_eq!(text, "finalized: executed skill_message");
    let was_called = called.lock().map(|guard| *guard).unwrap_or(false);
    assert!(was_called);

    handle.shutdown().await;
}

#[tokio::test]
async fn frontend_surfaces_backend_error_when_no_token_available() {
    let engine: Arc<dyn BackendEngine> = Arc::new(FailingBackendScenario);
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let called = Arc::new(Mutex::new(false));
    let executor = RecordingExecutor {
        called: called.clone(),
    };
    let client = FrontendClient::new(format!("http://{}", handle.bind));

    let text = client
        .run_turn("session-1", "what's the time?", &executor)
        .await
        .unwrap_or_else(|error| panic!("run_turn failed: {error}"));
    assert!(text.contains("backend error"));
    let was_called = called.lock().map(|guard| *guard).unwrap_or(false);
    assert!(!was_called);

    handle.shutdown().await;
}

#[tokio::test]
async fn frontend_includes_turn_id_in_turn_request() {
    let engine_impl = Arc::new(TurnIdCaptureScenario::default());
    let engine: Arc<dyn BackendEngine> = engine_impl.clone();
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));
    let called = Arc::new(Mutex::new(false));
    let executor = RecordingExecutor { called };
    let client = FrontendClient::new(format!("http://{}", handle.bind));

    let _ = client
        .run_turn("session-1", "hello", &executor)
        .await
        .unwrap_or_else(|error| panic!("run_turn failed: {error}"));
    let seen_turn_ids = engine_impl
        .seen_turn_ids
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    assert_eq!(seen_turn_ids.len(), 1);
    let first = seen_turn_ids[0]
        .as_ref()
        .unwrap_or_else(|| panic!("missing turn id"));
    assert!(first.starts_with("turn-session-1-"));
    let seen_device_ids = engine_impl
        .seen_device_ids
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    assert_eq!(seen_device_ids.len(), 1);
    assert!(seen_device_ids[0].as_deref().is_some());

    handle.shutdown().await;
}
