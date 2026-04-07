//! Abstract traits for STT, LLM, TTS to allow mocking in tests.

use async_trait::async_trait;
use futures::Stream;

/// Streaming STT: consumes audio, yields partial/final text.
#[async_trait]
pub trait SttStream: Send + Sync {
    /// Push audio chunk (16kHz mono 16-bit PCM). Caller decides chunk size.
    async fn push_audio(
        &mut self,
        pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// Flush and return final transcript for current utterance.
    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

/// Per-call options that override instance-level defaults for a single LLM invocation.
pub struct LlmCallOptions {
    /// Override the model temperature for this call only. `0.0` gives the most deterministic output.
    pub temperature: Option<f32>,
    /// When `true`, request JSON-constrained output from the model (e.g. Ollama `format: "json"`).
    pub format_json: bool,
    /// Optional output-token cap override for this call.
    pub max_output_tokens: Option<u32>,
}

impl LlmCallOptions {
    /// Options appropriate for classification calls: near-deterministic, JSON output.
    ///
    /// A small non-zero temperature (0.1) avoids the greedy-decoding collapse that causes
    /// small models to default to `chat` for short or single-word inputs when temperature is
    /// exactly 0.  It still produces highly consistent output while allowing the model to
    /// escape local minima on ambiguous tokens.
    pub fn for_classification() -> Self {
        Self {
            temperature: Some(0.1),
            format_json: true,
            max_output_tokens: Some(24),
        }
    }
}

/// Streaming LLM: user message in, token stream out.
#[async_trait]
pub trait LlmStream: Send + Sync {
    /// Run one turn: send user text, stream response tokens.
    ///
    /// - `system_prompt_override`: when `Some`, replaces the default system prompt for this call.
    /// - `call_options`: when `Some`, applies per-call overrides (temperature, format).
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<
        Box<dyn Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    >;
}

/// Streaming TTS: consume text chunks, produce audio (or play).
#[async_trait]
pub trait TtsSink: Send + Sync {
    /// Push text chunk for synthesis; may stream audio out internally.
    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// Flush remaining synthesis.
    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// When set, flush should send audio to this device (e.g. pod) instead of local playback.
    fn set_egress_device(&mut self, _device_id: Option<String>) {}

    /// Request immediate stop of any ongoing/queued playback.
    fn request_stop_playback(&mut self) {}

    /// Play raw PCM16 (16kHz mono) bytes directly when supported (e.g. pod egress).
    /// Returns true when playback was accepted, false when unsupported/unavailable.
    async fn play_pcm_bytes(
        &mut self,
        _pcm: &[u8],
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(false)
    }
}
