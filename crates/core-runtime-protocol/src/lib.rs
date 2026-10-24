use serde::{Deserialize, Serialize};

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
pub enum TurnStreamClientMessage {
    TurnStart {
        session_id: String,
        #[serde(default)]
        device_id: Option<String>,
        turn_id: String,
        #[serde(default)]
        supported_frontend_intents: Vec<String>,
        #[serde(default)]
        schema_version: Option<u32>,
    },
    TurnDone,
    TurnCancel,
    /// The room's help button was pressed; raise a staff ticket directly.
    HelpRequest,
    /// The client started playing an answer; the conversation stays awake.
    PlaybackStarted,
    /// The client finished (or stopped) playing an answer; the wake-word
    /// awake window runs from now.
    PlaybackFinished,
    /// The user talked over (or tapped to stop) the answer after about
    /// `spoken_ratio` (0 to 1) of it had played, so the conversation keeps
    /// only what was heard.
    PlaybackInterrupted {
        spoken_ratio: f32,
    },
    FrontendSkillResult {
        turn_id: String,
        intent_id: String,
        result: FrontendSkillResultRequest,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnStreamServerEvent {
    PartialTranscript {
        turn_id: String,
        text: String,
        stable: bool,
    },
    IntentUpdate {
        turn_id: String,
        intent: String,
    },
    Token {
        turn_id: String,
        text: String,
    },
    FrontendSkillIntent(FrontendSkillIntent),
    Done {
        turn_id: String,
    },
    /// Answer to `help_request`: the ticket (if the desk was reached) and what to say.
    HelpRaised {
        #[serde(default)]
        ticket_id: Option<String>,
        spoken: String,
    },
    Error {
        #[serde(default)]
        turn_id: Option<String>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::TurnStreamClientMessage;

    #[test]
    fn an_interruption_says_how_much_of_the_answer_was_heard() {
        let message: TurnStreamClientMessage =
            serde_json::from_str(r#"{"type":"playback_interrupted","spoken_ratio":0.4}"#)
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            message,
            TurnStreamClientMessage::PlaybackInterrupted { spoken_ratio: 0.4 }
        );
    }
}
