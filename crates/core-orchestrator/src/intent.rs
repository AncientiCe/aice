//! LLM-based intent classification: known skills vs chat.

use serde::Deserialize;
use std::fmt;

fn deserialize_optional_string_or_array<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Array(items) => {
            let values: Vec<String> = items
                .into_iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(s) => {
                        let t = s.trim().to_string();
                        if t.is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                    serde_json::Value::Null => None,
                    other => {
                        let t = other.to_string();
                        if t.trim().is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                })
                .collect();
            values.join(", ")
        }
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    if normalized.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

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
    /// Screenshot: save a local screenshot file.
    SkillScreenshot { filename: Option<String> },
    /// App switcher: switch apps, hide, quit, and force-quit actions.
    SkillAppSwitcher {
        action: Option<String>,
        target: Option<String>,
    },
    /// Reminder: create a macOS Reminders entry; title required, when is optional ISO date-time.
    SkillReminder {
        title: Option<String>,
        when: Option<String>,
    },
    /// Timer: start a macOS Clock timer; duration required, name is optional.
    SkillTimer {
        duration: Option<String>,
        name: Option<String>,
    },
    /// Shopping list: add or remove items in an Apple Notes shopping list for a given date.
    SkillShoppingList {
        action: Option<String>,
        items: Option<String>,
        when: Option<String>,
    },
    /// Message: send an iMessage to a contact.
    SkillMessage {
        command: Option<String>,
        contact: Option<String>,
        message: Option<String>,
    },
    /// Volume: set, adjust, mute/unmute, or query system output volume.
    SkillVolume {
        action: Option<String>,
        level: Option<u8>,
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
        command: Option<String>,
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
        #[serde(default)]
        screenshot_filename: Option<String>,
        #[serde(default)]
        app_switcher_action: Option<String>,
        #[serde(default)]
        app_switcher_target: Option<String>,
        #[serde(default)]
        reminder_title: Option<String>,
        #[serde(default)]
        reminder_when: Option<String>,
        #[serde(default)]
        timer_duration: Option<String>,
        #[serde(default)]
        timer_name: Option<String>,
        #[serde(default)]
        shopping_action: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_or_array")]
        shopping_items: Option<String>,
        #[serde(default)]
        shopping_when: Option<String>,
        #[serde(default)]
        message_contact: Option<String>,
        #[serde(default)]
        message_text: Option<String>,
        #[serde(default)]
        volume_action: Option<String>,
        #[serde(default)]
        volume_level: Option<u8>,
    }
    let p: Payload = serde_json::from_str(s).map_err(ParseIntentError::Json)?;
    let intent = p.intent.to_lowercase().trim().to_string();
    let location = p.location.filter(|l| !l.trim().is_empty());
    let origin = p.origin.filter(|o| !o.trim().is_empty());
    let destination = p.destination.filter(|d| !d.trim().is_empty());
    let opt_str = |o: Option<String>| o.filter(|x| !x.trim().is_empty());
    let command = opt_str(p.command);
    let action_from = |field: Option<String>, fallback: &Option<String>| {
        opt_str(field).or_else(|| fallback.clone())
    };

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
            action: action_from(p.smart_home_action, &command),
        }),
        "skill_assistant" | "assistant" => Ok(IntentDecision::SkillAssistant {
            kind: opt_str(p.assistant_kind),
        }),
        "skill_media" | "media" => Ok(IntentDecision::SkillMedia {
            action: action_from(p.media_action, &command),
            target: opt_str(p.media_target),
        }),
        "skill_memory" | "memory" => Ok(IntentDecision::SkillMemory {
            query: opt_str(p.memory_query),
            store: p.memory_store,
        }),
        "skill_computer" | "computer" => Ok(IntentDecision::SkillComputer {
            action: action_from(p.computer_action, &command),
            target: opt_str(p.computer_target),
        }),
        "skill_screenshot" | "screenshot" => Ok(IntentDecision::SkillScreenshot {
            filename: opt_str(p.screenshot_filename),
        }),
        "skill_app_switcher" | "app_switcher" => Ok(IntentDecision::SkillAppSwitcher {
            action: action_from(p.app_switcher_action, &command),
            target: opt_str(p.app_switcher_target),
        }),
        "skill_reminder" | "reminder" => Ok(IntentDecision::SkillReminder {
            title: opt_str(p.reminder_title),
            when: opt_str(p.reminder_when),
        }),
        "skill_timer" | "timer" => Ok(IntentDecision::SkillTimer {
            duration: opt_str(p.timer_duration),
            name: opt_str(p.timer_name),
        }),
        "skill_shopping_list" | "shopping_list" => Ok(IntentDecision::SkillShoppingList {
            action: action_from(p.shopping_action, &command),
            items: opt_str(p.shopping_items),
            when: opt_str(p.shopping_when),
        }),
        "skill_message" | "message" => Ok(IntentDecision::SkillMessage {
            command: command.clone(),
            contact: opt_str(p.message_contact),
            message: opt_str(p.message_text),
        }),
        "skill_volume" | "volume" => Ok(IntentDecision::SkillVolume {
            action: action_from(p.volume_action, &command),
            level: p.volume_level,
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
    pub trait TestResultExt<T, E> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(value) => value,
                Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
            }
        }
    }
    use super::{parse_intent, IntentDecision};

    #[test]
    fn parse_intent_smart_home() {
        let raw = r#"{"intent": "skill_smart_home", "smart_home_target": "living room", "smart_home_action": "turn off"}"#;
        let d = parse_intent(raw).must();
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
        let d = parse_intent(raw).must();
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
        let d = parse_intent(raw).must();
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
        let d = parse_intent(raw).must();
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
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillComputer { action, target } => {
                assert_eq!(action.as_deref(), Some("open"));
                assert_eq!(target.as_deref(), Some("browser"));
            }
            _ => panic!("expected SkillComputer"),
        }
    }

    #[test]
    fn parse_intent_screenshot() {
        let raw = r#"{"intent":"skill_screenshot","screenshot_filename":"desk.png"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillScreenshot { filename } => {
                assert_eq!(filename.as_deref(), Some("desk.png"));
            }
            _ => panic!("expected SkillScreenshot"),
        }
    }

    #[test]
    fn parse_intent_reminder_with_when() {
        let raw = r#"{"intent": "skill_reminder", "reminder_title": "Call mom", "reminder_when": "2026-03-20T17:00"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillReminder { title, when } => {
                assert_eq!(title.as_deref(), Some("Call mom"));
                assert_eq!(when.as_deref(), Some("2026-03-20T17:00"));
            }
            _ => panic!("expected SkillReminder"),
        }
    }

    #[test]
    fn parse_intent_reminder_without_when() {
        let raw = r#"{"intent": "skill_reminder", "reminder_title": "Buy groceries"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillReminder { title, when } => {
                assert_eq!(title.as_deref(), Some("Buy groceries"));
                assert!(when.is_none());
            }
            _ => panic!("expected SkillReminder"),
        }
    }

    #[test]
    fn parse_intent_timer_with_name() {
        let raw = r#"{"intent": "skill_timer", "timer_duration": "5 minutes", "timer_name": "pasta timer"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTimer { duration, name } => {
                assert_eq!(duration.as_deref(), Some("5 minutes"));
                assert_eq!(name.as_deref(), Some("pasta timer"));
            }
            _ => panic!("expected SkillTimer"),
        }
    }

    #[test]
    fn parse_intent_timer_without_name() {
        let raw = r#"{"intent": "skill_timer", "timer_duration": "30 minutes"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTimer { duration, name } => {
                assert_eq!(duration.as_deref(), Some("30 minutes"));
                assert!(name.is_none());
            }
            _ => panic!("expected SkillTimer"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_add() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": "strawberries, salami", "shopping_when": "today"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("strawberries, salami"));
                assert_eq!(when.as_deref(), Some("today"));
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_no_when_defaults_to_none() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": "milk"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("milk"));
                assert!(when.is_none());
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_items_array_is_normalized() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": ["strawberries", "salami"], "shopping_when": "today"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("strawberries, salami"));
                assert_eq!(when.as_deref(), Some("today"));
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_message() {
        let raw = r#"{"intent":"skill_message","command":"send","message_contact":"my wife","message_text":"How are you?"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMessage {
                command,
                contact,
                message,
            } => {
                assert_eq!(command.as_deref(), Some("send"));
                assert_eq!(contact.as_deref(), Some("my wife"));
                assert_eq!(message.as_deref(), Some("How are you?"));
            }
            _ => panic!("expected SkillMessage"),
        }
    }

    #[test]
    fn parse_intent_uses_global_command_for_action_skills() {
        let raw = r#"{"intent":"skill_volume","command":"mute"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("mute"));
                assert_eq!(*level, None);
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_volume_set_level() {
        let raw = r#"{"intent":"skill_volume","volume_action":"set","volume_level":40}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("set"));
                assert_eq!(*level, Some(40));
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_volume_without_level() {
        let raw = r#"{"intent":"volume","volume_action":"mute"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("mute"));
                assert_eq!(*level, None);
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_app_switcher_with_target() {
        let raw = r#"{"intent":"skill_app_switcher","app_switcher_action":"switch","app_switcher_target":"Safari"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAppSwitcher { action, target } => {
                assert_eq!(action.as_deref(), Some("switch"));
                assert_eq!(target.as_deref(), Some("Safari"));
            }
            _ => panic!("expected SkillAppSwitcher"),
        }
    }

    #[test]
    fn parse_intent_app_switcher_without_target() {
        let raw = r#"{"intent":"app_switcher","app_switcher_action":"next"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAppSwitcher { action, target } => {
                assert_eq!(action.as_deref(), Some("next"));
                assert!(target.is_none());
            }
            _ => panic!("expected SkillAppSwitcher"),
        }
    }
}
