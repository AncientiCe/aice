use core_runtime_protocol::{
    FrontendActivateRequest, FrontendDeactivateRequest, FrontendHeartbeatRequest,
    FrontendSkillResultRequest, RuntimeEvent, TurnChunkRequest, TurnRequest,
};

#[test]
fn turn_request_roundtrip_json() {
    let request = TurnRequest {
        session_id: "session-1".to_string(),
        device_id: Some("device-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        transcript: "what's the weather".to_string(),
        finalize: true,
        context: Some(serde_json::json!({"source":"desktop"})),
    };
    let encoded =
        serde_json::to_string(&request).unwrap_or_else(|error| panic!("encode failed: {error}"));
    let decoded: TurnRequest =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.device_id.as_deref(), Some("device-1"));
    assert_eq!(decoded.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(decoded.transcript, "what's the weather");
    assert!(decoded.finalize);
}

#[test]
fn turn_request_defaults_finalize_to_true_for_compatibility() {
    let payload = r#"{"session_id":"session-1","transcript":"hello"}"#;
    let decoded: TurnRequest =
        serde_json::from_str(payload).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert!(decoded.finalize);
    assert_eq!(decoded.device_id, None);
    assert_eq!(decoded.turn_id, None);
}

#[test]
fn runtime_event_accepts_unknown_optional_fields() {
    let payload = r#"{
      "type":"frontend_skill_intent",
      "turn_id":"turn-1",
      "intent":"skill_message",
      "slots":{"message_contact":"alex","message_text":"hi"},
      "unknown":"ignored"
    }"#;
    let event: RuntimeEvent = serde_json::from_str(payload)
        .unwrap_or_else(|error| panic!("event decode failed: {error}"));
    match event {
        RuntimeEvent::FrontendSkillIntent(intent) => {
            assert_eq!(intent.turn_id, "turn-1");
            assert_eq!(intent.intent, "skill_message");
        }
        _ => panic!("expected frontend skill intent"),
    }
}

#[test]
fn frontend_skill_result_serializes_error() {
    let result = FrontendSkillResultRequest {
        status: "error".to_string(),
        user_text: "send a message".to_string(),
        structured_result_context: None,
        error: Some("contact missing".to_string()),
    };
    let encoded =
        serde_json::to_string(&result).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("contact missing"));
}

#[test]
fn turn_chunk_roundtrip_json() {
    let request = TurnChunkRequest {
        session_id: "session-1".to_string(),
        chunk: "what's the weather".to_string(),
    };
    let encoded =
        serde_json::to_string(&request).unwrap_or_else(|error| panic!("encode failed: {error}"));
    let decoded: TurnChunkRequest =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.chunk, "what's the weather");
}

#[test]
fn frontend_activation_roundtrip_json() {
    let request = FrontendActivateRequest {
        device_id: "device-1".to_string(),
        session_id: "session-1".to_string(),
        platform: "macos".to_string(),
        frontend_version: "0.1.0".to_string(),
        supported_frontend_intents: vec!["skill_message".to_string(), "skill_timer".to_string()],
        expires_in_seconds: Some(90),
        protocol_version: Some(1),
    };
    let encoded =
        serde_json::to_string(&request).unwrap_or_else(|error| panic!("encode failed: {error}"));
    let decoded: FrontendActivateRequest =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.device_id, "device-1");
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.supported_frontend_intents.len(), 2);
}

#[test]
fn frontend_heartbeat_and_deactivate_roundtrip_json() {
    let heartbeat = FrontendHeartbeatRequest {
        device_id: "device-1".to_string(),
        session_id: "session-1".to_string(),
    };
    let deactivate = FrontendDeactivateRequest {
        device_id: "device-1".to_string(),
        session_id: "session-1".to_string(),
    };
    let heartbeat_encoded = serde_json::to_string(&heartbeat)
        .unwrap_or_else(|error| panic!("heartbeat encode failed: {error}"));
    let deactivate_encoded = serde_json::to_string(&deactivate)
        .unwrap_or_else(|error| panic!("deactivate encode failed: {error}"));
    let heartbeat_decoded: FrontendHeartbeatRequest = serde_json::from_str(&heartbeat_encoded)
        .unwrap_or_else(|error| panic!("heartbeat decode failed: {error}"));
    let deactivate_decoded: FrontendDeactivateRequest = serde_json::from_str(&deactivate_encoded)
        .unwrap_or_else(|error| panic!("deactivate decode failed: {error}"));
    assert_eq!(heartbeat_decoded.device_id, "device-1");
    assert_eq!(deactivate_decoded.session_id, "session-1");
}
