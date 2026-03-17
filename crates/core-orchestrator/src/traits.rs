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

/// Streaming LLM: user message in, token stream out.
#[async_trait]
pub trait LlmStream: Send + Sync {
    /// Run one turn: send user text, stream response tokens.
    /// When `system_prompt_override` is `Some`, use it instead of the default system prompt for this call only.
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
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
