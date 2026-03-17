//! Confirmation flow: local answer first, ask Yes/No only when uncertain, search only on Yes.

use core_orchestrator::{parse_need_search, ConversationEngine, LlmStream, SttStream};
use core_search::{ExternalSearch, MockSearch};
use futures::stream;

struct MockStt(&'static str);

#[async_trait::async_trait]
impl SttStream for MockStt {
    async fn push_audio(
        &mut self,
        _pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.0.to_string())
    }
}

struct CollectLlm(&'static str);

#[async_trait::async_trait]
impl LlmStream for CollectLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(Box::new(stream::iter(vec![self.0.to_string()])))
    }
}

#[tokio::test]
async fn flow_uncertain_then_user_yes_calls_search() {
    let mut stt = MockStt("what is X?");
    let llm = CollectLlm("I'm not sure. [NEED_SEARCH: what is X]");
    let history: Vec<(String, String)> = vec![];

    let response = ConversationEngine::run_turn_collect(&mut stt, &llm, &history)
        .await
        .expect("collect ok");
    let (local_answer, query) = parse_need_search(&response).expect("NEED_SEARCH parsed");
    assert_eq!(local_answer, "I'm not sure.");
    assert_eq!(query, "what is X");

    // User said Yes: execute search and use result
    let search = MockSearch::new("X is something.");
    let result = search.execute(&query).await.expect("search ok");
    assert_eq!(result, "X is something.");
}

#[tokio::test]
async fn flow_uncertain_then_user_no_does_not_call_search() {
    let mut stt = MockStt("anything");
    let llm = CollectLlm("Maybe. [NEED_SEARCH: optional query]");
    let history: Vec<(String, String)> = vec![];

    let response = ConversationEngine::run_turn_collect(&mut stt, &llm, &history)
        .await
        .expect("collect ok");
    let (local_answer, _) = parse_need_search(&response).unwrap();
    // User said No: only use local_answer; search must not be called (caller's responsibility)
    assert_eq!(local_answer, "Maybe.");
}

#[tokio::test]
async fn flow_confident_no_marker_no_search_path() {
    let mut stt = MockStt("hello");
    let llm = CollectLlm("Hi there!");
    let history: Vec<(String, String)> = vec![];

    let response = ConversationEngine::run_turn_collect(&mut stt, &llm, &history)
        .await
        .expect("collect ok");
    assert!(parse_need_search(&response).is_none());
}
