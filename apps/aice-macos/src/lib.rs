use async_trait::async_trait;
use core_audio::{AudioCapture, CaptureError, CpalCapture, SAMPLE_RATE};
use core_config::{Config, WakeWordConfig};
use core_observability::{
    record_endpointing_wait_duration, record_error, record_first_audio_latency,
    record_frontend_rpc_duration, record_frontend_skill_duration,
    record_frontend_tts_playback_duration, record_llm_first_token_latency,
    record_llm_stream_tail_duration, record_mic_to_stt_duration, record_session_start,
    record_speech_voiced_duration, record_stage_duration, record_tts_first_audio_latency,
    record_tts_flush_duration, record_turn_time_to_first_audio, Stage,
};
use core_orchestrator::{SttStream, TtsSink};
use core_runtime_protocol::{
    FrontendActivateRequest, FrontendDeactivateRequest, FrontendHeartbeatRequest,
    FrontendSkillIntent, FrontendSkillResultRequest, RuntimeEvent, TurnRequest,
    CURRENT_PROTOCOL_VERSION,
};
use core_skills::{
    AppSwitcherSkill, ComputerSkill, MacOsAppSwitcherSkill, MacOsClockTimerSkill,
    MacOsComputerSkill, MacOsMessagesSkill, MacOsMusicSkill, MacOsNotesShoppingListSkill,
    MacOsReminderSkill, MacOsScreenshotSkill, MacOsVolumeSkill, MediaSkill, MessageSkill,
    ReminderSkill, ScreenshotSkill, ShoppingListSkill, TimerSkill, VolumeSkill,
};
use core_stt::WhisperSttStream;
use core_tts::PiperTtsSink;
use core_vad::WakeWordGate;
use reqwest::Client;
use serde_json::Value;
use std::error::Error;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::info;

pub type DynError = Box<dyn Error + Send + Sync>;

#[async_trait]
pub trait FrontendSkillExecutor: Send + Sync {
    async fn execute(&self, intent: &FrontendSkillIntent) -> Result<String, DynError>;
}

pub struct FrontendClient {
    base_url: String,
    device_id: String,
    client: Client,
    turn_counter: AtomicU64,
}

impl FrontendClient {
    pub fn new(base_url: String) -> Self {
        let device_id = std::env::var("AICE_FRONTEND_DEVICE_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_else(|| "macos-device".to_string());
        Self {
            base_url,
            device_id,
            client: Client::new(),
            turn_counter: AtomicU64::new(1),
        }
    }

    fn next_turn_id(&self, session_id: &str) -> String {
        let suffix = self.turn_counter.fetch_add(1, Ordering::Relaxed);
        format!("turn-{session_id}-{suffix}")
    }

    pub async fn activate_frontend(
        &self,
        session_id: &str,
        supported_frontend_intents: Vec<String>,
    ) -> Result<(), DynError> {
        let request = FrontendActivateRequest {
            device_id: self.device_id.clone(),
            session_id: session_id.to_string(),
            platform: "macos".to_string(),
            frontend_version: env!("CARGO_PKG_VERSION").to_string(),
            supported_frontend_intents,
            expires_in_seconds: Some(120),
            protocol_version: Some(CURRENT_PROTOCOL_VERSION),
        };
        self.client
            .post(format!("{}/v1/frontends/activate", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn heartbeat_frontend(&self, session_id: &str) -> Result<(), DynError> {
        let request = FrontendHeartbeatRequest {
            device_id: self.device_id.clone(),
            session_id: session_id.to_string(),
        };
        self.client
            .post(format!("{}/v1/frontends/heartbeat", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn deactivate_frontend(&self, session_id: &str) -> Result<(), DynError> {
        let request = FrontendDeactivateRequest {
            device_id: self.device_id.clone(),
            session_id: session_id.to_string(),
        };
        self.client
            .post(format!("{}/v1/frontends/deactivate", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn run_turn<E: FrontendSkillExecutor>(
        &self,
        session_id: &str,
        transcript: &str,
        executor: &E,
    ) -> Result<String, DynError> {
        let started_at = Instant::now();
        let turn_id = self.next_turn_id(session_id);
        let response = self
            .client
            .post(format!("{}/v1/turns", self.base_url))
            .json(&TurnRequest {
                session_id: session_id.to_string(),
                device_id: Some(self.device_id.clone()),
                turn_id: Some(turn_id),
                transcript: transcript.to_string(),
                finalize: true,
                context: None,
            })
            .send()
            .await?;
        record_frontend_rpc_duration("turns", started_at.elapsed());

        let body = response.text().await?;
        let events = parse_sse_events(&body);

        let mut token_text = String::new();
        let mut error_text: Option<String> = None;
        for event in &events {
            if let RuntimeEvent::Token { text } = event {
                token_text.push_str(text);
            } else if let RuntimeEvent::Error { message } = event {
                error_text = Some(message.clone());
            }
        }
        if !token_text.trim().is_empty() {
            let first_token_elapsed = started_at.elapsed();
            record_llm_first_token_latency(first_token_elapsed);
            record_llm_stream_tail_duration(Duration::ZERO);
        }

        let intent = events.into_iter().find_map(|event| match event {
            RuntimeEvent::FrontendSkillIntent(intent) => Some(intent),
            _ => None,
        });

        if let Some(intent) = intent {
            let skill_started = Instant::now();
            let result = executor.execute(&intent).await;
            record_frontend_skill_duration(&intent.intent, skill_started.elapsed());

            let request = match result {
                Ok(context) => FrontendSkillResultRequest {
                    status: "success".to_string(),
                    user_text: intent.user_text,
                    structured_result_context: Some(context),
                    error: None,
                },
                Err(error) => FrontendSkillResultRequest {
                    status: "error".to_string(),
                    user_text: intent.user_text,
                    structured_result_context: None,
                    error: Some(error.to_string()),
                },
            };

            let finalize_started = Instant::now();
            let response = self
                .client
                .post(format!(
                    "{}/v1/turns/{}/frontend-skill-result",
                    self.base_url, intent.turn_id
                ))
                .json(&request)
                .send()
                .await?;
            record_frontend_rpc_duration("frontend_skill_result", finalize_started.elapsed());

            let body = response.text().await?;
            let events = parse_sse_events(&body);
            let mut finalized = String::new();
            let mut finalize_error: Option<String> = None;
            for event in events {
                if let RuntimeEvent::Token { text } = event {
                    finalized.push_str(&text);
                } else if let RuntimeEvent::Error { message } = event {
                    finalize_error = Some(message);
                }
            }
            if finalized.trim().is_empty() {
                if let Some(message) = finalize_error {
                    return Ok(message);
                }
            }
            return Ok(finalized);
        }

        if token_text.trim().is_empty() {
            if let Some(message) = error_text {
                return Ok(message);
            }
        }

        Ok(token_text)
    }
}

fn parse_sse_events(payload: &str) -> Vec<RuntimeEvent> {
    payload
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|json| serde_json::from_str::<RuntimeEvent>(json).ok())
        .collect()
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
        .map(|sample| {
            let value = *sample as f64 / i16::MAX as f64;
            value * value
        })
        .sum::<f64>();
    let rms = (sum_sq / chunk.len() as f64).sqrt() as f32;
    rms >= threshold
}

fn normalize_text(text: &str) -> String {
    let normalized = text
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.replace("front end", "frontend")
}

fn token_overlap_ratio(shorter: &str, longer: &str) -> f32 {
    let shorter_tokens = shorter.split_whitespace().collect::<Vec<_>>();
    let longer_tokens = longer.split_whitespace().collect::<Vec<_>>();
    if shorter_tokens.is_empty() || longer_tokens.is_empty() {
        return 0.0;
    }
    let overlap = shorter_tokens
        .iter()
        .filter(|token| longer_tokens.contains(token))
        .count();
    overlap as f32 / shorter_tokens.len() as f32
}

fn is_probable_echo_text(user_norm: &str, assistant_text: &str) -> bool {
    if user_norm == assistant_text
        || user_norm.contains(assistant_text)
        || assistant_text.contains(user_norm)
    {
        return true;
    }
    let (shorter, longer) = if user_norm.len() <= assistant_text.len() {
        (user_norm, assistant_text)
    } else {
        (assistant_text, user_norm)
    };
    let shorter_token_len = shorter.split_whitespace().count();
    if shorter_token_len < 4 {
        return false;
    }
    token_overlap_ratio(shorter, longer) >= 0.75
}

fn register_assistant_utterance(slot: &mut Option<(String, Instant)>, text: &str, now: Instant) {
    let normalized = normalize_text(text);
    if normalized.len() >= 8 {
        *slot = Some((normalized, now));
    }
}

fn is_probable_self_echo(
    wake_gate: &WakeWordGate,
    last_assistant_utterance: &Option<(String, Instant)>,
    user_text: &str,
    now: Instant,
) -> bool {
    let user_lower = user_text.to_lowercase();
    let Some((assistant_text, ts)) = last_assistant_utterance else {
        return false;
    };
    if now.saturating_duration_since(*ts) > Duration::from_secs(8) {
        return false;
    }
    let user_norm = normalize_text(user_text);
    if user_norm.len() < 8 {
        return false;
    }
    if wake_gate
        .phrases()
        .iter()
        .any(|phrase| user_lower.contains(&phrase.to_lowercase()))
    {
        let stripped = strip_wake_phrase(wake_gate, user_text);
        let stripped_norm = normalize_text(&stripped);
        if stripped_norm.len() >= 8 && is_probable_echo_text(&stripped_norm, assistant_text) {
            return true;
        }
        return false;
    }
    is_probable_echo_text(&user_norm, assistant_text)
}

fn mandatory_wake_word_config(config: &WakeWordConfig) -> WakeWordConfig {
    let mut forced = config.clone();
    forced.enabled = true;
    if forced.phrases.is_empty() {
        forced.phrases = vec!["computer".to_string()];
    }
    forced
}

fn strip_wake_phrase(wake_gate: &WakeWordGate, text: &str) -> String {
    let lower = text.to_lowercase();
    for phrase in wake_gate.phrases() {
        let p = phrase.to_lowercase();
        if let Some(index) = lower.find(&p) {
            let end = index + p.len();
            return text[end..]
                .trim_start_matches(|ch: char| ch == ',' || ch == ':' || ch.is_whitespace())
                .to_string();
        }
    }
    text.to_string()
}

fn try_activate_from_transcript(
    wake_gate: &mut WakeWordGate,
    text: &str,
    now: Instant,
) -> Option<String> {
    let lower = text.to_lowercase();
    for phrase in wake_gate.phrases() {
        let p = phrase.to_lowercase();
        if let Some(index) = lower.find(&p) {
            wake_gate.activate(now);
            let end = index + p.len();
            let stripped = text[end..]
                .trim_start_matches(|ch: char| ch == ',' || ch == ':' || ch.is_whitespace())
                .to_string();
            return Some(stripped);
        }
    }
    None
}

pub struct MacOsSkillExecutor {
    computer: MacOsComputerSkill,
    app_switcher: MacOsAppSwitcherSkill,
    reminder: MacOsReminderSkill,
    message: MacOsMessagesSkill,
    timer: MacOsClockTimerSkill,
    shopping: MacOsNotesShoppingListSkill,
    volume: MacOsVolumeSkill,
    media: MacOsMusicSkill,
    screenshot: MacOsScreenshotSkill,
}

impl Default for MacOsSkillExecutor {
    fn default() -> Self {
        Self {
            computer: MacOsComputerSkill::new(),
            app_switcher: MacOsAppSwitcherSkill::new(),
            reminder: MacOsReminderSkill::new(),
            message: MacOsMessagesSkill::new(),
            timer: MacOsClockTimerSkill::new(),
            shopping: MacOsNotesShoppingListSkill::new(),
            volume: MacOsVolumeSkill::new(),
            media: MacOsMusicSkill::new(),
            screenshot: MacOsScreenshotSkill::new(),
        }
    }
}

fn slot_str<'a>(slots: &'a Value, key: &str) -> Option<&'a str> {
    slots.get(key).and_then(Value::as_str)
}

fn slot_u8(slots: &Value, key: &str) -> Option<u8> {
    slots
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
}

fn supported_frontend_intents() -> Vec<String> {
    vec![
        "skill_computer".to_string(),
        "skill_app_switcher".to_string(),
        "skill_reminder".to_string(),
        "skill_message".to_string(),
        "skill_timer".to_string(),
        "skill_shopping_list".to_string(),
        "skill_volume".to_string(),
        "skill_media".to_string(),
        "skill_screenshot".to_string(),
        "skill_assistant".to_string(),
    ]
}

#[async_trait]
impl FrontendSkillExecutor for MacOsSkillExecutor {
    async fn execute(&self, intent: &FrontendSkillIntent) -> Result<String, DynError> {
        match intent.intent.as_str() {
            "skill_computer" => {
                let action = slot_str(&intent.slots, "computer_action");
                let target = slot_str(&intent.slots, "computer_target");
                let result = self.computer.execute(action, target).await?;
                Ok(result.to_prompt_context())
            }
            "skill_app_switcher" => {
                let action = slot_str(&intent.slots, "app_switcher_action");
                let target = slot_str(&intent.slots, "app_switcher_target");
                let result = self.app_switcher.execute(action, target).await?;
                Ok(result.to_prompt_context())
            }
            "skill_reminder" => {
                let title = slot_str(&intent.slots, "reminder_title").unwrap_or("");
                if title.is_empty() {
                    return Err("missing reminder title".into());
                }
                let when = slot_str(&intent.slots, "reminder_when");
                let result = self.reminder.execute(title, when).await?;
                Ok(result.to_prompt_context())
            }
            "skill_message" => {
                let contact = slot_str(&intent.slots, "message_contact").unwrap_or("");
                let message = slot_str(&intent.slots, "message_text").unwrap_or("");
                if contact.is_empty() || message.is_empty() {
                    return Err("missing message contact or text".into());
                }
                let result = self.message.execute(contact, message).await?;
                Ok(result.to_prompt_context())
            }
            "skill_timer" => {
                let duration = slot_str(&intent.slots, "timer_duration").unwrap_or("");
                if duration.is_empty() {
                    return Err("missing timer duration".into());
                }
                let name = slot_str(&intent.slots, "timer_name");
                let result = self.timer.execute(duration, name).await?;
                Ok(result.to_prompt_context())
            }
            "skill_shopping_list" => {
                let action = slot_str(&intent.slots, "shopping_action").unwrap_or("");
                let items = slot_str(&intent.slots, "shopping_items").unwrap_or("");
                if action.is_empty() || items.is_empty() {
                    return Err("missing shopping action or items".into());
                }
                let when = slot_str(&intent.slots, "shopping_when");
                let result = self.shopping.execute(action, items, when).await?;
                Ok(result.to_prompt_context())
            }
            "skill_volume" => {
                let action = slot_str(&intent.slots, "volume_action");
                let level = slot_u8(&intent.slots, "volume_level");
                let result = self.volume.execute(action, level).await?;
                Ok(result.to_prompt_context())
            }
            "skill_media" => {
                let action = slot_str(&intent.slots, "media_action");
                let target = slot_str(&intent.slots, "media_target");
                let result = self.media.execute(action, target).await?;
                Ok(result.to_prompt_context())
            }
            "skill_screenshot" => {
                let filename = slot_str(&intent.slots, "screenshot_filename");
                let result = self.screenshot.execute(filename).await?;
                Ok(result.to_prompt_context())
            }
            "skill_assistant" => {
                Err("I cannot do that assistant action on this frontend yet.".into())
            }
            _ => Err(format!("unsupported frontend intent: {}", intent.intent).into()),
        }
    }
}

pub async fn run_macos_frontend(config: Config) -> Result<(), DynError> {
    let backend_url =
        std::env::var("AICE_BACKEND_URL").unwrap_or_else(|_| "http://127.0.0.1:8781".to_string());
    let client = FrontendClient::new(backend_url);
    let executor = MacOsSkillExecutor::default();
    let frontend_session_id = std::env::var("AICE_FRONTEND_SESSION_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "macos-frontend-session".to_string());
    if let Err(error) = client
        .activate_frontend(&frontend_session_id, supported_frontend_intents())
        .await
    {
        info!(%error, "frontend activation failed, continuing without registration");
        record_error("frontend_activate");
    }

    let mut capture = CpalCapture::from_preferred_name(config.audio.input_device.as_deref())?;
    let mut stt = WhisperSttStream::new(Path::new(&config.stt.whisper_model_path))?;
    let mut tts = PiperTtsSink::new(Path::new(&config.tts.piper_model_path))?;

    let turn_window_ms = if config.audio.enable_endpointing_tuning {
        config.audio.tuned_turn_window_ms
    } else {
        config.audio.turn_window_ms
    };
    let chunk_timeout_cfg_ms = if config.audio.enable_endpointing_tuning {
        config.audio.tuned_chunk_timeout_ms
    } else {
        config.audio.chunk_timeout_ms
    };
    let speech_end_silence_cfg_ms = if config.audio.enable_endpointing_tuning {
        config.audio.tuned_speech_end_silence_ms
    } else {
        config.audio.speech_end_silence_ms
    };
    let tts_chunk_bytes = if config.tts.enable_chunked_push_optimization {
        config.tts.push_chunk_bytes.max(24)
    } else {
        24
    };

    let target_samples = ((SAMPLE_RATE as u64 * turn_window_ms) / 1000) as usize;
    let target_samples = target_samples.max(1);
    let chunk_timeout_ms = chunk_timeout_cfg_ms.max(1);
    let timeout = Duration::from_millis(chunk_timeout_ms);
    let speech_end_silence_ms = speech_end_silence_cfg_ms.max(chunk_timeout_ms);
    let speech_rms_threshold = config.audio.speech_rms_threshold.max(0.0);
    let idle_sleep = Duration::from_millis(config.audio.idle_sleep_ms);
    let forced_wake_word = mandatory_wake_word_config(&config.wake_word);
    let mut wake_gate = WakeWordGate::new(forced_wake_word);

    let (cancel_tx, mut cancel_rx) = broadcast::channel(1);
    let _ = cancel_tx;
    let mut buffered_samples = 0_usize;
    let mut silence_after_voice_ms = 0_u64;
    let mut observed_voice = false;
    let mut voiced_samples = 0_usize;
    let mut turn_started_at: Option<Instant> = None;
    let mut last_assistant_utterance: Option<(String, Instant)> = None;

    loop {
        let mut should_flush_turn = false;
        match capture.read_chunk(timeout) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    continue;
                }
                let chunk_ms = chunk_duration_ms(chunk.len());
                if is_voiced_chunk(&chunk, speech_rms_threshold) {
                    if turn_started_at.is_none() {
                        turn_started_at = Some(Instant::now());
                    }
                    observed_voice = true;
                    silence_after_voice_ms = 0;
                    stt.push_audio(&chunk).await?;
                    buffered_samples += chunk.len();
                    voiced_samples += chunk.len();
                } else if observed_voice {
                    stt.push_audio(&chunk).await?;
                    buffered_samples += chunk.len();
                    silence_after_voice_ms = silence_after_voice_ms.saturating_add(chunk_ms);
                    if silence_after_voice_ms >= speech_end_silence_ms && buffered_samples > 0 {
                        should_flush_turn = true;
                    }
                }
            }
            Err(CaptureError::Timeout) => {
                if observed_voice && buffered_samples > 0 {
                    silence_after_voice_ms =
                        silence_after_voice_ms.saturating_add(chunk_timeout_ms);
                    if silence_after_voice_ms >= speech_end_silence_ms {
                        should_flush_turn = true;
                    }
                }
                if !should_flush_turn {
                    tokio::time::sleep(idle_sleep).await;
                    continue;
                }
            }
            Err(error) => {
                return Err(Box::new(error));
            }
        }

        if !should_flush_turn {
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

        let stt_started = Instant::now();
        let mut transcript = stt.flush().await?;
        let stt_elapsed = stt_started.elapsed();
        record_stage_duration(Stage::Stt, stt_elapsed);
        transcript = transcript.trim().to_string();
        if transcript.is_empty() {
            voiced_samples = 0;
            turn_started_at = None;
            continue;
        }
        let turn_start = turn_started_at.unwrap_or(stt_started);
        let mic_to_stt_elapsed = turn_start.elapsed();
        record_session_start();
        record_mic_to_stt_duration(mic_to_stt_elapsed);
        if voiced_samples > 0 {
            let voiced_ms = chunk_duration_ms(voiced_samples);
            let voiced_elapsed = Duration::from_millis(voiced_ms);
            record_speech_voiced_duration(voiced_elapsed);
            let endpointing_wait = mic_to_stt_elapsed
                .saturating_sub(voiced_elapsed)
                .saturating_sub(stt_elapsed);
            record_endpointing_wait_duration(endpointing_wait);
        }
        voiced_samples = 0;
        turn_started_at = None;
        let now = Instant::now();
        if is_probable_self_echo(&wake_gate, &last_assistant_utterance, &transcript, now) {
            continue;
        }

        if wake_gate.is_enabled() && !wake_gate.should_listen(now) {
            if let Some(stripped) = try_activate_from_transcript(&mut wake_gate, &transcript, now) {
                transcript = stripped;
                if transcript.is_empty() {
                    continue;
                }
            } else {
                continue;
            }
        } else if wake_gate.is_enabled() {
            transcript = strip_wake_phrase(&wake_gate, &transcript);
            if transcript.is_empty() {
                continue;
            }
        }

        let llm_started = Instant::now();
        let rpc_started = Instant::now();
        let answer = match client
            .run_turn(&frontend_session_id, &transcript, &executor)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                record_error("frontend_run_turn");
                return Err(error);
            }
        };
        if let Err(error) = client.heartbeat_frontend(&frontend_session_id).await {
            info!(%error, "frontend heartbeat failed");
            record_error("frontend_heartbeat");
        }
        record_frontend_rpc_duration("run_turn_total", rpc_started.elapsed());
        record_stage_duration(Stage::Llm, llm_started.elapsed());

        if !answer.trim().is_empty() {
            let tts_started = Instant::now();
            register_assistant_utterance(&mut last_assistant_utterance, &answer, Instant::now());
            let mut first_audio_recorded = false;
            for chunk in answer.as_bytes().chunks(tts_chunk_bytes) {
                if let Ok(()) = cancel_rx.try_recv() {
                    tts.request_stop_playback();
                    break;
                }
                let text = String::from_utf8_lossy(chunk).to_string();
                tts.push_text(&text).await?;
                if !first_audio_recorded {
                    let first_audio = turn_start.elapsed();
                    record_turn_time_to_first_audio(first_audio);
                    record_first_audio_latency(first_audio);
                    record_tts_first_audio_latency(tts_started.elapsed());
                    first_audio_recorded = true;
                }
            }
            let tts_flush_started = Instant::now();
            tts.flush().await?;
            record_tts_flush_duration(tts_flush_started.elapsed());
            record_stage_duration(Stage::Tts, tts_started.elapsed());
            record_frontend_tts_playback_duration(tts_started.elapsed());
        }

        if wake_gate.is_enabled() {
            wake_gate.deactivate();
        }

        info!(transcript = %transcript, "frontend turn complete");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chunk_duration_ms, is_probable_self_echo, is_voiced_chunk, normalize_text,
        strip_wake_phrase, try_activate_from_transcript,
    };
    use core_config::WakeWordConfig;
    use core_vad::WakeWordGate;
    use std::time::Instant;

    #[test]
    fn wake_activation_strips_phrase_from_transcript() {
        let mut gate = WakeWordGate::new(WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        let now = Instant::now();
        let stripped = try_activate_from_transcript(&mut gate, "computer open safari", now);
        assert_eq!(stripped.as_deref(), Some("open safari"));
        assert!(gate.should_listen(now));
    }

    #[test]
    fn wake_strip_removes_phrase_when_gate_is_open() {
        let gate = WakeWordGate::new(WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        let stripped = strip_wake_phrase(&gate, "computer, set timer");
        assert_eq!(stripped, "set timer");
    }

    #[test]
    fn mandatory_wake_word_forces_enabled_flag() {
        let forced = super::mandatory_wake_word_config(&WakeWordConfig {
            enabled: false,
            phrases: vec!["computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        assert!(forced.enabled);
        assert_eq!(forced.phrases, vec!["computer".to_string()]);
    }

    #[test]
    fn voiced_chunk_detection_matches_threshold() {
        let silence = vec![0_i16; 320];
        let voice = vec![1_600_i16; 320];
        assert!(!is_voiced_chunk(&silence, 0.008));
        assert!(is_voiced_chunk(&voice, 0.008));
    }

    #[test]
    fn chunk_duration_matches_16k_pipeline() {
        assert_eq!(chunk_duration_ms(320), 20);
    }

    #[test]
    fn self_echo_detects_recent_assistant_reply_without_wake_phrase() {
        let gate = WakeWordGate::new(WakeWordConfig {
            enabled: true,
            phrases: vec!["assistant".to_string(), "computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        let now = Instant::now();
        let last = Some((
            normalize_text("I cannot do that assistant action on this frontend yet."),
            now,
        ));
        assert!(is_probable_self_echo(
            &gate,
            &last,
            "I cannot do that assistant action on this frontend yet.",
            now
        ));
    }

    #[test]
    fn self_echo_detects_wake_prefixed_replay_of_assistant_text() {
        let gate = WakeWordGate::new(WakeWordConfig {
            enabled: true,
            phrases: vec!["assistant".to_string(), "computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        let now = Instant::now();
        let last = Some((
            normalize_text("skill not implemented on macos frontend"),
            now,
        ));
        assert!(is_probable_self_echo(
            &gate,
            &last,
            "assistant, skill not implemented on macos frontend",
            now
        ));
    }

    #[test]
    fn self_echo_detects_partial_stt_replay_with_wake_prefix() {
        let gate = WakeWordGate::new(WakeWordConfig {
            enabled: true,
            phrases: vec!["assistant".to_string(), "computer".to_string()],
            sensitivity: 0.5,
            cooldown_secs: 6,
        });
        let now = Instant::now();
        let last = Some((
            normalize_text("I cannot do that assistant action on this frontend yet."),
            now,
        ));
        assert!(is_probable_self_echo(
            &gate,
            &last,
            "assistant, to action on this front end yet.",
            now
        ));
    }
}
