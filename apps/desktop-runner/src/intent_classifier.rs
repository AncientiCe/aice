//! LLM-based intent classifier: calls LLM with classification prompt and parses JSON.

use core_orchestrator::{parse_intent, IntentClassifier, IntentDecision, LlmStream};
use futures::StreamExt;
use tracing::info;

const CLASSIFICATION_SYSTEM_PROMPT: &str = "You are an intent classifier. Reply with only a JSON object, no other text. \
Use \"intent\": one of \"chat\", \"skill_weather\", \"skill_time\", \"skill_distance\", \"skill_smart_home\", \"skill_assistant\", \"skill_media\", \"skill_memory\", \"skill_computer\", \"skill_screenshot\", \"skill_app_switcher\", \"skill_reminder\", \"skill_timer\", \"skill_shopping_list\", \"skill_message\", \"skill_volume\". \
When the user names a place, output an EXACT location string (city + country), e.g. \"Rome, Italy\". \
Weather: ANY weather question is skill_weather. Set \"location\" only when they name a place. \
Time: \"what time is it\", \"time in X\". Omit \"location\" unless they name a place. \
Distance: \"origin\" and/or \"destination\" as full location strings. One place = \"destination\" only. \
Smart home: lights, thermostat, locks, scenes, devices. Use \"skill_smart_home\"; optional \"smart_home_target\" (e.g. \"living room\"), \"smart_home_action\" (e.g. \"turn off\"). \
Assistant: calendar and assistant-only actions. Use \"skill_assistant\"; optional \"assistant_kind\": \"calendar\". \
Media: play, pause, volume, multi-room, \"play X in Y\". Use \"skill_media\"; optional \"media_action\", \"media_target\". \
Memory: remember/recall facts, \"remember that\", \"what did I say about X\". Use \"skill_memory\"; optional \"memory_query\", \"memory_store\": true when storing. \
Computer: open app, browser, run script, file operations. Use \"skill_computer\"; optional \"computer_action\", \"computer_target\". \
Screenshot: take a local screenshot. Use \"skill_screenshot\"; optional \"screenshot_filename\" (e.g. \"meeting-notes.png\") when user asks for a specific filename. \
App switcher: switch/focus apps, cycle next/previous, hide apps, quit, force quit. Use \"skill_app_switcher\"; optional \"app_switcher_action\" (\"switch\", \"next\", \"previous\", \"hide\", \"hide_others\", \"show_all_windows\", \"quit\", \"force_quit\"), optional \"app_switcher_target\" (required for switch/hide/quit/force_quit). For user words like \"close\" or \"exit\" an app, output \"quit\". \
Reminder: add/create a reminder. Use \"skill_reminder\"; \"reminder_title\" = what to be reminded about; optional \"reminder_when\" = date-time in ISO 8601 (e.g. \"2026-03-20T17:00\") when the user specifies a time. Omit \"reminder_when\" if no time given. \
Timer: set/start a timer. Use \"skill_timer\"; \"timer_duration\" = duration string (e.g. \"5 minutes\", \"1 hour 30 minutes\"); optional \"timer_name\" = label the user gives the timer. \
Shopping list: add/remove items from a shopping list. Use \"skill_shopping_list\"; \"shopping_action\": \"add\" or \"remove\"; \"shopping_items\" = comma-separated items to add/remove; optional \"shopping_when\" = target date (e.g. \"today\", \"tomorrow\", \"saturday\", \"2026-03-20\"). Default when not specified: \"today\". \
Message: send a message to a contact via iMessage. Use \"skill_message\"; \"message_contact\" = target contact phrase (e.g. \"my wife\", \"John Doe\"); \"message_text\" = what to send (e.g. \"How are you?\"). Never classify message-sending as \"skill_assistant\". \
Volume: set/change/mute/unmute/query system volume. Use \"skill_volume\"; optional \"volume_action\" (\"set\", \"up\", \"down\", \"mute\", \"unmute\", \"get\"), optional \"volume_level\" integer 0-100 (only for \"set\"). \
Do NOT use \"skill_assistant\" for sending iMessages. \
Examples: {\"intent\": \"skill_reminder\", \"reminder_title\": \"Call mom\", \"reminder_when\": \"2026-03-20T17:00\"}; {\"intent\": \"skill_timer\", \"timer_duration\": \"5 minutes\"}; {\"intent\": \"skill_shopping_list\", \"shopping_action\": \"add\", \"shopping_items\": \"strawberries, salami, celery\"}; {\"intent\": \"skill_message\", \"message_contact\": \"my wife\", \"message_text\": \"How are you?\"}; {\"intent\": \"skill_message\", \"message_contact\": \"my wife\", \"message_text\": \"how she's doing\"}; {\"intent\": \"skill_volume\", \"volume_action\": \"set\", \"volume_level\": 40}; {\"intent\": \"skill_screenshot\", \"screenshot_filename\": \"desk.png\"}; {\"intent\": \"skill_app_switcher\", \"app_switcher_action\": \"switch\", \"app_switcher_target\": \"Safari\"}; {\"intent\": \"skill_app_switcher\", \"app_switcher_action\": \"quit\", \"app_switcher_target\": \"Music\"}; {\"intent\": \"skill_app_switcher\", \"app_switcher_action\": \"force_quit\", \"app_switcher_target\": \"Safari\"}.";

const CLASSIFICATION_FEW_SHOTS: [(&str, &str); 6] = [
    (
        "how far is Paris?",
        "{\"intent\":\"skill_distance\",\"destination\":\"Paris, France\"}",
    ),
    (
        "what's the weather in Paris?",
        "{\"intent\":\"skill_weather\",\"location\":\"Paris, France\"}",
    ),
    (
        "what's the weather?",
        "{\"intent\":\"skill_weather\"}",
    ),
    (
        "time in Tokyo",
        "{\"intent\":\"skill_time\",\"location\":\"Tokyo, Japan\"}",
    ),
    (
        "set a timer for 5 minutes",
        "{\"intent\":\"skill_timer\",\"timer_duration\":\"5 minutes\"}",
    ),
    (
        "send a message to John saying running late",
        "{\"intent\":\"skill_message\",\"message_contact\":\"John\",\"message_text\":\"running late\"}",
    ),
];

/// Intent classifier that uses an LLM to classify user text; parses JSON response.
pub struct LlmIntentClassifier<'a, L> {
    pub llm: &'a L,
    pub system_prompt: String,
}

impl<'a, L> LlmIntentClassifier<'a, L> {
    pub fn new(llm: &'a L) -> Self {
        Self {
            llm,
            system_prompt: CLASSIFICATION_SYSTEM_PROMPT.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl<L> IntentClassifier for LlmIntentClassifier<'_, L>
where
    L: LlmStream + Send + Sync,
{
    async fn classify(
        &self,
        user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>> {
        let user_message = format!(
            "Classify this user request. Reply with only the JSON object.\nUser request: \"{}\"",
            user_text.trim()
        );
        let few_shot_history: Vec<(String, String)> = CLASSIFICATION_FEW_SHOTS
            .iter()
            .map(|(u, a)| (u.to_string(), a.to_string()))
            .collect();
        let llm_history =
            serde_json::to_string(&few_shot_history).unwrap_or_else(|_| "[]".to_string());
        info!(
            llm_input = %user_message.trim(),
            llm_history = %llm_history,
            "llm_intent_input"
        );
        let mut stream = self
            .llm
            .chat_stream(
                &user_message,
                few_shot_history.as_slice(),
                Some(self.system_prompt.as_str()),
            )
            .await?;
        let mut raw = String::new();
        while let Some(token) = stream.next().await {
            raw.push_str(&token);
        }
        info!(llm_output = %raw.trim(), "llm_intent_output");
        let decision = parse_intent(raw.trim())?;
        info!(intent_decision = ?decision, "llm_intent_decision");
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::LlmIntentClassifier;
    use async_trait::async_trait;
    use core_orchestrator::{IntentClassifier, IntentDecision, LlmStream};
    use futures::stream;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

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

    struct RecordingLlm {
        response: String,
        last_history: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl RecordingLlm {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                last_history: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn last_history(&self) -> Vec<(String, String)> {
            self.last_history.lock().must().clone()
        }
    }

    #[async_trait]
    impl LlmStream for RecordingLlm {
        async fn chat_stream(
            &self,
            _user_text: &str,
            history: &[(String, String)],
            _system_prompt_override: Option<&str>,
        ) -> Result<
            Box<dyn futures::Stream<Item = String> + Send + Unpin>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            *self.last_history.lock().must() = history.to_vec();
            Ok(Box::new(stream::iter(vec![self.response.clone()])))
        }
    }

    #[tokio::test]
    async fn classifier_provides_contrastive_few_shot_history() {
        let llm = RecordingLlm::new(r#"{"intent":"chat"}"#);
        let classifier = LlmIntentClassifier::new(&llm);

        classifier.classify("how far is Paris?").await.must();
        let history = llm.last_history();

        assert!(
            history.iter().any(|(u, a)| {
                u.contains("how far is Paris?")
                    && a.contains("\"intent\":\"skill_distance\"")
                    && a.contains("\"destination\":\"Paris, France\"")
            }),
            "distance disambiguation example missing from classifier history"
        );
        assert!(
            history.iter().any(|(u, a)| {
                u.contains("what's the weather in Paris?")
                    && a.contains("\"intent\":\"skill_weather\"")
                    && a.contains("\"location\":\"Paris, France\"")
            }),
            "weather location extraction example missing from classifier history"
        );
    }

    #[tokio::test]
    async fn classifier_parses_distance_decision_from_llm_json() {
        let llm = RecordingLlm::new(r#"{"intent":"skill_distance","destination":"Paris, France"}"#);
        let classifier = LlmIntentClassifier::new(&llm);

        let decision = classifier.classify("how far is Paris?").await.must();

        assert_eq!(
            decision,
            IntentDecision::SkillDistance {
                origin: None,
                destination: Some("Paris, France".to_string())
            }
        );
    }

    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().must().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct SharedMakeWriter {
        sink: Arc<Mutex<Vec<u8>>>,
    }

    impl<'a> MakeWriter<'a> for SharedMakeWriter {
        type Writer = SharedBuf;

        fn make_writer(&'a self) -> Self::Writer {
            SharedBuf(self.sink.clone())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn classifier_logs_intent_llm_input_and_output() {
        let llm = RecordingLlm::new(r#"{"intent":"skill_weather","location":"Paris, France"}"#);
        let classifier = LlmIntentClassifier::new(&llm);
        let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
        let make_writer = SharedMakeWriter { sink: sink.clone() };

        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(make_writer)
            .without_time()
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        classifier
            .classify("what's the weather in Paris?")
            .await
            .must();
        drop(_guard);

        let output = String::from_utf8(sink.lock().must().clone()).must();
        assert!(
            output.contains("llm_intent_input"),
            "expected classifier input log entry, got: {}",
            output
        );
        assert!(
            output.contains("llm_intent_output"),
            "expected classifier output log entry, got: {}",
            output
        );
    }
}
