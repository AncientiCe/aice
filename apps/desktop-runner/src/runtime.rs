//! Desktop runtime: one-turn and continuous-loop flow with wake-word gating.

use chrono::Local;
use core_audio::{AudioCapture, CaptureError, SAMPLE_RATE};
use core_config::Config;
use core_observability::{
    record_assistant_skill, record_computer_skill, record_distance_skill, record_intent_classifier,
    record_intent_routed, record_media_execute, record_media_execute_duration, record_media_skill,
    record_memory_fact_recall, record_memory_fact_recall_duration, record_memory_fact_store,
    record_memory_fact_store_duration, record_memory_save, record_memory_save_duration,
    record_memory_save_error, record_memory_skill, record_policy_denied, record_smart_home_execute,
    record_smart_home_execute_duration, record_smart_home_skill, record_time_skill,
    record_weather_skill,
};
use core_orchestrator::{
    parse_need_search, IntentClassifier, IntentDecision, LlmStream, SttStream, TtsSink,
};
use core_policy::{skill_id_and_risk, ActionRequest, PolicyDecision, PolicyEngine};
use core_search::ExternalSearch;
use core_skills::{
    AssistantSkill, ComputerSkill, DistanceResult, DistanceSkill, MediaSkill, MemorySkill,
    ResolvedLocation, SmartHomeSkill, TimeResult, TimeSkill, WeatherResult, WeatherSkill,
};
use core_vad::WakeWordGate;
use std::fs;
use std::future::Future;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::{info, instrument, warn};

use crate::memory::MemoryStore;

/// Result of one runtime turn (high-level).
#[derive(Debug, Eq, PartialEq)]
pub enum RuntimeTurnOutcome {
    /// Turn completed (response spoken).
    Complete,
    /// Wake word gate: not listening.
    GateClosed,
    /// No user input (empty transcript).
    EmptyInput,
    /// User interrupted (barge-in).
    Interrupted,
}

enum LocalCommand {
    Speak(String),
    PlayChocobo,
}

#[derive(Clone, Debug)]
struct ParsedMediaCommand {
    action: String,
    target: Option<String>,
}

/// Aggregate stats from continuous runtime execution.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct RuntimeLoopStats {
    pub turns_completed: usize,
    pub turns_interrupted: usize,
    pub turns_empty: usize,
    pub wake_activations: usize,
}

/// Optional intent classifier + skills context for a turn (avoids passing many args).
pub struct SkillRunContext<'a> {
    pub intent_classifier: Option<&'a dyn IntentClassifier>,
    pub weather_skill: Option<&'a dyn WeatherSkill>,
    pub time_skill: Option<&'a dyn TimeSkill>,
    pub distance_skill: Option<&'a dyn DistanceSkill>,
    pub smart_home_skill: Option<&'a dyn SmartHomeSkill>,
    pub assistant_skill: Option<&'a dyn AssistantSkill>,
    pub media_skill: Option<&'a dyn MediaSkill>,
    pub memory_skill: Option<&'a dyn MemorySkill>,
    pub computer_skill: Option<&'a dyn ComputerSkill>,
    pub resolved_location: Option<&'a ResolvedLocation>,
    /// Shared memory store for conversation history and profile; used for chat history and post-turn save.
    pub memory: Option<Arc<tokio::sync::Mutex<MemoryStore>>>,
    /// Optional autonomy policy: when set, all side-effecting skill executions are gated by allow_action and emergency_stop.
    pub policy: Option<&'a dyn PolicyEngine>,
}

/// Options for continuous desktop runtime execution.
pub struct ContinuousRunOptions<'a, E> {
    pub search: Option<&'a E>,
    pub cancel_rx: broadcast::Receiver<()>,
    pub max_turns: Option<usize>,
    pub skills: SkillRunContext<'a>,
}

/// User confirmation callback: (local_answer, query) -> true = search, false = use local only.
pub type UserConfirmFn =
    Arc<dyn Fn(String, String) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Desktop runtime: composes config, wake-word gate, and one-turn flow.
pub struct DesktopRuntime {
    config: Config,
    wake_gate: WakeWordGate,
    last_assistant_utterance: Option<(String, Instant)>,
    /// When NeedsSearch is detected, call this to get yes/no. None = treat as No.
    user_confirm: Option<UserConfirmFn>,
}

impl DesktopRuntime {
    pub fn new(config: Config) -> Self {
        let wake_gate = WakeWordGate::new(config.wake_word.clone());
        Self {
            config,
            wake_gate,
            last_assistant_utterance: None,
            user_confirm: None,
        }
    }

    pub fn with_user_confirm(mut self, f: UserConfirmFn) -> Self {
        self.user_confirm = Some(f);
        self
    }

    /// Call when wake word is detected (or user taps); enables listening during cooldown.
    pub fn activate_wake(&mut self) {
        self.wake_gate.activate(Instant::now());
    }

    /// Call when wake word is detected at a specific instant.
    pub fn activate_wake_at(&mut self, now: Instant) {
        self.wake_gate.activate(now);
    }

    /// Run one turn: STT flush -> LLM (collect) -> if NeedsSearch ask user -> speak (local or search result).
    /// If cancel_rx receives, interrupt during TTS.
    #[instrument(skip_all)]
    pub async fn run_one_turn<S, L, T, E>(
        &mut self,
        stt: &mut S,
        llm: &L,
        tts: &mut T,
        search: Option<&E>,
        mut cancel_rx: broadcast::Receiver<()>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        S: SttStream,
        L: LlmStream,
        T: TtsSink,
        E: ExternalSearch,
    {
        let now = Instant::now();
        if self.wake_gate.is_enabled() && !self.wake_gate.should_listen(now) {
            return Ok(RuntimeTurnOutcome::GateClosed);
        }

        let user_text = stt.flush().await?;
        let skills = SkillRunContext {
            intent_classifier: None,
            weather_skill: None,
            time_skill: None,
            distance_skill: None,
            smart_home_skill: None,
            assistant_skill: None,
            media_skill: None,
            memory_skill: None,
            computer_skill: None,
            resolved_location: None,
            memory: None,
            policy: None,
        };
        self.run_turn_from_user_text(user_text, llm, tts, search, &mut cancel_rx, &skills)
            .await
    }

    /// Run one turn with optional intent classifier and weather skill (for tests or custom wiring).
    #[instrument(skip_all)]
    #[allow(clippy::too_many_arguments)]
    pub async fn run_one_turn_with_skills<S, L, T, E>(
        &mut self,
        stt: &mut S,
        llm: &L,
        tts: &mut T,
        search: Option<&E>,
        mut cancel_rx: broadcast::Receiver<()>,
        skills: &SkillRunContext<'_>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        S: SttStream,
        L: LlmStream,
        T: TtsSink,
        E: ExternalSearch,
    {
        let now = Instant::now();
        if self.wake_gate.is_enabled() && !self.wake_gate.should_listen(now) {
            return Ok(RuntimeTurnOutcome::GateClosed);
        }

        let user_text = stt.flush().await?;
        self.run_turn_from_user_text(user_text, llm, tts, search, &mut cancel_rx, skills)
            .await
    }

    /// Continuously capture and process turns until `max_turns` is reached (or forever when `None`).
    /// Turn boundaries are approximated by the configured turn window duration.
    pub async fn run_continuous<C, S, L, T, E>(
        &mut self,
        capture: &mut C,
        stt: &mut S,
        llm: &L,
        tts: &mut T,
        mut options: ContinuousRunOptions<'_, E>,
    ) -> Result<RuntimeLoopStats, Box<dyn std::error::Error + Send + Sync>>
    where
        C: AudioCapture,
        S: SttStream,
        L: LlmStream,
        T: TtsSink,
        E: ExternalSearch,
    {
        let mut stats = RuntimeLoopStats::default();
        let mut buffered_samples: usize = 0;
        let target_samples =
            ((SAMPLE_RATE as u64 * self.config.audio.turn_window_ms) / 1000) as usize;
        let target_samples = target_samples.max(1);
        let chunk_timeout_ms = self.config.audio.chunk_timeout_ms.max(1);
        let timeout = Duration::from_millis(chunk_timeout_ms);
        let speech_end_silence_ms = self
            .config
            .audio
            .speech_end_silence_ms
            .max(chunk_timeout_ms);
        let speech_rms_threshold = self.config.audio.speech_rms_threshold.max(0.0);
        let idle_sleep = Duration::from_millis(self.config.audio.idle_sleep_ms);
        let mut consecutive_timeouts: u64 = 0;
        let mut silence_after_voice_ms: u64 = 0;
        let mut observed_voice = false;

        loop {
            let mut flush_partial_on_timeout = false;
            match capture.read_chunk(timeout) {
                Ok(chunk) => {
                    consecutive_timeouts = 0;
                    if !chunk.is_empty() {
                        let chunk_ms = Self::chunk_duration_ms(chunk.len());
                        if Self::is_voiced_chunk(&chunk, speech_rms_threshold) {
                            observed_voice = true;
                            silence_after_voice_ms = 0;
                            buffered_samples += chunk.len();
                            stt.push_audio(&chunk).await?;
                        } else if observed_voice {
                            silence_after_voice_ms =
                                silence_after_voice_ms.saturating_add(chunk_ms);
                            if silence_after_voice_ms >= speech_end_silence_ms
                                && buffered_samples > 0
                            {
                                flush_partial_on_timeout = true;
                                silence_after_voice_ms = 0;
                                observed_voice = false;
                            }
                        }
                    }
                }
                Err(CaptureError::Timeout) => {
                    consecutive_timeouts += 1;
                    if buffered_samples > 0 {
                        silence_after_voice_ms =
                            silence_after_voice_ms.saturating_add(chunk_timeout_ms);
                        if silence_after_voice_ms >= speech_end_silence_ms {
                            flush_partial_on_timeout = true;
                            consecutive_timeouts = 0;
                            silence_after_voice_ms = 0;
                            observed_voice = false;
                        }
                    }
                    if consecutive_timeouts.is_multiple_of(20) {
                        warn!(
                            consecutive_timeouts,
                            "no audio received yet (check mic permissions or pod connection/LED)"
                        );
                    }
                    if !flush_partial_on_timeout {
                        tokio::time::sleep(idle_sleep).await;
                        continue;
                    }
                }
                Err(e) => return Err(Box::new(e)),
            }

            if !flush_partial_on_timeout {
                // Do not flush while speech is ongoing; wait for end-of-speech silence.
                if observed_voice {
                    continue;
                }
                if buffered_samples < target_samples {
                    continue;
                }
            }
            buffered_samples = 0;
            silence_after_voice_ms = 0;
            observed_voice = false;

            let transcript = match stt.flush().await {
                Ok(t) => t,
                Err(e) => {
                    if Self::is_console_interrupt_stt_error(e.as_ref()) {
                        info!("stt interrupted by console control event; stopping runtime loop");
                        return Ok(stats);
                    }
                    if Self::is_access_violation_stt_error(e.as_ref()) {
                        warn!(error = %e, "recoverable stt access violation; continuing");
                        continue;
                    }
                    return Err(e);
                }
            };
            let mut user_text = transcript.trim().to_string();
            if user_text.is_empty() {
                stats.turns_empty += 1;
                continue;
            }
            let now = Instant::now();
            if self.is_probable_self_echo(&user_text, now) {
                continue;
            }

            if self.wake_gate.is_enabled() && !self.wake_gate.should_listen(now) {
                if Self::is_gate_bypass_command(&user_text) {
                    match self
                        .run_turn_from_user_text(
                            user_text,
                            llm,
                            tts,
                            options.search,
                            &mut options.cancel_rx,
                            &options.skills,
                        )
                        .await?
                    {
                        RuntimeTurnOutcome::Complete => stats.turns_completed += 1,
                        RuntimeTurnOutcome::Interrupted => stats.turns_interrupted += 1,
                        RuntimeTurnOutcome::EmptyInput => stats.turns_empty += 1,
                        RuntimeTurnOutcome::GateClosed => {}
                    }
                    if let Some(limit) = options.max_turns {
                        if stats.turns_completed + stats.turns_interrupted >= limit {
                            return Ok(stats);
                        }
                    }
                    continue;
                }
                if let Some(stripped) = self.try_activate_from_transcript(&user_text, now) {
                    stats.wake_activations += 1;
                    user_text = stripped;
                    if user_text.trim().is_empty() {
                        continue;
                    }
                } else {
                    continue;
                }
            } else if self.wake_gate.is_enabled() {
                user_text = self.strip_wake_phrase(&user_text);
                if user_text.trim().is_empty() {
                    continue;
                }
            }
            if Self::is_non_action_fragment(&user_text) {
                stats.turns_empty += 1;
                continue;
            }

            // Route TTS to pod when this turn's audio came from a pod.
            tts.set_egress_device(capture.source_device_id());

            match self
                .run_turn_from_user_text(
                    user_text,
                    llm,
                    tts,
                    options.search,
                    &mut options.cancel_rx,
                    &options.skills,
                )
                .await?
            {
                RuntimeTurnOutcome::Complete => stats.turns_completed += 1,
                RuntimeTurnOutcome::Interrupted => stats.turns_interrupted += 1,
                RuntimeTurnOutcome::EmptyInput => stats.turns_empty += 1,
                RuntimeTurnOutcome::GateClosed => {}
            }
            if self.wake_gate.is_enabled() {
                // Strict mode: each new command must include wake phrase again.
                self.wake_gate.deactivate();
            }

            if let Some(limit) = options.max_turns {
                if stats.turns_completed + stats.turns_interrupted >= limit {
                    return Ok(stats);
                }
            }
        }
    }

    async fn run_turn_from_user_text<L, T, E>(
        &mut self,
        user_text: String,
        llm: &L,
        tts: &mut T,
        search: Option<&E>,
        cancel_rx: &mut broadcast::Receiver<()>,
        skills: &SkillRunContext<'_>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        L: LlmStream,
        T: TtsSink,
        E: ExternalSearch,
    {
        let intent_classifier = skills.intent_classifier;
        let weather_skill = skills.weather_skill;
        let time_skill = skills.time_skill;
        let distance_skill = skills.distance_skill;
        let smart_home_skill = skills.smart_home_skill;
        let assistant_skill = skills.assistant_skill;
        let media_skill = skills.media_skill;
        let memory_skill = skills.memory_skill;
        let computer_skill = skills.computer_skill;
        let resolved_location = skills.resolved_location;
        let memory = skills.memory.as_ref();
        if user_text.trim().is_empty() {
            return Ok(RuntimeTurnOutcome::EmptyInput);
        }

        info!(user_text = %user_text.trim(), "turn");

        if let Some(skill) = memory_skill {
            let t0 = Instant::now();
            if let Err(e) = skill.ingest_turn(&user_text).await {
                record_memory_fact_store("error", "turn");
                warn!(error = %e, "memory turn ingestion failed");
            } else {
                record_memory_fact_store("success", "turn");
            }
            record_memory_fact_store_duration("turn", t0.elapsed());
        }

        let lowered = user_text.to_lowercase();
        if Self::wants_stop(&lowered) {
            if let Some(skill) = media_skill {
                let _ = skill.execute(Some("stop"), None).await;
            }
            tts.request_stop_playback();
            return Ok(RuntimeTurnOutcome::Complete);
        }

        if let Some(local_command) = Self::local_command(&user_text) {
            return match local_command {
                LocalCommand::Speak(local_response) => {
                    self.register_assistant_utterance(&local_response, Instant::now());
                    Self::speak_with_cancel(tts, &local_response, cancel_rx).await
                }
                LocalCommand::PlayChocobo => Self::play_chocobo_with_cancel(tts, cancel_rx).await,
            };
        }

        if cancel_rx.try_recv().is_ok() {
            tts.request_stop_playback();
            return Ok(RuntimeTurnOutcome::Interrupted);
        }

        let parsed_media_cmd = Self::parse_media_command(&user_text);

        if let Some(media_cmd) = parsed_media_cmd {
            if let Some(skill) = media_skill {
                let action_label = media_cmd.action.as_str();
                let t0 = Instant::now();
                match skill
                    .execute(Some(action_label), media_cmd.target.as_deref())
                    .await
                {
                    Ok(result) => {
                        record_media_skill("success");
                        record_media_execute("success", action_label);
                        record_media_execute_duration(action_label, t0.elapsed());
                        let spoken = if let Some(np) = result.now_playing.as_deref() {
                            format!("Now Playing - {}", np)
                        } else {
                            result.summary
                        };
                        self.register_assistant_utterance(&spoken, Instant::now());
                        return Self::speak_with_cancel(tts, &spoken, cancel_rx).await;
                    }
                    Err(e) => {
                        record_media_skill("error");
                        record_media_execute("error", action_label);
                        record_media_execute_duration(action_label, t0.elapsed());
                        warn!(error = %e, "direct media command failed");
                        return Self::speak_with_cancel(
                            tts,
                            "I could not control Music.app for that command.",
                            cancel_rx,
                        )
                        .await;
                    }
                }
            }
        }

        // Intent classification: if we have a classifier, use it to decide chat vs skill.
        let decision = if let Some(classifier) = intent_classifier {
            record_intent_classifier();
            match classifier.classify(&user_text).await {
                Ok(d) => {
                    record_intent_routed(match &d {
                        IntentDecision::Chat => "chat",
                        IntentDecision::SkillWeather { .. } => "skill_weather",
                        IntentDecision::SkillTime { .. } => "skill_time",
                        IntentDecision::SkillDistance { .. } => "skill_distance",
                        IntentDecision::SkillSmartHome { .. } => "skill_smart_home",
                        IntentDecision::SkillAssistant { .. } => "skill_assistant",
                        IntentDecision::SkillMedia { .. } => "skill_media",
                        IntentDecision::SkillMemory { .. } => "skill_memory",
                        IntentDecision::SkillComputer { .. } => "skill_computer",
                    });
                    d
                }
                Err(e) => {
                    tracing::warn!(error = %e, "intent classification failed, falling back to chat");
                    record_intent_routed("chat");
                    IntentDecision::Chat
                }
            }
        } else {
            IntentDecision::Chat
        };

        // Policy gate: if emergency stop or action denied, fall through to chat.
        let policy = skills.policy;
        let action_allowed = |decision: &IntentDecision| -> bool {
            let Some(p) = policy else { return true };
            if p.emergency_stop() {
                return false;
            }
            let (intent_name, action_hint) = match decision {
                IntentDecision::SkillWeather { location } => ("skill_weather", location.clone()),
                IntentDecision::SkillTime { location } => ("skill_time", location.clone()),
                IntentDecision::SkillDistance {
                    origin,
                    destination,
                } => {
                    let h = origin.as_ref().or(destination.as_ref()).cloned();
                    ("skill_distance", h)
                }
                IntentDecision::SkillSmartHome { target, action } => (
                    "skill_smart_home",
                    target.clone().or_else(|| action.clone()),
                ),
                IntentDecision::SkillAssistant { kind } => ("skill_assistant", kind.clone()),
                IntentDecision::SkillMedia { action, target } => {
                    ("skill_media", action.clone().or_else(|| target.clone()))
                }
                IntentDecision::SkillMemory { query, .. } => ("skill_memory", query.clone()),
                IntentDecision::SkillComputer { action, target } => {
                    ("skill_computer", action.clone().or_else(|| target.clone()))
                }
                IntentDecision::Chat => return true,
            };
            let (skill_id, risk_tier) = skill_id_and_risk(intent_name);
            let request = ActionRequest {
                skill: skill_id,
                action_hint,
                risk_tier,
            };
            matches!(p.allow_action(&request), PolicyDecision::Allow)
        };

        // Weather skill path: run skill then stream LLM answer from payload.
        if let IntentDecision::SkillWeather { location } = &decision {
            if let Some(skill) = weather_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_weather");
                    tracing::warn!("policy denied weather skill, falling back to chat");
                } else {
                    let location_override = location.as_deref();
                    match skill.execute(location_override, resolved_location).await {
                        Ok(weather) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "weather", "skill_executed");
                            record_weather_skill("success");
                            let prompt = Self::weather_answer_prompt(&user_text, &weather);
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_weather_skill("error");
                            tracing::warn!(error = %e, "weather skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Time skill path: optional location, default = current.
        if let IntentDecision::SkillTime { location } = &decision {
            if let Some(skill) = time_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_time");
                    tracing::warn!("policy denied time skill, falling back to chat");
                } else {
                    let location_override = location.as_deref();
                    match skill.execute(location_override, resolved_location).await {
                        Ok(time_result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "time", "skill_executed");
                            record_time_skill("success");
                            let prompt = Self::time_answer_prompt(&user_text, &time_result);
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_time_skill("error");
                            tracing::warn!(error = %e, "time skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Distance skill path: origin/destination; missing one = current location.
        if let IntentDecision::SkillDistance {
            origin,
            destination,
        } = &decision
        {
            if let Some(skill) = distance_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_distance");
                    tracing::warn!("policy denied distance skill, falling back to chat");
                } else {
                    match skill
                        .execute(origin.as_deref(), destination.as_deref(), resolved_location)
                        .await
                    {
                        Ok(dist_result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "distance", "skill_executed");
                            record_distance_skill("success");
                            let prompt = Self::distance_answer_prompt(&user_text, &dist_result);
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_distance_skill("error");
                            tracing::warn!(error = %e, "distance skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Smart home skill path.
        if let IntentDecision::SkillSmartHome { target, action } = &decision {
            if let Some(skill) = smart_home_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_smart_home");
                    tracing::warn!("policy denied smart home skill, falling back to chat");
                } else {
                    let action_label = action.as_deref().unwrap_or("status");
                    let t0 = Instant::now();
                    match skill.execute(target.as_deref(), action.as_deref()).await {
                        Ok(result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "smart_home", "skill_executed");
                            record_smart_home_skill("success");
                            record_smart_home_execute("success", action_label);
                            record_smart_home_execute_duration(action_label, t0.elapsed());
                            let prompt =
                                Self::skill_answer_prompt(&user_text, &result.to_prompt_context());
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_smart_home_skill("error");
                            record_smart_home_execute("error", action_label);
                            record_smart_home_execute_duration(action_label, t0.elapsed());
                            tracing::warn!(error = %e, "smart home skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Assistant skill path.
        if let IntentDecision::SkillAssistant { kind } = &decision {
            if let Some(skill) = assistant_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_assistant");
                    tracing::warn!("policy denied assistant skill, falling back to chat");
                } else {
                    match skill.execute(kind.as_deref()).await {
                        Ok(result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "assistant", "skill_executed");
                            record_assistant_skill("success");
                            let prompt =
                                Self::skill_answer_prompt(&user_text, &result.to_prompt_context());
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_assistant_skill("error");
                            tracing::warn!(error = %e, "assistant skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Media skill path.
        if let IntentDecision::SkillMedia { action, target } = &decision {
            if let Some(skill) = media_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_media");
                    tracing::warn!("policy denied media skill, falling back to chat");
                } else {
                    let action_label = action.as_deref().unwrap_or("status");
                    let t0 = Instant::now();
                    match skill.execute(action.as_deref(), target.as_deref()).await {
                        Ok(result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "media", "skill_executed");
                            record_media_skill("success");
                            record_media_execute("success", action_label);
                            record_media_execute_duration(action_label, t0.elapsed());
                            // Media responses should be deterministic and concise.
                            let spoken = if let Some(np) = result.now_playing.as_deref() {
                                format!("Now Playing - {}", np)
                            } else {
                                result.summary.clone()
                            };
                            self.register_assistant_utterance(&spoken, Instant::now());
                            return Self::speak_with_cancel(tts, &spoken, cancel_rx).await;
                        }
                        Err(e) => {
                            record_media_skill("error");
                            record_media_execute("error", action_label);
                            record_media_execute_duration(action_label, t0.elapsed());
                            tracing::warn!(error = %e, "media skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Memory skill path.
        if let IntentDecision::SkillMemory { query, store } = &decision {
            if let Some(skill) = memory_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_memory");
                    tracing::warn!("policy denied memory skill, falling back to chat");
                } else {
                    let t0 = Instant::now();
                    match skill.execute(query.as_deref(), *store).await {
                        Ok(result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "memory", "skill_executed");
                            record_memory_skill("success");
                            record_memory_fact_recall("success");
                            record_memory_fact_recall_duration(t0.elapsed());
                            let prompt =
                                Self::skill_answer_prompt(&user_text, &result.to_prompt_context());
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_memory_skill("error");
                            record_memory_fact_recall("error");
                            record_memory_fact_recall_duration(t0.elapsed());
                            tracing::warn!(error = %e, "memory skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Computer skill path.
        if let IntentDecision::SkillComputer { action, target } = &decision {
            if let Some(skill) = computer_skill {
                if !action_allowed(&decision) {
                    record_policy_denied("skill_computer");
                    tracing::warn!("policy denied computer skill, falling back to chat");
                } else {
                    match skill.execute(action.as_deref(), target.as_deref()).await {
                        Ok(result) => {
                            if let Some(p) = policy {
                                p.record_action();
                            }
                            info!(skill = "computer", "skill_executed");
                            record_computer_skill("success");
                            let prompt =
                                Self::skill_answer_prompt(&user_text, &result.to_prompt_context());
                            return Self::stream_llm_to_tts(llm, tts, cancel_rx, &prompt, None)
                                .await;
                        }
                        Err(e) => {
                            record_computer_skill("error");
                            tracing::warn!(error = %e, "computer skill failed, falling back to chat");
                        }
                    }
                }
            }
        }

        // Chat path: stream LLM response with optional conversation history.
        let history: Vec<(String, String)> = if let Some(mem) = memory {
            let guard = mem.lock().await;
            guard.history()
        } else {
            vec![]
        };
        let mut stream = llm.chat_stream(&user_text, &history, None).await?;
        use futures::StreamExt;
        let mut full_response = String::new();
        loop {
            tokio::select! {
                token = stream.next() => {
                    let Some(token) = token else { break };
                    if !token.is_empty() {
                        full_response.push_str(&token);
                        tts.push_text(&token).await?;
                    }
                }
                _ = cancel_rx.recv() => {
                    tts.request_stop_playback();
                    return Ok(RuntimeTurnOutcome::Interrupted);
                }
            }
        }
        tts.flush().await?;
        self.register_assistant_utterance(&full_response, Instant::now());
        if let Some((local_answer, query)) = parse_need_search(&full_response) {
            let do_search = if let Some(ref confirm) = self.user_confirm {
                confirm(local_answer.clone(), query.clone()).await
            } else {
                false
            };
            if do_search {
                tts.request_stop_playback();
                let to_speak = if let Some(s) = search {
                    s.execute(&query).await.unwrap_or_else(|e| e.to_string())
                } else {
                    local_answer
                };
                self.register_assistant_utterance(&to_speak, Instant::now());
                return Self::speak_with_cancel(tts, &to_speak, cancel_rx).await;
            }
        }
        // Persist turn to memory and optionally save to disk.
        if let Some(mem) = memory {
            let mut guard = mem.lock().await;
            guard.push_turn(user_text.trim(), full_response.trim());
            if self.config.memory.enabled && self.config.memory.autosave {
                let path = Path::new(&self.config.memory.path);
                let t0 = Instant::now();
                match guard.save(path) {
                    Ok(()) => {
                        record_memory_save_duration(t0.elapsed());
                        record_memory_save();
                    }
                    Err(e) => {
                        record_memory_save_error("io");
                        tracing::warn!(error = %e, path = %self.config.memory.path, "memory save failed");
                    }
                }
            }
        }
        Ok(RuntimeTurnOutcome::Complete)
    }

    fn weather_answer_prompt(user_text: &str, weather: &WeatherResult) -> String {
        let context = weather.to_prompt_context();
        let user_lower = user_text.to_lowercase();
        let asks_tomorrow = user_lower.contains("tomorrow") || user_lower.contains("tomorrow's");
        let only_current_note = if asks_tomorrow {
            format!(
                "\n\nThis data is CURRENT conditions only (no forecast for tomorrow). \
                If the user asked about tomorrow, say you only have current conditions for {} \
                and give the current conditions. Do not invent percentages, probabilities, or future forecast.",
                weather.location_display
            )
        } else {
            "\n\nUse only the data provided. Do not invent percentages or probabilities."
                .to_string()
        };
        format!(
            "The user asked: \"{}\"\n\nHere is the weather data: {}.{}\n\nReply in 1-2 short sentences for voice, using only this data.",
            user_text.trim(),
            context,
            only_current_note
        )
    }

    fn time_answer_prompt(user_text: &str, time_result: &TimeResult) -> String {
        let context = time_result.to_prompt_context();
        format!(
            "The user asked: \"{}\"\n\nHere is the time data: {}.\n\nReply in 1-2 short sentences for voice, using this data.",
            user_text.trim(),
            context
        )
    }

    fn distance_answer_prompt(user_text: &str, dist_result: &DistanceResult) -> String {
        let context = dist_result.to_prompt_context();
        format!(
            "The user asked: \"{}\"\n\nHere is the distance data: {}.\n\nReply in 1-2 short sentences for voice, using this data.",
            user_text.trim(),
            context
        )
    }

    /// Generic prompt for skill results (smart home, assistant, media, memory, computer).
    fn skill_answer_prompt(user_text: &str, context: &str) -> String {
        format!(
            "The user asked: \"{}\"\n\nHere is the data: {}.\n\nReply in 1-2 short sentences for voice, using this data.",
            user_text.trim(),
            context
        )
    }

    /// Stream LLM response for a given prompt to TTS; supports optional system prompt override.
    async fn stream_llm_to_tts<L, T>(
        llm: &L,
        tts: &mut T,
        cancel_rx: &mut broadcast::Receiver<()>,
        user_prompt: &str,
        system_prompt_override: Option<&str>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>>
    where
        L: LlmStream,
        T: TtsSink,
    {
        let mut stream = llm
            .chat_stream(user_prompt, &[], system_prompt_override)
            .await?;
        use futures::StreamExt;
        loop {
            tokio::select! {
                token = stream.next() => {
                    let Some(token) = token else { break };
                    if !token.is_empty() {
                        tts.push_text(&token).await?;
                    }
                }
                _ = cancel_rx.recv() => {
                    tts.request_stop_playback();
                    return Ok(RuntimeTurnOutcome::Interrupted);
                }
            }
        }
        tts.flush().await?;
        Ok(RuntimeTurnOutcome::Complete)
    }

    /// Push text to TTS in chunks; if cancel_rx fires, stop and return Interrupted.
    async fn speak_with_cancel<T: TtsSink>(
        tts: &mut T,
        text: &str,
        cancel_rx: &mut broadcast::Receiver<()>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>> {
        const CHUNK: usize = 20;
        let mut i = 0;
        let chars: Vec<char> = text.chars().collect();
        while i < chars.len() {
            let end = (i + CHUNK).min(chars.len());
            let chunk: String = chars[i..end].iter().collect();
            tts.push_text(&chunk).await?;
            i = end;
            if let Ok(()) = cancel_rx.try_recv() {
                tts.request_stop_playback();
                return Ok(RuntimeTurnOutcome::Interrupted);
            }
        }
        tts.flush().await?;
        Ok(RuntimeTurnOutcome::Complete)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn wake_gate(&self) -> &WakeWordGate {
        &self.wake_gate
    }

    fn strip_wake_phrase(&self, text: &str) -> String {
        let lower = text.to_lowercase();
        for phrase in self.wake_gate.phrases() {
            let p = phrase.to_lowercase();
            if let Some(idx) = lower.find(&p) {
                let end = idx + p.len();
                let stripped = text[end..]
                    .trim_start_matches(|c: char| c == ',' || c == ':' || c.is_whitespace())
                    .to_string();
                return stripped;
            }
        }
        text.to_string()
    }

    fn try_activate_from_transcript(&mut self, text: &str, now: Instant) -> Option<String> {
        let lower = text.to_lowercase();
        for phrase in self.wake_gate.phrases() {
            let p = phrase.to_lowercase();
            if lower.contains(&p) {
                self.activate_wake_at(now);
                return Some(self.strip_wake_phrase(text));
            }
        }
        None
    }

    /// True if the user is asking to stop ongoing sound/music/speech (voice: "Computer stop", etc.).
    fn wants_stop(lowered: &str) -> bool {
        let t = lowered.trim();
        if t == "stop"
            || t == "pause"
            || t == "quiet"
            || t == "mute"
            || t == "enough"
            || t == "cancel"
        {
            return true;
        }
        lowered.contains("computer stop")
            || lowered.contains("computer pause")
            || lowered.contains("computer, stop")
            || lowered.contains("computer, pause")
            || lowered.contains("hey computer stop")
            || lowered.contains("hey computer pause")
            || lowered.contains("ok computer stop")
            || lowered.contains("ok computer pause")
            || lowered.contains("stop talking")
            || lowered.contains("be quiet")
            || lowered.contains("shut up")
            || lowered.contains("stop the music")
            || lowered.contains("stop music")
            || lowered.contains("stop playing")
            || lowered.contains("stop sound")
            || lowered.contains("stop the sound")
            || lowered.contains("stop it")
            || lowered.contains("stop that")
            || lowered.contains("that's enough")
            || lowered.contains("thats enough")
            || lowered.contains("stop playback")
            || lowered.contains("pause playback")
            || lowered.contains("pause music")
    }

    fn local_command(user_text: &str) -> Option<LocalCommand> {
        let lower = user_text.to_lowercase();
        let wants_play = lower.contains("play")
            || lower.contains("start")
            || lower.contains("run")
            || lower.contains("music")
            || lower.contains("song");
        let mentions_chocobo = lower.contains("chocobo")
            || lower.contains("choco bo")
            || lower.contains("choco-bow")
            || lower.contains("choco bow");
        if wants_play && mentions_chocobo {
            return Some(LocalCommand::PlayChocobo);
        }
        if lower.contains("what time is it")
            || lower.contains("tell me the time")
            || (lower.contains("time") && lower.contains("now"))
        {
            let now = Local::now();
            return Some(LocalCommand::Speak(format!(
                "The current time is {}.",
                now.format("%H:%M")
            )));
        }
        if lower.contains("what date is it")
            || lower.contains("today's date")
            || lower.contains("today date")
        {
            let now = Local::now();
            return Some(LocalCommand::Speak(format!(
                "Today's date is {}.",
                now.format("%Y-%m-%d")
            )));
        }
        None
    }

    fn parse_media_command(user_text: &str) -> Option<ParsedMediaCommand> {
        let text = Self::normalize_stt_media_variants(Self::strip_polite_prefix(
            Self::normalize_voice_command_text(user_text),
        ));
        if text.is_empty() {
            return None;
        }
        let lower = text;
        if let Some(stripped) = lower.strip_prefix("play ") {
            let target = stripped.trim().trim_matches('.').to_string();
            if !target.is_empty() {
                return Some(ParsedMediaCommand {
                    action: "play".to_string(),
                    target: Some(target),
                });
            }
        }
        if matches!(
            lower.as_str(),
            "pause"
                | "stop"
                | "next"
                | "previous"
                | "resume"
                | "shuffle"
                | "shuffle on"
                | "shuffle off"
        ) {
            let action = match lower.as_str() {
                "shuffle" => "shuffle_on".to_string(),
                "shuffle on" => "shuffle_on".to_string(),
                "shuffle off" => "shuffle_off".to_string(),
                _ => lower,
            };
            return Some(ParsedMediaCommand {
                action,
                target: None,
            });
        }
        if lower.contains("turn on shuffle")
            || lower.contains("turn shuffle on")
            || lower.contains("enable shuffle")
        {
            return Some(ParsedMediaCommand {
                action: "shuffle_on".to_string(),
                target: None,
            });
        }
        if lower.contains("turn off shuffle")
            || lower.contains("turn shuffle off")
            || lower.contains("disable shuffle")
        {
            return Some(ParsedMediaCommand {
                action: "shuffle_off".to_string(),
                target: None,
            });
        }
        if lower.contains("pause music") || lower.contains("stop music") {
            return Some(ParsedMediaCommand {
                action: "stop".to_string(),
                target: None,
            });
        }
        None
    }

    fn normalize_stt_media_variants(user_text: &str) -> String {
        // Keep deterministic normalization minimal; semantic repair belongs to LLM normalization.
        Self::normalize_text(user_text)
    }

    fn strip_polite_prefix(user_text: &str) -> &str {
        let mut text = user_text.trim();
        loop {
            let lower = text.to_lowercase();
            let next = if lower.starts_with("please ") {
                Some(&text[7..])
            } else if lower.starts_with("can you ") {
                Some(&text[8..])
            } else if lower.starts_with("could you ") {
                Some(&text[10..])
            } else {
                None
            };
            let Some(next_text) = next else {
                break;
            };
            text = next_text.trim_start();
        }
        text
    }

    fn is_non_action_fragment(user_text: &str) -> bool {
        let normalized = Self::normalize_text(user_text);
        if normalized.is_empty() {
            return true;
        }
        if Self::wants_stop(&normalized) || Self::parse_media_command(&normalized).is_some() {
            return false;
        }
        matches!(
            normalized.as_str(),
            "please"
                | "thanks"
                | "thank you"
                | "ok"
                | "okay"
                | "yes"
                | "no"
                | "sure"
                | "uh"
                | "um"
                | "hmm"
                | "huh"
                | "sorry"
        )
    }

    fn is_gate_bypass_command(user_text: &str) -> bool {
        let lowered = user_text.to_lowercase();
        Self::wants_stop(&lowered)
    }

    fn register_assistant_utterance(&mut self, text: &str, now: Instant) {
        let normalized = Self::normalize_text(text);
        if normalized.len() < 8 {
            return;
        }
        self.last_assistant_utterance = Some((normalized, now));
    }

    fn is_probable_self_echo(&self, user_text: &str, now: Instant) -> bool {
        let lower = user_text.to_lowercase();
        if Self::wants_stop(&lower) {
            return false;
        }
        let has_wake = self
            .wake_gate
            .phrases()
            .iter()
            .any(|p| lower.contains(&p.to_lowercase()));
        if has_wake {
            return false;
        }
        let Some((assistant_text, ts)) = &self.last_assistant_utterance else {
            return false;
        };
        if now.saturating_duration_since(*ts) > Duration::from_secs(8) {
            return false;
        }
        let user_norm = Self::normalize_text(user_text);
        if user_norm.len() < 8 {
            return false;
        }
        user_norm == *assistant_text
            || user_norm.contains(assistant_text)
            || assistant_text.contains(&user_norm)
    }

    fn normalize_text(text: &str) -> String {
        text.to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c.is_whitespace() {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn chunk_duration_ms(sample_count: usize) -> u64 {
        ((sample_count as u64 * 1000) / SAMPLE_RATE as u64).max(1)
    }

    fn is_voiced_chunk(chunk: &[i16], threshold: f32) -> bool {
        if chunk.is_empty() {
            return false;
        }
        let sum_sq = chunk
            .iter()
            .map(|s| {
                let v = *s as f64 / i16::MAX as f64;
                v * v
            })
            .sum::<f64>();
        let rms = (sum_sq / chunk.len() as f64).sqrt() as f32;
        rms >= threshold
    }

    fn normalize_voice_command_text(user_text: &str) -> &str {
        let text = user_text.trim();
        let lower = text.to_lowercase();
        for prefix in [
            "computer,",
            "computer",
            "hey computer,",
            "hey computer",
            "ok computer,",
            "ok computer",
        ] {
            if lower.starts_with(prefix) {
                return text[prefix.len()..]
                    .trim_start_matches(|c: char| c == ',' || c == ':' || c.is_whitespace());
            }
        }
        text
    }

    fn is_console_interrupt_stt_error(err: &(dyn std::error::Error + Send + Sync)) -> bool {
        let msg = err.to_string().to_ascii_lowercase();
        msg.contains("whisper-cli interrupted by console control event")
            || msg.contains("0xc000013a")
    }

    fn is_access_violation_stt_error(err: &(dyn std::error::Error + Send + Sync)) -> bool {
        let msg = err.to_string().to_ascii_lowercase();
        msg.contains("whisper-cli access violation (0xc0000005)") || msg.contains("0xc0000005")
    }

    async fn play_chocobo_with_cancel<T: TtsSink>(
        tts: &mut T,
        cancel_rx: &mut broadcast::Receiver<()>,
    ) -> Result<RuntimeTurnOutcome, Box<dyn std::error::Error + Send + Sync>> {
        if cancel_rx.try_recv().is_ok() {
            tts.request_stop_playback();
            return Ok(RuntimeTurnOutcome::Interrupted);
        }
        let pcm = Self::load_chocobo_pcm()?;
        if cancel_rx.try_recv().is_ok() {
            tts.request_stop_playback();
            return Ok(RuntimeTurnOutcome::Interrupted);
        }
        let played = tts.play_pcm_bytes(&pcm).await?;
        if played {
            Ok(RuntimeTurnOutcome::Complete)
        } else {
            Self::speak_with_cancel(
                tts,
                "I can play chocobo on the pod when pod audio output is active.",
                cancel_rx,
            )
            .await
        }
    }

    fn load_chocobo_pcm() -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let path = Self::chocobo_path();
        let source = fs::read_to_string(&path).map_err(|e| {
            IoError::new(
                ErrorKind::NotFound,
                format!("failed to read chocobo source at {}: {e}", path.display()),
            )
        })?;
        let mut out = Vec::new();
        for token in source
            .split(|c: char| c == ',' || c == '{' || c == '}' || c == ';' || c.is_whitespace())
        {
            let t = token.trim();
            if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                if hex.is_empty() || hex.len() > 2 {
                    continue;
                }
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                }
            }
        }
        if out.is_empty() {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                format!("no PCM bytes found in {}", path.display()),
            )
            .into());
        }
        Ok(out)
    }

    fn chocobo_path() -> PathBuf {
        if let Ok(p) = std::env::var("AICE_CHOCOBO_C_PATH") {
            let candidate = PathBuf::from(p);
            if candidate.exists() {
                return candidate;
            }
        }
        let default = Path::new("Examples").join("chocobo.c");
        default
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopRuntime;

    #[test]
    fn detects_console_interrupt_stt_error_message() {
        let err = std::io::Error::other("Whisper-cli interrupted by console control event");
        assert!(DesktopRuntime::is_console_interrupt_stt_error(&err));
    }

    #[test]
    fn ignores_non_interrupt_stt_error_message() {
        let err = std::io::Error::other("whisper-cli exited with status 1");
        assert!(!DesktopRuntime::is_console_interrupt_stt_error(&err));
    }

    #[test]
    fn detects_recoverable_stt_access_violation_message() {
        let err = std::io::Error::other("whisper-cli access violation (0xc0000005)");
        assert!(DesktopRuntime::is_access_violation_stt_error(&err));
    }

    #[test]
    fn parse_media_command_accepts_polite_play_prefix() {
        let cmd = DesktopRuntime::parse_media_command("computer please play blinding lights.");
        assert!(cmd.is_some());
        let cmd = cmd.expect("media command");
        assert_eq!(cmd.action, "play");
        assert_eq!(cmd.target.as_deref(), Some("blinding lights"));
    }

    #[test]
    fn parse_media_command_accepts_shuffle_on() {
        let cmd = DesktopRuntime::parse_media_command("computer turn on shuffle");
        assert!(cmd.is_some());
        let cmd = cmd.expect("media command");
        assert_eq!(cmd.action, "shuffle_on");
        assert!(cmd.target.is_none());
    }

    #[test]
    fn parse_media_command_accepts_shuffle_off() {
        let cmd = DesktopRuntime::parse_media_command("computer turn shuffle off");
        assert!(cmd.is_some());
        let cmd = cmd.expect("media command");
        assert_eq!(cmd.action, "shuffle_off");
        assert!(cmd.target.is_none());
    }

    #[test]
    fn filters_non_action_fragment_please() {
        assert!(DesktopRuntime::is_non_action_fragment("please."));
        assert!(!DesktopRuntime::is_non_action_fragment(
            "play blinding lights"
        ));
    }

    #[test]
    fn voiced_chunk_detection_respects_threshold() {
        let silence = vec![0_i16; 320];
        let voice = vec![2_000_i16; 320];
        assert!(!DesktopRuntime::is_voiced_chunk(&silence, 0.008));
        assert!(DesktopRuntime::is_voiced_chunk(&voice, 0.008));
    }

    #[test]
    fn chunk_duration_ms_matches_16k_pipeline() {
        assert_eq!(DesktopRuntime::chunk_duration_ms(320), 20);
    }
}
