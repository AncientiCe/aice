//! Baseline voice-assistant metrics.
//!
//! Names follow convention: voice_<subsystem>_<operation>_<unit>.

use metrics::{counter, histogram};
use std::time::Duration;

/// Voice pipeline stages for duration tracking.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Stage {
    Stt,
    Llm,
    Tts,
    Vad,
    Orchestrator,
}

impl Stage {
    fn as_label(self) -> &'static str {
        match self {
            Stage::Stt => "stt",
            Stage::Llm => "llm",
            Stage::Tts => "tts",
            Stage::Vad => "vad",
            Stage::Orchestrator => "orchestrator",
        }
    }
}

const VOICE_SESSIONS_TOTAL: &str = "voice_sessions_total";
const VOICE_ERRORS_TOTAL: &str = "voice_errors_total";
const VOICE_STAGE_DURATION_SECONDS: &str = "voice_stage_duration_seconds";
const VOICE_MIC_TO_STT_DURATION_SECONDS: &str = "voice_mic_to_stt_duration_seconds";
const VOICE_FIRST_TOKEN_LATENCY_SECONDS: &str = "voice_first_token_latency_seconds";
const VOICE_FIRST_AUDIO_LATENCY_SECONDS: &str = "voice_first_audio_latency_seconds";
const VOICE_SPEECH_VOICED_DURATION_SECONDS: &str = "voice_speech_voiced_duration_seconds";
const VOICE_ENDPOINTING_WAIT_DURATION_SECONDS: &str = "voice_endpointing_wait_duration_seconds";
const VOICE_LLM_FIRST_TOKEN_LATENCY_SECONDS: &str = "voice_llm_first_token_latency_seconds";
const VOICE_LLM_STREAM_TAIL_DURATION_SECONDS: &str = "voice_llm_stream_tail_duration_seconds";
const VOICE_TURN_TIME_TO_FIRST_AUDIO_SECONDS: &str = "voice_turn_time_to_first_audio_seconds";
const VOICE_TTS_FIRST_AUDIO_LATENCY_SECONDS: &str = "voice_tts_first_audio_latency_seconds";
const VOICE_TTS_FLUSH_DURATION_SECONDS: &str = "voice_tts_flush_duration_seconds";
const VOICE_SKILL_DURATION_SECONDS: &str = "voice_skill_duration_seconds";
const VOICE_INTERRUPTIONS_TOTAL: &str = "voice_interruptions_total";
const VOICE_CANCELLATION_SUCCESS_TOTAL: &str = "voice_cancellation_success_total";
const POD_CONNECTIONS_TOTAL: &str = "pod_connections_total";
const POD_DISCONNECTS_TOTAL: &str = "pod_disconnects_total";
const POD_EGRESS_SEND_ERRORS_TOTAL: &str = "pod_egress_send_errors_total";
const POD_EGRESS_QUEUE_DROPS_TOTAL: &str = "pod_egress_queue_drops_total";
const POD_AUDIO_FRAMES_TOTAL: &str = "pod_audio_frames_total";
const POD_AUDIO_BYTES_TOTAL: &str = "pod_audio_bytes_total";
const POD_TTS_CHUNKS_TOTAL: &str = "pod_tts_chunks_total";
const POD_TTS_BYTES_TOTAL: &str = "pod_tts_bytes_total";
const POD_EGRESS_DEVICE_LOCK_POISON_TOTAL: &str = "pod_egress_device_lock_poison_total";
const VOICE_INTENT_CLASSIFIER_TOTAL: &str = "voice_intent_classifier_total";
const VOICE_INTENT_ROUTED_TOTAL: &str = "voice_intent_routed_total";
const VOICE_WEATHER_SKILL_TOTAL: &str = "voice_weather_skill_total";
const VOICE_TIME_SKILL_TOTAL: &str = "voice_time_skill_total";
const VOICE_DISTANCE_SKILL_TOTAL: &str = "voice_distance_skill_total";
const VOICE_SMART_HOME_SKILL_TOTAL: &str = "voice_smart_home_skill_total";
const VOICE_ASSISTANT_SKILL_TOTAL: &str = "voice_assistant_skill_total";
const VOICE_MEDIA_SKILL_TOTAL: &str = "voice_media_skill_total";
const VOICE_MEMORY_SKILL_TOTAL: &str = "voice_memory_skill_total";
const VOICE_COMPUTER_SKILL_TOTAL: &str = "voice_computer_skill_total";
const VOICE_SCREENSHOT_SKILL_TOTAL: &str = "voice_screenshot_skill_total";
const VOICE_APP_SWITCHER_SKILL_TOTAL: &str = "voice_app_switcher_skill_total";
const VOICE_REMINDER_SKILL_TOTAL: &str = "voice_reminder_skill_total";
const VOICE_MESSAGE_SKILL_TOTAL: &str = "voice_message_skill_total";
const VOICE_TIMER_SKILL_TOTAL: &str = "voice_timer_skill_total";
const VOICE_SHOPPING_LIST_SKILL_TOTAL: &str = "voice_shopping_list_skill_total";
const VOICE_VOLUME_SKILL_TOTAL: &str = "voice_volume_skill_total";
const VOICE_POLICY_DENIED_TOTAL: &str = "voice_policy_denied_total";
const VOICE_LOCATION_PRELOAD_TOTAL: &str = "voice_location_preload_total";
const VOICE_LOCATION_CONTRACT_TOTAL: &str = "voice_location_contract_total";
const VOICE_LOCATION_CONTRACT_DURATION_SECONDS: &str = "voice_location_contract_duration_seconds";
const VOICE_SHUTDOWN_SIGNALS_TOTAL: &str = "voice_shutdown_signals_total";
const MEMORY_LOAD_TOTAL: &str = "memory_load_total";
const MEMORY_SAVE_TOTAL: &str = "memory_save_total";
const MEMORY_LOAD_ERRORS_TOTAL: &str = "memory_load_errors_total";
const MEMORY_SAVE_ERRORS_TOTAL: &str = "memory_save_errors_total";
const MEMORY_LOAD_DURATION_SECONDS: &str = "memory_load_duration_seconds";
const MEMORY_SAVE_DURATION_SECONDS: &str = "memory_save_duration_seconds";
const SMART_HOME_EXECUTE_TOTAL: &str = "smart_home_execute_total";
const SMART_HOME_EXECUTE_DURATION_SECONDS: &str = "smart_home_execute_duration_seconds";
const MEDIA_EXECUTE_TOTAL: &str = "media_execute_total";
const MEDIA_EXECUTE_DURATION_SECONDS: &str = "media_execute_duration_seconds";
const MEMORY_FACT_STORE_TOTAL: &str = "memory_fact_store_total";
const MEMORY_FACT_RECALL_TOTAL: &str = "memory_fact_recall_total";
const MEMORY_FACT_STORE_DURATION_SECONDS: &str = "memory_fact_store_duration_seconds";
const MEMORY_FACT_RECALL_DURATION_SECONDS: &str = "memory_fact_recall_duration_seconds";
const SCREENSHOT_SKILL_EXECUTE_TOTAL: &str = "screenshot_skill_execute_total";
const SCREENSHOT_SKILL_ERRORS_TOTAL: &str = "screenshot_skill_errors_total";
const SCREENSHOT_SKILL_EXECUTE_DURATION_SECONDS: &str = "screenshot_skill_execute_duration_seconds";

/// Register metric descriptors / ensure they exist. Call once at startup.
pub fn register_metrics() {
    counter!(VOICE_SESSIONS_TOTAL, 0);
    counter!(VOICE_ERRORS_TOTAL, 0, "kind" => "unknown");
    histogram!(VOICE_STAGE_DURATION_SECONDS, 0.0_f64, "stage" => "unknown");
    histogram!(VOICE_MIC_TO_STT_DURATION_SECONDS, 0.0_f64);
    histogram!(VOICE_FIRST_TOKEN_LATENCY_SECONDS, 0.0_f64);
    histogram!(VOICE_FIRST_AUDIO_LATENCY_SECONDS, 0.0_f64);
    histogram!(VOICE_SPEECH_VOICED_DURATION_SECONDS, 0.0_f64);
    histogram!(VOICE_ENDPOINTING_WAIT_DURATION_SECONDS, 0.0_f64);
    histogram!(VOICE_LLM_FIRST_TOKEN_LATENCY_SECONDS, 0.0_f64);
    histogram!(VOICE_LLM_STREAM_TAIL_DURATION_SECONDS, 0.0_f64);
    histogram!(VOICE_TURN_TIME_TO_FIRST_AUDIO_SECONDS, 0.0_f64);
    histogram!(VOICE_TTS_FIRST_AUDIO_LATENCY_SECONDS, 0.0_f64);
    histogram!(VOICE_TTS_FLUSH_DURATION_SECONDS, 0.0_f64);
    histogram!(VOICE_SKILL_DURATION_SECONDS, 0.0_f64, "skill" => "unknown");
    counter!(VOICE_INTERRUPTIONS_TOTAL, 0);
    counter!(VOICE_CANCELLATION_SUCCESS_TOTAL, 0);
    counter!(POD_CONNECTIONS_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_DISCONNECTS_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_EGRESS_SEND_ERRORS_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_EGRESS_QUEUE_DROPS_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_AUDIO_FRAMES_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_AUDIO_BYTES_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_TTS_CHUNKS_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_TTS_BYTES_TOTAL, 0, "device_id" => "unknown");
    counter!(POD_EGRESS_DEVICE_LOCK_POISON_TOTAL, 0, "operation" => "unknown");
    counter!(VOICE_INTENT_CLASSIFIER_TOTAL, 0);
    counter!(VOICE_INTENT_ROUTED_TOTAL, 0, "intent" => "unknown");
    counter!(VOICE_WEATHER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_TIME_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_DISTANCE_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_SMART_HOME_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_ASSISTANT_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_MEDIA_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_MEMORY_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_COMPUTER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_SCREENSHOT_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_APP_SWITCHER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_REMINDER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_MESSAGE_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_TIMER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_SHOPPING_LIST_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_VOLUME_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_POLICY_DENIED_TOTAL, 0, "reason" => "unknown");
    counter!(VOICE_LOCATION_PRELOAD_TOTAL, 0, "result" => "unknown");
    counter!(
        VOICE_LOCATION_CONTRACT_TOTAL,
        0,
        "intent" => "unknown",
        "result" => "unknown"
    );
    histogram!(
        VOICE_LOCATION_CONTRACT_DURATION_SECONDS,
        0.0_f64,
        "intent" => "unknown"
    );
    counter!(VOICE_SHUTDOWN_SIGNALS_TOTAL, 0, "signal" => "unknown", "action" => "unknown");
    counter!(MEMORY_LOAD_TOTAL, 0);
    counter!(MEMORY_SAVE_TOTAL, 0);
    counter!(MEMORY_LOAD_ERRORS_TOTAL, 0, "kind" => "unknown");
    counter!(MEMORY_SAVE_ERRORS_TOTAL, 0, "kind" => "unknown");
    histogram!(MEMORY_LOAD_DURATION_SECONDS, 0.0_f64);
    histogram!(MEMORY_SAVE_DURATION_SECONDS, 0.0_f64);
    counter!(SMART_HOME_EXECUTE_TOTAL, 0, "result" => "unknown", "action" => "unknown");
    histogram!(SMART_HOME_EXECUTE_DURATION_SECONDS, 0.0_f64, "action" => "unknown");
    counter!(MEDIA_EXECUTE_TOTAL, 0, "result" => "unknown", "action" => "unknown");
    histogram!(MEDIA_EXECUTE_DURATION_SECONDS, 0.0_f64, "action" => "unknown");
    counter!(MEMORY_FACT_STORE_TOTAL, 0, "result" => "unknown", "source" => "unknown");
    counter!(MEMORY_FACT_RECALL_TOTAL, 0, "result" => "unknown");
    histogram!(MEMORY_FACT_STORE_DURATION_SECONDS, 0.0_f64, "source" => "unknown");
    histogram!(MEMORY_FACT_RECALL_DURATION_SECONDS, 0.0_f64);
    counter!(SCREENSHOT_SKILL_EXECUTE_TOTAL, 0, "result" => "unknown");
    counter!(SCREENSHOT_SKILL_ERRORS_TOTAL, 0, "kind" => "unknown");
    histogram!(SCREENSHOT_SKILL_EXECUTE_DURATION_SECONDS, 0.0_f64);
}

/// Record a new voice session start.
pub fn record_session_start() {
    counter!(VOICE_SESSIONS_TOTAL, 1);
}

/// Record an error by kind.
pub fn record_error(kind: &str) {
    let k = kind.to_string();
    counter!(VOICE_ERRORS_TOTAL, 1, "kind" => k);
}

/// Record duration of a pipeline stage.
pub fn record_stage_duration(stage: Stage, duration: Duration) {
    let secs = duration.as_secs_f64();
    histogram!(
        VOICE_STAGE_DURATION_SECONDS,
        secs,
        "stage" => stage.as_label()
    );
}

/// Record total latency from mic capture start to STT completion.
pub fn record_mic_to_stt_duration(duration: Duration) {
    histogram!(VOICE_MIC_TO_STT_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record latency from turn/request start to first LLM token.
pub fn record_first_token_latency(duration: Duration) {
    histogram!(VOICE_FIRST_TOKEN_LATENCY_SECONDS, duration.as_secs_f64());
}

/// Record latency from first token to first TTS push (time to first audio).
pub fn record_first_audio_latency(duration: Duration) {
    histogram!(VOICE_FIRST_AUDIO_LATENCY_SECONDS, duration.as_secs_f64());
}

/// Record voiced speech duration (voice-only, excluding pauses/silence).
pub fn record_speech_voiced_duration(duration: Duration) {
    histogram!(VOICE_SPEECH_VOICED_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record endpointing wait (post-speech pause before STT flush starts).
pub fn record_endpointing_wait_duration(duration: Duration) {
    histogram!(
        VOICE_ENDPOINTING_WAIT_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record LLM first-token latency from request start.
pub fn record_llm_first_token_latency(duration: Duration) {
    histogram!(
        VOICE_LLM_FIRST_TOKEN_LATENCY_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record post-first-token LLM stream tail duration.
pub fn record_llm_stream_tail_duration(duration: Duration) {
    histogram!(
        VOICE_LLM_STREAM_TAIL_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record turn-level latency from speech start to first audio milestone.
pub fn record_turn_time_to_first_audio(duration: Duration) {
    histogram!(
        VOICE_TURN_TIME_TO_FIRST_AUDIO_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record time from first token to first synthesized audio frame.
pub fn record_tts_first_audio_latency(duration: Duration) {
    histogram!(
        VOICE_TTS_FIRST_AUDIO_LATENCY_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record TTS output flush/drain completion duration.
pub fn record_tts_flush_duration(duration: Duration) {
    histogram!(VOICE_TTS_FLUSH_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record skill execution latency.
pub fn record_skill_duration(skill: &str, duration: Duration) {
    histogram!(
        VOICE_SKILL_DURATION_SECONDS,
        duration.as_secs_f64(),
        "skill" => skill.to_string()
    );
}

/// Record that the user interrupted (barge-in).
pub fn record_interruption() {
    counter!(VOICE_INTERRUPTIONS_TOTAL, 1);
}

/// Record that TTS/LLM was successfully cancelled after interrupt.
pub fn record_cancellation_success() {
    counter!(VOICE_CANCELLATION_SUCCESS_TOTAL, 1);
}

pub fn record_pod_connection(device_id: &str) {
    counter!(POD_CONNECTIONS_TOTAL, 1, "device_id" => device_id.to_string());
}

pub fn record_pod_disconnect(device_id: &str) {
    counter!(POD_DISCONNECTS_TOTAL, 1, "device_id" => device_id.to_string());
}

pub fn record_pod_egress_send_error(device_id: &str) {
    counter!(
        POD_EGRESS_SEND_ERRORS_TOTAL,
        1,
        "device_id" => device_id.to_string()
    );
}

pub fn record_pod_egress_queue_drop(device_id: &str) {
    counter!(
        POD_EGRESS_QUEUE_DROPS_TOTAL,
        1,
        "device_id" => device_id.to_string()
    );
}

pub fn record_pod_audio_frame(device_id: &str, bytes: usize) {
    let id = device_id.to_string();
    counter!(POD_AUDIO_FRAMES_TOTAL, 1, "device_id" => id.clone());
    counter!(POD_AUDIO_BYTES_TOTAL, bytes as u64, "device_id" => id);
}

pub fn record_pod_tts_chunk(device_id: &str, bytes: usize) {
    let id = device_id.to_string();
    counter!(POD_TTS_CHUNKS_TOTAL, 1, "device_id" => id.clone());
    counter!(POD_TTS_BYTES_TOTAL, bytes as u64, "device_id" => id);
}

pub fn record_pod_egress_device_lock_poison(operation: &str) {
    counter!(
        POD_EGRESS_DEVICE_LOCK_POISON_TOTAL,
        1,
        "operation" => operation.to_string()
    );
}

/// Record an intent classification call.
pub fn record_intent_classifier() {
    counter!(VOICE_INTENT_CLASSIFIER_TOTAL, 1);
}

/// Record routed intent (chat or skill_weather).
pub fn record_intent_routed(intent: &str) {
    counter!(VOICE_INTENT_ROUTED_TOTAL, 1, "intent" => intent.to_string());
}

/// Record weather skill result (success or error).
pub fn record_weather_skill(result: &str) {
    counter!(VOICE_WEATHER_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record time skill result (success or error).
pub fn record_time_skill(result: &str) {
    counter!(VOICE_TIME_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record distance skill result (success or error).
pub fn record_distance_skill(result: &str) {
    counter!(VOICE_DISTANCE_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record smart home skill result (success or error).
pub fn record_smart_home_skill(result: &str) {
    counter!(VOICE_SMART_HOME_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record assistant skill result (success or error).
pub fn record_assistant_skill(result: &str) {
    counter!(VOICE_ASSISTANT_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record media skill result (success or error).
pub fn record_media_skill(result: &str) {
    counter!(VOICE_MEDIA_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record memory skill result (success or error).
pub fn record_memory_skill(result: &str) {
    counter!(VOICE_MEMORY_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record computer skill result (success or error).
pub fn record_computer_skill(result: &str) {
    counter!(VOICE_COMPUTER_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record screenshot skill result (success or error).
pub fn record_screenshot_skill(result: &str) {
    counter!(VOICE_SCREENSHOT_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record app switcher skill result (success or error).
pub fn record_app_switcher_skill(result: &str) {
    counter!(
        VOICE_APP_SWITCHER_SKILL_TOTAL,
        1,
        "result" => result.to_string()
    );
}

/// Record reminder skill result (success or error).
pub fn record_reminder_skill(result: &str) {
    counter!(VOICE_REMINDER_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record message skill result (success or error).
pub fn record_message_skill(result: &str) {
    counter!(VOICE_MESSAGE_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record timer skill result (success or error).
pub fn record_timer_skill(result: &str) {
    counter!(VOICE_TIMER_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record shopping list skill result (success or error).
pub fn record_shopping_list_skill(result: &str) {
    counter!(VOICE_SHOPPING_LIST_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record volume skill result (success or error).
pub fn record_volume_skill(result: &str) {
    counter!(VOICE_VOLUME_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record policy denial (reason: e.g. "emergency_stop", "budget_exhausted").
pub fn record_policy_denied(reason: &str) {
    counter!(VOICE_POLICY_DENIED_TOTAL, 1, "reason" => reason.to_string());
}

/// Record startup location preload result (success or failure).
pub fn record_location_preload(result: &str) {
    counter!(VOICE_LOCATION_PRELOAD_TOTAL, 1, "result" => result.to_string());
}

/// Record a location contract normalization decision by intent and result.
pub fn record_location_contract(intent: &str, result: &str) {
    counter!(
        VOICE_LOCATION_CONTRACT_TOTAL,
        1,
        "intent" => intent.to_string(),
        "result" => result.to_string()
    );
}

/// Record latency for location contract normalization.
pub fn record_location_contract_duration(intent: &str, duration: Duration) {
    histogram!(
        VOICE_LOCATION_CONTRACT_DURATION_SECONDS,
        duration.as_secs_f64(),
        "intent" => intent.to_string()
    );
}

/// Record shutdown signal handling action (graceful shutdown vs force-exit).
pub fn record_shutdown_signal(signal: &str, action: &str) {
    counter!(
        VOICE_SHUTDOWN_SIGNALS_TOTAL,
        1,
        "signal" => signal.to_string(),
        "action" => action.to_string()
    );
}

/// Record a memory load (successful load from disk).
pub fn record_memory_load() {
    counter!(MEMORY_LOAD_TOTAL, 1);
}

/// Record memory load duration in seconds.
pub fn record_memory_load_duration(duration: Duration) {
    histogram!(MEMORY_LOAD_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record a memory load error by kind (e.g. "io", "parse").
pub fn record_memory_load_error(kind: &str) {
    counter!(MEMORY_LOAD_ERRORS_TOTAL, 1, "kind" => kind.to_string());
}

/// Record a memory save (successful write to disk).
pub fn record_memory_save() {
    counter!(MEMORY_SAVE_TOTAL, 1);
}

/// Record memory save duration in seconds.
pub fn record_memory_save_duration(duration: Duration) {
    histogram!(MEMORY_SAVE_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record a memory save error by kind (e.g. "io", "serialize").
pub fn record_memory_save_error(kind: &str) {
    counter!(MEMORY_SAVE_ERRORS_TOTAL, 1, "kind" => kind.to_string());
}

pub fn record_smart_home_execute(result: &str, action: &str) {
    counter!(
        SMART_HOME_EXECUTE_TOTAL,
        1,
        "result" => result.to_string(),
        "action" => action.to_string()
    );
}

pub fn record_smart_home_execute_duration(action: &str, duration: Duration) {
    histogram!(
        SMART_HOME_EXECUTE_DURATION_SECONDS,
        duration.as_secs_f64(),
        "action" => action.to_string()
    );
}

pub fn record_media_execute(result: &str, action: &str) {
    counter!(
        MEDIA_EXECUTE_TOTAL,
        1,
        "result" => result.to_string(),
        "action" => action.to_string()
    );
}

pub fn record_media_execute_duration(action: &str, duration: Duration) {
    histogram!(
        MEDIA_EXECUTE_DURATION_SECONDS,
        duration.as_secs_f64(),
        "action" => action.to_string()
    );
}

pub fn record_memory_fact_store(result: &str, source: &str) {
    counter!(
        MEMORY_FACT_STORE_TOTAL,
        1,
        "result" => result.to_string(),
        "source" => source.to_string()
    );
}

pub fn record_memory_fact_store_duration(source: &str, duration: Duration) {
    histogram!(
        MEMORY_FACT_STORE_DURATION_SECONDS,
        duration.as_secs_f64(),
        "source" => source.to_string()
    );
}

pub fn record_memory_fact_recall(result: &str) {
    counter!(MEMORY_FACT_RECALL_TOTAL, 1, "result" => result.to_string());
}

pub fn record_memory_fact_recall_duration(duration: Duration) {
    histogram!(MEMORY_FACT_RECALL_DURATION_SECONDS, duration.as_secs_f64());
}
