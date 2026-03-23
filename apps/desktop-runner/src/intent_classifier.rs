//! LLM-based intent classifier: calls LLM with classification prompt and parses JSON.

use core_orchestrator::{
    intent_classifier_few_shots, intent_classifier_system_prompt, parse_intent, IntentClassifier,
    IntentDecision, LlmStream,
};
use futures::StreamExt;
use tracing::info;

/// Intent classifier that uses an LLM to classify user text; parses JSON response.
pub struct LlmIntentClassifier<'a, L> {
    pub llm: &'a L,
    pub system_prompt: String,
}

impl<'a, L> LlmIntentClassifier<'a, L> {
    pub fn new(llm: &'a L) -> Self {
        Self {
            llm,
            system_prompt: intent_classifier_system_prompt(),
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
        let few_shot_history = intent_classifier_few_shots();
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
        assert!(
            history.iter().any(|(u, a)| {
                u.contains("send a message to John saying running late")
                    && a.contains("\"intent\":\"skill_message\"")
                    && a.contains("\"command\":\"send\"")
                    && a.contains("\"message_contact\":\"John\"")
                    && a.contains("\"message_text\":\"running late\"")
            }),
            "canonical message send contract example missing from classifier history"
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
