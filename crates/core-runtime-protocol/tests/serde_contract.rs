use core_runtime_protocol::{
    DoneReason, FrontendSkillIntent, FrontendSkillResultRequest, TurnStreamClientMessage,
    TurnStreamServerEvent,
};

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
fn turn_stream_client_turn_start_roundtrip_json() {
    let payload = r#"{"type":"turn_start","session_id":"s1","device_id":"d1","turn_id":"t1","supported_frontend_intents":["skill_timer","skill_message"],"schema_version":1}"#;
    let decoded: TurnStreamClientMessage =
        serde_json::from_str(payload).unwrap_or_else(|error| panic!("decode failed: {error}"));
    match decoded {
        TurnStreamClientMessage::TurnStart {
            session_id,
            device_id,
            turn_id,
            supported_frontend_intents,
            schema_version,
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(device_id.as_deref(), Some("d1"));
            assert_eq!(turn_id, "t1");
            assert_eq!(supported_frontend_intents.len(), 2);
            assert_eq!(schema_version, Some(1));
        }
        _ => panic!("expected turn_start"),
    }
}

#[test]
fn turn_stream_client_turn_start_minimal_roundtrip() {
    let payload = r#"{"type":"turn_start","session_id":"s1","turn_id":"t1"}"#;
    let decoded: TurnStreamClientMessage =
        serde_json::from_str(payload).unwrap_or_else(|error| panic!("decode failed: {error}"));
    match decoded {
        TurnStreamClientMessage::TurnStart {
            supported_frontend_intents,
            schema_version,
            device_id,
            ..
        } => {
            assert!(supported_frontend_intents.is_empty());
            assert_eq!(schema_version, None);
            assert_eq!(device_id, None);
        }
        _ => panic!("expected turn_start"),
    }
}

#[test]
fn turn_stream_client_turn_done_roundtrip() {
    let msg = TurnStreamClientMessage::TurnDone;
    let encoded =
        serde_json::to_string(&msg).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"turn_done\""));
    let decoded: TurnStreamClientMessage =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded, TurnStreamClientMessage::TurnDone);
}

#[test]
fn turn_stream_client_turn_cancel_roundtrip() {
    let msg = TurnStreamClientMessage::TurnCancel;
    let encoded =
        serde_json::to_string(&msg).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"turn_cancel\""));
}

#[test]
fn turn_stream_client_frontend_skill_result_roundtrip() {
    let msg = TurnStreamClientMessage::FrontendSkillResult {
        turn_id: "turn-1".to_string(),
        intent_id: "skill_message".to_string(),
        result: FrontendSkillResultRequest {
            status: "success".to_string(),
            user_text: "send alex hi".to_string(),
            structured_result_context: Some("Message sent".to_string()),
            error: None,
        },
    };
    let encoded =
        serde_json::to_string(&msg).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"frontend_skill_result\""));
    assert!(encoded.contains("turn-1"));
    let decoded: TurnStreamClientMessage =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    match decoded {
        TurnStreamClientMessage::FrontendSkillResult {
            turn_id,
            intent_id,
            result,
        } => {
            assert_eq!(turn_id, "turn-1");
            assert_eq!(intent_id, "skill_message");
            assert_eq!(result.status, "success");
        }
        _ => panic!("expected frontend_skill_result"),
    }
}

#[test]
fn turn_stream_server_event_partial_transcript_roundtrip() {
    let event = TurnStreamServerEvent::PartialTranscript {
        turn_id: "turn-1".to_string(),
        text: "hello".to_string(),
        stable: true,
    };
    let encoded =
        serde_json::to_string(&event).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"partial_transcript\""));
    let decoded: TurnStreamServerEvent =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    match decoded {
        TurnStreamServerEvent::PartialTranscript {
            turn_id,
            text,
            stable,
        } => {
            assert_eq!(turn_id, "turn-1");
            assert_eq!(text, "hello");
            assert!(stable);
        }
        _ => panic!("expected partial_transcript"),
    }
}

#[test]
fn turn_stream_server_event_token_roundtrip() {
    let event = TurnStreamServerEvent::Token {
        turn_id: "turn-1".to_string(),
        text: "answer text".to_string(),
    };
    let encoded =
        serde_json::to_string(&event).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"token\""));
    let decoded: TurnStreamServerEvent =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded, event);
}

#[test]
fn turn_stream_server_event_frontend_skill_intent_roundtrip() {
    let event = TurnStreamServerEvent::FrontendSkillIntent(FrontendSkillIntent {
        turn_id: "turn-1".to_string(),
        intent: "skill_message".to_string(),
        slots: serde_json::json!({"contact": "alex"}),
        user_text: "send alex hi".to_string(),
    });
    let encoded =
        serde_json::to_string(&event).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"frontend_skill_intent\""));
    let decoded: TurnStreamServerEvent =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded, event);
}

#[test]
fn turn_stream_server_event_done_roundtrip() {
    let event = TurnStreamServerEvent::Done {
        turn_id: "turn-1".to_string(),
    };
    let encoded =
        serde_json::to_string(&event).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"done\""));
}

#[test]
fn turn_stream_server_event_error_roundtrip() {
    let event = TurnStreamServerEvent::Error {
        turn_id: Some("turn-1".to_string()),
        message: "something failed".to_string(),
    };
    let encoded =
        serde_json::to_string(&event).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert!(encoded.contains("\"type\":\"error\""));
    assert!(encoded.contains("something failed"));
}

#[test]
fn done_reason_roundtrip() {
    let reason = DoneReason::VadEnd;
    let encoded =
        serde_json::to_string(&reason).unwrap_or_else(|error| panic!("encode failed: {error}"));
    assert_eq!(encoded, "\"vad_end\"");
    let decoded: DoneReason =
        serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("decode failed: {error}"));
    assert_eq!(decoded, DoneReason::VadEnd);
}
