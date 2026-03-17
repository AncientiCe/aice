//! Conversation engine: one-turn pipeline STT -> LLM -> TTS with metrics and barge-in.

use crate::traits::{LlmStream, SttStream, TtsSink};
use core_observability::{
    record_cancellation_success, record_error, record_first_audio_latency,
    record_first_token_latency, record_interruption, record_session_start, record_stage_duration,
    Stage,
};
use std::time::Instant;
use tokio::sync::broadcast;

/// Runs the voice pipeline for one turn: STT transcript -> LLM stream -> TTS.
pub struct ConversationEngine;

impl ConversationEngine {
    /// Process one turn: flush STT to get user text, stream LLM response into TTS.
    /// Records session start, stage durations, and errors.
    pub async fn run_turn<S, L, T>(
        stt: &mut S,
        llm: &L,
        tts: &mut T,
        history: &[(String, String)],
    ) -> Result<TurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        S: SttStream,
        L: LlmStream,
        T: TtsSink,
    {
        record_session_start();

        let t0 = Instant::now();
        let user_text = stt
            .flush()
            .await
            .inspect_err(|_| record_error("stt_flush"))?;
        record_stage_duration(Stage::Stt, t0.elapsed());

        if user_text.trim().is_empty() {
            return Ok(TurnOutcome::EmptyInput);
        }

        let t1 = Instant::now();
        let mut stream = llm
            .chat_stream(&user_text, history, None)
            .await
            .inspect_err(|_| record_error("llm_stream"))?;
        record_stage_duration(Stage::Llm, t1.elapsed());

        let t2 = Instant::now();
        use futures::StreamExt;
        let mut first_token_at: Option<Instant> = None;
        let mut first_audio_at: Option<Instant> = None;
        while let Some(token) = stream.next().await {
            if first_token_at.is_none() {
                first_token_at = Some(Instant::now());
                record_first_token_latency(t1.elapsed());
            }
            tts.push_text(&token)
                .await
                .inspect_err(|_| record_error("tts_push"))?;
            if first_audio_at.is_none() {
                let now = Instant::now();
                first_audio_at = Some(now);
                if let Some(t) = first_token_at {
                    record_first_audio_latency(now.duration_since(t));
                }
            }
        }
        tts.flush()
            .await
            .inspect_err(|_| record_error("tts_flush"))?;
        record_stage_duration(Stage::Tts, t2.elapsed());

        Ok(TurnOutcome::Complete)
    }

    /// Run STT and LLM only, returning the full response text (for fallback search detection).
    pub async fn run_turn_collect<S, L>(
        stt: &mut S,
        llm: &L,
        history: &[(String, String)],
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
    where
        S: SttStream,
        L: LlmStream,
    {
        record_session_start();
        let t0 = Instant::now();
        let user_text = stt
            .flush()
            .await
            .inspect_err(|_| record_error("stt_flush"))?;
        record_stage_duration(Stage::Stt, t0.elapsed());
        if user_text.trim().is_empty() {
            return Ok(String::new());
        }
        Self::run_llm_collect(llm, &user_text, history).await
    }

    /// Run LLM only with given user text; returns collected response (for runtime when STT already flushed).
    pub async fn run_llm_collect<L>(
        llm: &L,
        user_text: &str,
        history: &[(String, String)],
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
    where
        L: LlmStream,
    {
        let t1 = Instant::now();
        let mut stream = llm
            .chat_stream(user_text, history, None)
            .await
            .inspect_err(|_| record_error("llm_stream"))?;
        record_stage_duration(Stage::Llm, t1.elapsed());
        use futures::StreamExt;
        let mut out = String::new();
        while let Some(token) = stream.next().await {
            out.push_str(&token);
        }
        Ok(out)
    }

    /// Like `run_turn` but exits early if `cancel_rx` receives (barge-in).
    /// Records voice_interruptions_total and voice_cancellation_success_total when interrupted.
    pub async fn run_turn_with_cancel<S, L, T>(
        stt: &mut S,
        llm: &L,
        tts: &mut T,
        history: &[(String, String)],
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        S: SttStream,
        L: LlmStream,
        T: TtsSink,
    {
        record_session_start();

        let t0 = Instant::now();
        let user_text = stt
            .flush()
            .await
            .inspect_err(|_| record_error("stt_flush"))?;
        record_stage_duration(Stage::Stt, t0.elapsed());

        if user_text.trim().is_empty() {
            return Ok(TurnOutcome::EmptyInput);
        }

        let t1 = Instant::now();
        let mut stream = llm
            .chat_stream(&user_text, history, None)
            .await
            .inspect_err(|_| record_error("llm_stream"))?;
        record_stage_duration(Stage::Llm, t1.elapsed());

        let t2 = Instant::now();
        use futures::StreamExt;
        let mut first_token_at: Option<Instant> = None;
        let mut first_audio_at: Option<Instant> = None;
        loop {
            tokio::select! {
                token = stream.next() => {
                    let Some(token) = token else { break };
                    if first_token_at.is_none() {
                        first_token_at = Some(Instant::now());
                        record_first_token_latency(t1.elapsed());
                    }
                    tts.push_text(&token)
                        .await
                        .inspect_err(|_| record_error("tts_push"))?;
                    if first_audio_at.is_none() {
                        let now = Instant::now();
                        first_audio_at = Some(now);
                        if let Some(t) = first_token_at {
                            record_first_audio_latency(now.duration_since(t));
                        }
                    }
                }
                _ = cancel_rx.recv() => {
                    record_interruption();
                    record_cancellation_success();
                    record_stage_duration(Stage::Tts, t2.elapsed());
                    return Ok(TurnOutcome::Interrupted);
                }
            }
        }
        tts.flush()
            .await
            .inspect_err(|_| record_error("tts_flush"))?;
        record_stage_duration(Stage::Tts, t2.elapsed());
        Ok(TurnOutcome::Complete)
    }
}

/// Result of one conversation turn.
#[derive(Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Complete,
    EmptyInput,
    /// User interrupted (barge-in); TTS/LLM cancelled.
    Interrupted,
    /// Model indicated uncertainty; assistant asks user to confirm web search.
    NeedsSearch {
        local_answer: String,
        query: String,
    },
}
