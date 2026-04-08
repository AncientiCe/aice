use core_runtime_protocol::{
    AudioChunkRequest, AudioFinalizeRequest, DoneReason, FrontendActivateRequest,
    FrontendDeactivateRequest, FrontendHeartbeatRequest, FrontendSkillResultRequest, RuntimeEvent,
    CURRENT_PROTOCOL_VERSION,
};

#[test]
fn audio_chunk_request_roundtrip_json() {
    let request = AudioChunkRequest {
        session_id: "session-1".to_string(),
        device_id: Some("device-1".to_string()),
        turn_id: "turn-1".to_string(),
        seq: 3,
        pcm_s16le_base64: "AQID".to_string(),
        sample_rate_hz: 16_000,
        channels: 1,
    };
    let encoded =
        serde_json::to_string(&request).unwrap_or_else(|error| panic!("encode failed: {error}"));
    let decoded: AudioChunkRequest =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.device_id.as_deref(), Some("device-1"));
    assert_eq!(decoded.turn_id, "turn-1");
    assert_eq!(decoded.seq, 3);
    assert_eq!(decoded.pcm_s16le_base64, "AQID");
    assert_eq!(decoded.sample_rate_hz, 16_000);
    assert_eq!(decoded.channels, 1);
}

#[test]
fn audio_finalize_request_roundtrip_json() {
    let payload = r#"{"session_id":"session-1","device_id":"device-1","turn_id":"turn-1","done_reason":"vad_end"}"#;
    let decoded: AudioFinalizeRequest =
        serde_json::from_str(payload).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.device_id.as_deref(), Some("device-1"));
    assert_eq!(decoded.turn_id, "turn-1");
    assert_eq!(decoded.done_reason, DoneReason::VadEnd);
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
fn frontend_activation_roundtrip_json() {
    let request = FrontendActivateRequest {
        device_id: "device-1".to_string(),
        session_id: "session-1".to_string(),
        platform: "macos".to_string(),
        frontend_version: "0.1.0".to_string(),
        supported_frontend_intents: vec!["skill_message".to_string(), "skill_timer".to_string()],
        expires_in_seconds: Some(90),
        protocol_version: Some(2),
    };
    let encoded =
        serde_json::to_string(&request).unwrap_or_else(|error| panic!("encode failed: {error}"));
    let decoded: FrontendActivateRequest =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded.device_id, "device-1");
    assert_eq!(decoded.session_id, "session-1");
    assert_eq!(decoded.supported_frontend_intents.len(), 2);
    assert_eq!(decoded.protocol_version, Some(2));
}

#[test]
fn current_protocol_version_matches_frontend_handshake_version() {
    assert_eq!(CURRENT_PROTOCOL_VERSION, 2);
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
