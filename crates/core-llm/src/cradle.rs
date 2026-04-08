//! Cradle in-process LLM stream adapter.
//!
//! Current implementation delegates transport to the existing Ollama-compatible
//! backend while exposing a Cradle provider surface for backend integration.

use crate::ollama::OllamaLlmStream;
use async_trait::async_trait;
use core_orchestrator::{LlmCallOptions, LlmStream};
use futures::Stream;

/// Cradle provider implementation used by backend runtime.
pub struct CradleLlmStream {
    inner: OllamaLlmStream,
}

impl CradleLlmStream {
    pub fn new(
        base_url: String,
        model: String,
        short_replies: bool,
        max_output_tokens: u32,
        system_prompt: Option<String>,
    ) -> Self {
        Self {
            inner: OllamaLlmStream::new(
                base_url,
                model,
                short_replies,
                max_output_tokens,
                system_prompt,
            ),
        }
    }

    pub async fn chat_once(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.inner
            .chat_once(user_text, history, system_prompt_override, call_options)
            .await
    }
}

#[async_trait]
impl LlmStream for CradleLlmStream {
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<
        Box<dyn Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        self.inner
            .chat_stream(user_text, history, system_prompt_override, call_options)
            .await
    }
}
