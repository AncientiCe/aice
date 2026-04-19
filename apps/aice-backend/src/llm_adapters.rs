//! Backend LLM adapters bridging `CradleLlmStream` to skill-specific LLM traits.
//!
//! Each skill in `aice-skills` defines a narrow trait it depends on (e.g.
//! `TranslationLlm`, `ScreenOcrLlm`). The skills crate purposely does not pull
//! in any LLM provider; the backend wires real LLM calls via `CradleLlmStream`.
//!
//! Every adapter holds an `Arc<CradleLlmStream>` so multiple skill instances
//! share the same provider connection (and warm KV cache) at no extra cost.
//!
//! Note: there is no `EmailLlm` adapter on purpose. Email is fully
//! frontend-owned (per-frontend provider), so the backend never sees email
//! content and does not run an LLM-backed triage path.

use std::sync::Arc;

use async_trait::async_trait;
use core_llm::CradleLlmStream;
use core_observability::{record_news_summary_chunk, record_news_summary_duration};
use core_orchestrator::{LlmCallOptions, LlmStream};
use core_skills::{
    MeetingNotesLlm, NewsHeadline, NewsHeadlinesError, NewsSummaryLlm, ScreenOcrLlm, TranslationLlm,
};
use futures_util::StreamExt;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::warn;

fn skill_call_options(
    temperature: f32,
    max_output_tokens: u32,
    format_json: bool,
) -> LlmCallOptions {
    LlmCallOptions {
        temperature: Some(temperature),
        format_json,
        format_json_schema: None,
        max_output_tokens: Some(max_output_tokens),
        num_ctx: None,
    }
}

/// Adapter implementing `TranslationLlm` for `SkillTranslate`.
pub struct TranslationLlmAdapter {
    llm: Arc<CradleLlmStream>,
}

impl TranslationLlmAdapter {
    pub fn new(llm: Arc<CradleLlmStream>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl TranslationLlm for TranslationLlmAdapter {
    async fn complete(&self, system_prompt: &str, user_text: &str) -> Result<String, String> {
        let options = skill_call_options(0.2, 256, false);
        self.llm
            .chat_once(user_text, &[], Some(system_prompt), Some(&options))
            .await
            .map_err(|err| err.to_string())
    }
}

/// Adapter implementing `MeetingNotesLlm` for `SkillMeetingNotes`.
pub struct MeetingNotesLlmAdapter {
    llm: Arc<CradleLlmStream>,
}

impl MeetingNotesLlmAdapter {
    pub fn new(llm: Arc<CradleLlmStream>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl MeetingNotesLlm for MeetingNotesLlmAdapter {
    async fn complete_json(&self, system_prompt: &str, user_text: &str) -> Result<String, String> {
        let options = skill_call_options(0.1, 512, true);
        self.llm
            .chat_once(user_text, &[], Some(system_prompt), Some(&options))
            .await
            .map_err(|err| err.to_string())
    }
}

/// Adapter implementing `ScreenOcrLlm` for `SkillScreenOcr`.
///
/// The frontend captures the screen and runs OCR locally (e.g. Apple Vision,
/// tesseract) before posting `ocr_text` back to the backend; this adapter only
/// runs the LLM that answers `question` against that text. That keeps the
/// "heavy load" (LLM reasoning) on the backend while the OS-specific capture
/// path stays per-frontend.
pub struct ScreenOcrLlmAdapter {
    llm: Arc<CradleLlmStream>,
}

impl ScreenOcrLlmAdapter {
    pub fn new(llm: Arc<CradleLlmStream>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl ScreenOcrLlm for ScreenOcrLlmAdapter {
    async fn answer(&self, question: &str, ocr_text: &str) -> Result<String, String> {
        let system_prompt = "You answer questions about text the user has on screen. \
             Use only the provided OCR text. Be concise (at most 2 short voice-friendly \
             sentences). If the answer is not present, say so plainly.";
        let user_prompt = format!(
            "Question: {}\n\nOn-screen text:\n{}",
            question.trim(),
            ocr_text.trim()
        );
        let options = skill_call_options(0.2, 160, false);
        self.llm
            .chat_once(&user_prompt, &[], Some(system_prompt), Some(&options))
            .await
            .map_err(|err| err.to_string())
    }
}

/// Adapter implementing `NewsSummaryLlm` for opt-in per-headline summary
/// streaming (driven by `core_skills::stream_news_summaries`).
///
/// Each call to `summarize` spawns a task that collects tokens from
/// `CradleLlmStream::chat_stream` and forwards them through an mpsc channel.
/// Per-chunk and total-duration metrics are recorded so streaming behavior is
/// observable.
pub struct NewsSummaryLlmAdapter {
    llm: Arc<CradleLlmStream>,
}

impl NewsSummaryLlmAdapter {
    pub fn new(llm: Arc<CradleLlmStream>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl NewsSummaryLlm for NewsSummaryLlmAdapter {
    async fn summarize(
        &self,
        headline: &NewsHeadline,
    ) -> Result<mpsc::Receiver<String>, NewsHeadlinesError> {
        let system_prompt = "You write one-sentence neutral summaries of news headlines. \
             Reply with at most one short voice-friendly sentence. No preamble.";
        let user_prompt = build_news_summary_prompt(headline);
        let options = skill_call_options(0.3, 80, false);

        let stream_result = self
            .llm
            .chat_stream(&user_prompt, &[], Some(system_prompt), Some(&options))
            .await;

        let mut stream = match stream_result {
            Ok(stream) => stream,
            Err(err) => {
                record_news_summary_chunk("error");
                return Err(NewsHeadlinesError::ProviderUnavailable(err.to_string()));
            }
        };

        let (tx, rx) = mpsc::channel::<String>(32);
        tokio::spawn(async move {
            let started = Instant::now();
            while let Some(token) = stream.next().await {
                if token.is_empty() {
                    continue;
                }
                if tx.send(token).await.is_err() {
                    record_news_summary_chunk("dropped");
                    break;
                }
                record_news_summary_chunk("ok");
            }
            record_news_summary_duration(started.elapsed());
        });
        Ok(rx)
    }
}

fn build_news_summary_prompt(headline: &NewsHeadline) -> String {
    let mut buf = String::with_capacity(headline.title.len() + 64);
    buf.push_str("Summarize this headline in one sentence:\n");
    buf.push_str(&headline.title);
    if let Some(source) = headline.source.as_deref() {
        buf.push_str("\nSource: ");
        buf.push_str(source);
    }
    if let Some(published_at) = headline.published_at.as_deref() {
        buf.push_str("\nPublished: ");
        buf.push_str(published_at);
    }
    if buf.len() > 4096 {
        warn!(
            len = buf.len(),
            "news summary prompt exceeded 4 KiB; truncating"
        );
        buf.truncate(4096);
    }
    buf
}
