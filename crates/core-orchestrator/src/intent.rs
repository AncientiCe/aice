//! LLM-based intent classification: known skills vs chat.

use std::fmt;

/// Result of classifying user input (from LLM, parsed from JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntentDecision {
    /// Normal chat / question / unknown; use main chat flow.
    Chat,
    /// Weather skill; optional location override (e.g. "Rome").
    SkillWeather { location: Option<String> },
    /// Time skill; optional location (if absent, use current/default location).
    SkillTime { location: Option<String> },
    /// Distance skill; origin and/or destination. If only one place, it is destination (origin = current).
    SkillDistance {
        origin: Option<String>,
        destination: Option<String>,
    },
    /// Smart home: lights, climate, scenes, device control.
    SkillSmartHome {
        target: Option<String>,
        action: Option<String>,
    },
    /// Personal assistant: calendar, reminders, messages.
    SkillAssistant { kind: Option<String> },
    /// Media: playback, multi-room, source selection.
    SkillMedia {
        action: Option<String>,
        target: Option<String>,
    },
    /// Knowledge/memory: remember, recall, personal knowledge.
    SkillMemory {
        query: Option<String>,
        store: Option<bool>,
    },
    /// Computer-use: browser, apps, files.
    SkillComputer {
        action: Option<String>,
        target: Option<String>,
    },
}

/// Parse LLM classifier output into IntentDecision. Expects a single JSON object.
/// Tolerates surrounding whitespace and optional markdown code fence.
pub fn parse_intent(raw: &str) -> Result<IntentDecision, ParseIntentError> {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s)
        .strip_suffix("```")
        .unwrap_or(s)
        .trim();
    #[derive(serde::Deserialize)]
    struct Payload {
        intent: String,
        #[serde(default)]
        location: Option<String>,
        #[serde(default)]
        origin: Option<String>,
        #[serde(default)]
        destination: Option<String>,
        #[serde(default)]
        smart_home_target: Option<String>,
        #[serde(default)]
        smart_home_action: Option<String>,
        #[serde(default)]
        assistant_kind: Option<String>,
        #[serde(default)]
        media_action: Option<String>,
        #[serde(default)]
        media_target: Option<String>,
        #[serde(default)]
        memory_query: Option<String>,
        #[serde(default)]
        memory_store: Option<bool>,
        #[serde(default)]
        computer_action: Option<String>,
        #[serde(default)]
        computer_target: Option<String>,
    }
    let p: Payload = serde_json::from_str(s).map_err(ParseIntentError::Json)?;
    let intent = p.intent.to_lowercase().trim().to_string();
    let location = p.location.filter(|l| !l.trim().is_empty());
    let origin = p.origin.filter(|o| !o.trim().is_empty());
    let destination = p.destination.filter(|d| !d.trim().is_empty());
    let opt_str = |o: Option<String>| o.filter(|x| !x.trim().is_empty());
    match intent.as_str() {
        "chat" => Ok(IntentDecision::Chat),
        "skill_weather" | "weather" => Ok(IntentDecision::SkillWeather { location }),
        "skill_time" | "time" => Ok(IntentDecision::SkillTime { location }),
        "skill_distance" | "distance" => Ok(IntentDecision::SkillDistance {
            origin,
            destination,
        }),
        "skill_smart_home" | "smart_home" => Ok(IntentDecision::SkillSmartHome {
            target: opt_str(p.smart_home_target),
            action: opt_str(p.smart_home_action),
        }),
        "skill_assistant" | "assistant" => Ok(IntentDecision::SkillAssistant {
            kind: opt_str(p.assistant_kind),
        }),
        "skill_media" | "media" => Ok(IntentDecision::SkillMedia {
            action: opt_str(p.media_action),
            target: opt_str(p.media_target),
        }),
        "skill_memory" | "memory" => Ok(IntentDecision::SkillMemory {
            query: opt_str(p.memory_query),
            store: p.memory_store,
        }),
        "skill_computer" | "computer" => Ok(IntentDecision::SkillComputer {
            action: opt_str(p.computer_action),
            target: opt_str(p.computer_target),
        }),
        _ => Ok(IntentDecision::Chat),
    }
}

#[derive(Debug)]
pub enum ParseIntentError {
    Json(serde_json::Error),
}

impl fmt::Display for ParseIntentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseIntentError::Json(e) => write!(f, "intent JSON parse error: {}", e),
        }
    }
}

impl std::error::Error for ParseIntentError {}

/// Async classifier: user text -> intent decision (implementations call LLM and parse).
#[async_trait::async_trait]
pub trait IntentClassifier: Send + Sync {
    async fn classify(
        &self,
        user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>>;
}

#[cfg(test)]
mod tests {
    use super::{parse_intent, IntentDecision};

    #[test]
    fn parse_intent_smart_home() {
        let raw = r#"{"intent": "skill_smart_home", "smart_home_target": "living room", "smart_home_action": "turn off"}"#;
        let d = parse_intent(raw).unwrap();
        match &d {
            IntentDecision::SkillSmartHome { target, action } => {
                assert_eq!(target.as_deref(), Some("living room"));
                assert_eq!(action.as_deref(), Some("turn off"));
            }
            _ => panic!("expected SkillSmartHome"),
        }
    }

    #[test]
    fn parse_intent_assistant() {
        let raw = r#"{"intent": "skill_assistant", "assistant_kind": "calendar"}"#;
        let d = parse_intent(raw).unwrap();
        match &d {
            IntentDecision::SkillAssistant { kind } => {
                assert_eq!(kind.as_deref(), Some("calendar"));
            }
            _ => panic!("expected SkillAssistant"),
        }
    }

    #[test]
    fn parse_intent_media() {
        let raw = r#"{"intent": "skill_media", "media_action": "play", "media_target": "kitchen"}"#;
        let d = parse_intent(raw).unwrap();
        match &d {
            IntentDecision::SkillMedia { action, target } => {
                assert_eq!(action.as_deref(), Some("play"));
                assert_eq!(target.as_deref(), Some("kitchen"));
            }
            _ => panic!("expected SkillMedia"),
        }
    }

    #[test]
    fn parse_intent_memory() {
        let raw = r#"{"intent": "skill_memory", "memory_query": "where did I leave keys", "memory_store": true}"#;
        let d = parse_intent(raw).unwrap();
        match &d {
            IntentDecision::SkillMemory { query, store } => {
                assert_eq!(query.as_deref(), Some("where did I leave keys"));
                assert_eq!(*store, Some(true));
            }
            _ => panic!("expected SkillMemory"),
        }
    }

    #[test]
    fn parse_intent_computer() {
        let raw = r#"{"intent": "skill_computer", "computer_action": "open", "computer_target": "browser"}"#;
        let d = parse_intent(raw).unwrap();
        match &d {
            IntentDecision::SkillComputer { action, target } => {
                assert_eq!(action.as_deref(), Some("open"));
                assert_eq!(target.as_deref(), Some("browser"));
            }
            _ => panic!("expected SkillComputer"),
        }
    }
}
