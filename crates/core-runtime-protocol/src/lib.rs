use serde::{Deserialize, Serialize};

pub const CURRENT_PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioChunkRequest {
    pub session_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
    pub turn_id: String,
    pub seq: u64,
    pub pcm_s16le_base64: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioFinalizeRequest {
    pub session_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
    pub turn_id: String,
    pub done_reason: DoneReason,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DoneReason {
    VadEnd,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TurnRequest {
    pub session_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub transcript: String,
    #[serde(default = "default_true")]
    pub finalize: bool,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrontendActivateRequest {
    pub device_id: String,
    pub session_id: String,
    pub platform: String,
    pub frontend_version: String,
    #[serde(default)]
    pub supported_frontend_intents: Vec<String>,
    #[serde(default)]
    pub expires_in_seconds: Option<u64>,
    #[serde(default)]
    pub protocol_version: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrontendHeartbeatRequest {
    pub device_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrontendDeactivateRequest {
    pub device_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrontendSkillIntent {
    pub turn_id: String,
    pub intent: String,
    pub slots: serde_json::Value,
    #[serde(default)]
    pub user_text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrontendSkillResultRequest {
    pub status: String,
    pub user_text: String,
    #[serde(default)]
    pub structured_result_context: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    Token { text: String },
    FrontendSkillIntent(FrontendSkillIntent),
    Done,
    Error { message: String },
}

pub fn sse_data_line(event: &RuntimeEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|json| format!("data: {json}\n\n"))
}
