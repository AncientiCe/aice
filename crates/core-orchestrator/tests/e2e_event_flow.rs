//! End-to-end event flow test with mock STT, LLM, TTS.
//! Defines expected behaviour: user text -> LLM stream -> TTS receives full response.
//! Barge-in test: cancel mid-stream yields Interrupted and fewer TTS pushes.

use async_trait::async_trait;
use core_orchestrator::{
    parse_need_search, ConversationEngine, LlmStream, SttStream, TtsSink, TurnOutcome,
};
use core_search::{ExternalSearch, MockSearch};
use futures::stream;
use std::time::Duration;
use tokio::sync::broadcast;

pub trait TestOptionExt<T> {
    fn must(self) -> T;
}

impl<T> TestOptionExt<T> for Option<T> {
    fn must(self) -> T {
        match self {
            Some(value) => value,
            None => panic!("expected Some(..) in test"),
        }
    }
}

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
    fn must_err(self) -> E;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }

    fn must_err(self) -> E {
        match self {
            Ok(_) => panic!("expected Err(..) in test, got Ok"),
            Err(error) => error,
        }
    }
}

struct MockStt {
    transcript: String,
}

impl MockStt {
    fn new(transcript: &str) -> Self {
        Self {
            transcript: transcript.to_string(),
        }
    }
}

#[async_trait]
impl SttStream for MockStt {
    async fn push_audio(
        &mut self,
        _pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.transcript.clone())
    }
}

struct MockLlm {
    response_tokens: Vec<String>,
}

impl MockLlm {
    fn new(response: &str) -> Self {
        Self {
            response_tokens: response.chars().map(|c| c.to_string()).collect(),
        }
    }
}

#[async_trait]
impl LlmStream for MockLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let tokens = self.response_tokens.clone();
        let s = stream::iter(tokens);
        Ok(Box::new(s))
    }
}

struct MockTts {
    pub received: Vec<String>,
}

impl MockTts {
    fn new() -> Self {
        Self {
            received: Vec::new(),
        }
    }
    fn full_text(&self) -> String {
        self.received.join("")
    }
}

#[async_trait]
impl TtsSink for MockTts {
    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.received.push(text.to_string());
        Ok(())
    }
    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

#[tokio::test]
async fn e2e_mock_stt_llm_tts_produces_complete_turn() {
    let mut stt = MockStt::new("hello");
    let llm = MockLlm::new("Hi there");
    let mut tts = MockTts::new();
    let history: Vec<(String, String)> = vec![];

    let outcome = ConversationEngine::run_turn(&mut stt, &llm, &mut tts, &history)
        .await
        .must();

    assert_eq!(outcome, TurnOutcome::Complete);
    assert_eq!(tts.full_text(), "Hi there");
}

/// LLM that yields tokens with delay so we can trigger cancel mid-stream.
struct SlowMockLlm {
    tokens: Vec<String>,
}

impl SlowMockLlm {
    fn new(tokens: Vec<&str>) -> Self {
        Self {
            tokens: tokens.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

struct ReceiverStream(tokio::sync::mpsc::UnboundedReceiver<String>);

impl futures::Stream for ReceiverStream {
    type Item = String;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}

#[async_trait]
impl LlmStream for SlowMockLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let tokens = self.tokens.clone();
        tokio::spawn(async move {
            for t in tokens {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if tx.send(t).is_err() {
                    break;
                }
            }
        });
        Ok(Box::new(ReceiverStream(rx)))
    }
}

#[tokio::test]
async fn barge_in_cancel_produces_interrupted_and_fewer_tts_pushes() {
    let mut stt = MockStt::new("stop");
    let llm = SlowMockLlm::new(vec!["A", "B", "C", "D", "E"]);
    let tts = std::sync::Arc::new(tokio::sync::Mutex::new(MockTts::new()));
    let tts_clone = std::sync::Arc::clone(&tts);
    let history: Vec<(String, String)> = vec![];

    let (tx, rx) = broadcast::channel(1);
    let run = tokio::spawn(async move {
        let mut t = tts_clone.lock().await;
        ConversationEngine::run_turn_with_cancel(&mut stt, &llm, &mut *t, &history, rx).await
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    let _ = tx.send(());
    let outcome = run.await.must().must();

    assert_eq!(outcome, TurnOutcome::Interrupted);
    let count = tts.lock().await.received.len();
    assert!(
        count < 5,
        "TTS should have received fewer than 5 tokens after cancel, got {}",
        count
    );
}

/// LLM that returns a single string (for collect + NEED_SEARCH tests).
struct CollectMockLlm {
    response: String,
}

#[async_trait]
impl LlmStream for CollectMockLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let s = stream::iter(vec![self.response.clone()]);
        Ok(Box::new(s))
    }
}

#[tokio::test]
async fn need_search_parse_then_user_confirm_yes_executes_search() {
    let mut stt = MockStt::new("what is the weather?");
    let llm = CollectMockLlm {
        response: "I'm not sure. [NEED_SEARCH: weather today]".to_string(),
    };
    let history: Vec<(String, String)> = vec![];

    let response = ConversationEngine::run_turn_collect(&mut stt, &llm, &history)
        .await
        .must();

    let (local_answer, query) = parse_need_search(&response).must();
    assert_eq!(local_answer, "I'm not sure.");
    assert_eq!(query, "weather today");

    // Simulate user said Yes: execute search.
    let search = MockSearch::new("Sunny, 72°F");
    let result = search.execute(&query).await.must();
    assert_eq!(result, "Sunny, 72°F");
}

#[tokio::test]
async fn need_search_user_says_no_use_local_answer_only() {
    let mut stt = MockStt::new("something");
    let llm = CollectMockLlm {
        response: "Maybe. [NEED_SEARCH: optional]".to_string(),
    };
    let history: Vec<(String, String)> = vec![];

    let response = ConversationEngine::run_turn_collect(&mut stt, &llm, &history)
        .await
        .must();

    let (local_answer, _query) = parse_need_search(&response).must();
    // User said No: we only use local_answer, never call search.
    assert_eq!(local_answer, "Maybe.");
}
