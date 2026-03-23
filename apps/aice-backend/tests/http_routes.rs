use aice_backend::{spawn_server, BackendEngine, BackendEngineDecision};
use async_trait::async_trait;
use core_runtime_protocol::{
    FrontendActivateRequest, FrontendSkillIntent, FrontendSkillResultRequest, TurnChunkRequest,
    TurnRequest,
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
async fn turns_endpoint_emits_frontend_intent_event() {
    let engine_impl = Arc::new(DeterministicEngine::default());
    let engine: Arc<dyn BackendEngine> = engine_impl.clone();
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let client = reqwest::Client::new();
    let url = format!("http://{}/v1/turns", handle.bind);
    let response = client
        .post(url)
        .json(&TurnRequest {
            session_id: "s1".to_string(),
            device_id: None,
            turn_id: Some("turn-client-1".to_string()),
            transcript: "send alex hi".to_string(),
            finalize: true,
            context: None,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(body.contains("frontend_skill_intent"));
    assert!(body.contains("skill_message"));
    let seen_turn_ids = engine_impl
        .seen_turn_ids
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    assert_eq!(seen_turn_ids, vec![Some("turn-client-1".to_string())]);

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
async fn chunked_turn_executes_only_after_finalize() {
    let engine_impl = Arc::new(DeterministicEngine::default());
    let engine: Arc<dyn BackendEngine> = engine_impl.clone();
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));

    let client = reqwest::Client::new();
    let chunk_url = format!("http://{}/v1/turns/chunks", handle.bind);
    let chunk_response = client
        .post(chunk_url)
        .json(&TurnChunkRequest {
            session_id: "s1".to_string(),
            chunk: "i want to buy strawberries".to_string(),
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
    assert!(
        seen_after_chunk.is_empty(),
        "backend engine should not execute before finalize"
    );

    let turns_url = format!("http://{}/v1/turns", handle.bind);
    let response = client
        .post(turns_url)
        .json(&TurnRequest {
            session_id: "s1".to_string(),
            device_id: None,
            turn_id: Some("turn-client-2".to_string()),
            transcript: String::new(),
            finalize: true,
            context: None,
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
async fn turns_endpoint_blocks_frontend_intent_when_not_in_registered_capabilities() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let handle = spawn_server("127.0.0.1:0", engine)
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
        protocol_version: Some(1),
    };
    let activate_response = client
        .post(activate_url)
        .json(&activate)
        .send()
        .await
        .unwrap_or_else(|error| panic!("activation failed: {error}"));
    assert_eq!(activate_response.status(), reqwest::StatusCode::ACCEPTED);

    let turns_url = format!("http://{}/v1/turns", handle.bind);
    let response = client
        .post(turns_url)
        .json(&TurnRequest {
            session_id: "session-a".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: Some("turn-client-3".to_string()),
            transcript: "send alex hi".to_string(),
            finalize: true,
            context: None,
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
async fn turns_endpoint_allows_frontend_intent_when_registered_capability_matches() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let handle = spawn_server("127.0.0.1:0", engine)
        .await
        .unwrap_or_else(|error| panic!("spawn failed: {error}"));
    let client = reqwest::Client::new();
    let activate_url = format!("http://{}/v1/frontends/activate", handle.bind);
    let activate = FrontendActivateRequest {
        device_id: "device-a".to_string(),
        session_id: "session-a".to_string(),
        platform: "macos".to_string(),
        frontend_version: "0.1.0".to_string(),
        supported_frontend_intents: vec!["skill_message".to_string()],
        expires_in_seconds: Some(60),
        protocol_version: Some(1),
    };
    let activate_response = client
        .post(activate_url)
        .json(&activate)
        .send()
        .await
        .unwrap_or_else(|error| panic!("activation failed: {error}"));
    assert_eq!(activate_response.status(), reqwest::StatusCode::ACCEPTED);

    let turns_url = format!("http://{}/v1/turns", handle.bind);
    let response = client
        .post(turns_url)
        .json(&TurnRequest {
            session_id: "session-a".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: Some("turn-client-4".to_string()),
            transcript: "send alex hi".to_string(),
            finalize: true,
            context: None,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(body.contains("frontend_skill_intent"));
    assert!(body.contains("skill_message"));

    handle.shutdown().await;
}

#[tokio::test]
async fn turns_endpoint_ignores_expired_frontend_activation() {
    let engine: Arc<dyn BackendEngine> = Arc::new(DeterministicEngine::default());
    let handle = spawn_server("127.0.0.1:0", engine)
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
        expires_in_seconds: Some(0),
        protocol_version: Some(1),
    };
    let activate_response = client
        .post(activate_url)
        .json(&activate)
        .send()
        .await
        .unwrap_or_else(|error| panic!("activation failed: {error}"));
    assert_eq!(activate_response.status(), reqwest::StatusCode::ACCEPTED);

    let turns_url = format!("http://{}/v1/turns", handle.bind);
    let response = client
        .post(turns_url)
        .json(&TurnRequest {
            session_id: "session-a".to_string(),
            device_id: Some("device-a".to_string()),
            turn_id: Some("turn-client-5".to_string()),
            transcript: "send alex hi".to_string(),
            finalize: true,
            context: None,
        })
        .send()
        .await
        .unwrap_or_else(|error| panic!("request failed: {error}"));
    assert!(response.status().is_success());
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("text failed: {error}"));
    assert!(body.contains("frontend_skill_intent"));
    assert!(body.contains("skill_message"));

    handle.shutdown().await;
}
