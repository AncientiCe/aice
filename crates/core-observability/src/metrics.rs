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
const VOICE_INTENT_VALIDATION_REJECTED_TOTAL: &str = "voice_intent_validation_rejected_total";
const VOICE_INTENT_ROUTED_TOTAL: &str = "voice_intent_routed_total";
const VOICE_WEATHER_SKILL_TOTAL: &str = "voice_weather_skill_total";
const VOICE_TIME_SKILL_TOTAL: &str = "voice_time_skill_total";
const VOICE_DISTANCE_SKILL_TOTAL: &str = "voice_distance_skill_total";
const VOICE_SMART_HOME_SKILL_TOTAL: &str = "voice_smart_home_skill_total";
const VOICE_MEDIA_SKILL_TOTAL: &str = "voice_media_skill_total";
const VOICE_COMPUTER_SKILL_TOTAL: &str = "voice_computer_skill_total";
const VOICE_CALCULATOR_SKILL_TOTAL: &str = "voice_calculator_skill_total";
const VOICE_UNIT_CONVERSION_SKILL_TOTAL: &str = "voice_unit_conversion_skill_total";
const VOICE_CURRENCY_SKILL_TOTAL: &str = "voice_currency_skill_total";
const VOICE_AIR_QUALITY_SKILL_TOTAL: &str = "voice_air_quality_skill_total";
const VOICE_DICTIONARY_SKILL_TOTAL: &str = "voice_dictionary_skill_total";
const VOICE_TRANSLATE_SKILL_TOTAL: &str = "voice_translate_skill_total";
const VOICE_CALENDAR_SKILL_TOTAL: &str = "voice_calendar_skill_total";
const VOICE_MEETING_NOTES_SKILL_TOTAL: &str = "voice_meeting_notes_skill_total";
const VOICE_EMAIL_SKILL_TOTAL: &str = "voice_email_skill_total";
const VOICE_BRIEFING_SKILL_TOTAL: &str = "voice_briefing_skill_total";
const VOICE_JOURNAL_SKILL_TOTAL: &str = "voice_journal_skill_total";
const VOICE_SCREEN_OCR_SKILL_TOTAL: &str = "voice_screen_ocr_skill_total";
const VOICE_NEWS_SUMMARY_CHUNK_TOTAL: &str = "voice_news_summary_chunk_total";
const VOICE_NEWS_SUMMARY_DURATION_SECONDS: &str = "voice_news_summary_duration_seconds";
const VOICE_SCREENSHOT_SKILL_TOTAL: &str = "voice_screenshot_skill_total";
const VOICE_APP_SWITCHER_SKILL_TOTAL: &str = "voice_app_switcher_skill_total";
const VOICE_REMINDER_SKILL_TOTAL: &str = "voice_reminder_skill_total";
const VOICE_MESSAGE_SKILL_TOTAL: &str = "voice_message_skill_total";
const VOICE_TIMER_SKILL_TOTAL: &str = "voice_timer_skill_total";
const VOICE_SHOPPING_LIST_SKILL_TOTAL: &str = "voice_shopping_list_skill_total";
const VOICE_VOLUME_SKILL_TOTAL: &str = "voice_volume_skill_total";
const VOICE_POLICY_DENIED_TOTAL: &str = "voice_policy_denied_total";
const VOICE_LOCATION_PRELOAD_TOTAL: &str = "voice_location_preload_total";
const VOICE_MODEL_PRELOAD_TOTAL: &str = "voice_model_preload_total";
const VOICE_MODEL_PRELOAD_DURATION_SECONDS: &str = "voice_model_preload_duration_seconds";
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
const BACKEND_TURN_TOTAL: &str = "backend_turn_total";
const BACKEND_TURN_DURATION_SECONDS: &str = "backend_turn_duration_seconds";
const BACKEND_TURN_STAGE_DURATION_SECONDS: &str = "backend_turn_stage_duration_seconds";
const BACKEND_HTTP_REQUESTS_TOTAL: &str = "backend_http_requests_total";
const BACKEND_HTTP_REQUEST_DURATION_SECONDS: &str = "backend_http_request_duration_seconds";
const BACKEND_SKILL_EXECUTE_TOTAL: &str = "backend_skill_execute_total";
const BACKEND_SKILL_EXECUTE_DURATION_SECONDS: &str = "backend_skill_execute_duration_seconds";
const BACKEND_DEPENDENCY_REQUESTS_TOTAL: &str = "backend_dependency_requests_total";
const BACKEND_DEPENDENCY_REQUEST_DURATION_SECONDS: &str =
    "backend_dependency_request_duration_seconds";
const BACKEND_UDP_DISCOVERY_LISTEN_TOTAL: &str = "backend_udp_discovery_listen_total";
const BACKEND_UDP_DISCOVERY_LISTEN_DURATION_SECONDS: &str =
    "backend_udp_discovery_listen_duration_seconds";
const BACKEND_UDP_DISCOVERY_REQUESTS_TOTAL: &str = "backend_udp_discovery_requests_total";
const BACKEND_UDP_DISCOVERY_RESPONSES_TOTAL: &str = "backend_udp_discovery_responses_total";
const BACKEND_AUDIO_CHUNKS_TOTAL: &str = "backend_audio_chunks_total";
const BACKEND_AUDIO_CHUNK_BYTES_TOTAL: &str = "backend_audio_chunk_bytes_total";
const BACKEND_AUDIO_CHUNK_DURATION_SECONDS: &str = "backend_audio_chunk_duration_seconds";
const BACKEND_STT_FLUSH_DURATION_SECONDS: &str = "backend_stt_flush_duration_seconds";
const BACKEND_AUDIO_FINALIZE_TOTAL: &str = "backend_audio_finalize_total";
const BACKEND_AUDIO_SESSION_TIMEOUT_TOTAL: &str = "backend_audio_session_timeout_total";
const BACKEND_TURN_FIRST_TOKEN_DURATION_SECONDS: &str = "backend_turn_first_token_duration_seconds";
const BACKEND_TURN_PARTIAL_TRANSCRIPT_DURATION_SECONDS: &str =
    "backend_turn_partial_transcript_duration_seconds";
const BACKEND_TURN_SPECULATIVE_RESTARTS_TOTAL: &str = "backend_turn_speculative_restarts_total";
const BACKEND_TURN_CANCELLATIONS_TOTAL: &str = "backend_turn_cancellations_total";
const BACKEND_LLM_PROVIDER_DURATION_SECONDS: &str = "backend_llm_provider_duration_seconds";
const FRONTEND_RPC_DURATION_SECONDS: &str = "frontend_rpc_duration_seconds";
const FRONTEND_SKILL_DURATION_SECONDS: &str = "frontend_skill_duration_seconds";
const FRONTEND_TTS_PLAYBACK_DURATION_SECONDS: &str = "frontend_tts_playback_duration_seconds";
const PALACE_OPEN_TOTAL: &str = "palace_open_total";
const PALACE_OPEN_DURATION_SECONDS: &str = "palace_open_duration_seconds";
const PALACE_WAKE_UP_TOTAL: &str = "palace_wake_up_total";
const PALACE_WAKE_UP_DURATION_SECONDS: &str = "palace_wake_up_duration_seconds";
const PALACE_SEARCH_TOTAL: &str = "palace_search_total";
const PALACE_SEARCH_DURATION_SECONDS: &str = "palace_search_duration_seconds";
const PALACE_INGEST_TOTAL: &str = "palace_ingest_total";
const PALACE_INGEST_DURATION_SECONDS: &str = "palace_ingest_duration_seconds";
const PALACE_ADD_MEMORY_TOTAL: &str = "palace_add_memory_total";
const PALACE_ADD_MEMORY_DURATION_SECONDS: &str = "palace_add_memory_duration_seconds";
const PALACE_KG_QUERY_TOTAL: &str = "palace_kg_query_total";
const PALACE_KG_QUERY_DURATION_SECONDS: &str = "palace_kg_query_duration_seconds";
const PALACE_KG_ADD_TOTAL: &str = "palace_kg_add_total";
const PALACE_KG_ADD_DURATION_SECONDS: &str = "palace_kg_add_duration_seconds";
const PALACE_ERRORS_TOTAL: &str = "palace_errors_total";

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
    counter!(VOICE_MEDIA_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_COMPUTER_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_CALCULATOR_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_UNIT_CONVERSION_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_CURRENCY_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_AIR_QUALITY_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_DICTIONARY_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_TRANSLATE_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_CALENDAR_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_MEETING_NOTES_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_EMAIL_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_BRIEFING_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_JOURNAL_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_SCREEN_OCR_SKILL_TOTAL, 0, "result" => "unknown");
    counter!(VOICE_NEWS_SUMMARY_CHUNK_TOTAL, 0, "result" => "unknown");
    histogram!(VOICE_NEWS_SUMMARY_DURATION_SECONDS, 0.0_f64);
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
        VOICE_MODEL_PRELOAD_TOTAL,
        0,
        "component" => "unknown",
        "result" => "unknown"
    );
    histogram!(
        VOICE_MODEL_PRELOAD_DURATION_SECONDS,
        0.0_f64,
        "component" => "unknown"
    );
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
    counter!(BACKEND_TURN_TOTAL, 0, "path" => "unknown", "result" => "unknown");
    histogram!(BACKEND_TURN_DURATION_SECONDS, 0.0_f64, "path" => "unknown");
    histogram!(
        BACKEND_TURN_STAGE_DURATION_SECONDS,
        0.0_f64,
        "stage" => "unknown"
    );
    counter!(
        BACKEND_HTTP_REQUESTS_TOTAL,
        0,
        "route" => "unknown",
        "method" => "unknown",
        "result" => "unknown",
        "status_class" => "unknown"
    );
    histogram!(
        BACKEND_HTTP_REQUEST_DURATION_SECONDS,
        0.0_f64,
        "route" => "unknown",
        "method" => "unknown"
    );
    counter!(
        BACKEND_SKILL_EXECUTE_TOTAL,
        0,
        "skill" => "unknown",
        "result" => "unknown",
        "error_kind" => "none"
    );
    histogram!(
        BACKEND_SKILL_EXECUTE_DURATION_SECONDS,
        0.0_f64,
        "skill" => "unknown"
    );
    counter!(
        BACKEND_DEPENDENCY_REQUESTS_TOTAL,
        0,
        "dependency" => "unknown",
        "operation" => "unknown",
        "result" => "unknown",
        "error_kind" => "none"
    );
    histogram!(
        BACKEND_DEPENDENCY_REQUEST_DURATION_SECONDS,
        0.0_f64,
        "dependency" => "unknown",
        "operation" => "unknown"
    );
    counter!(BACKEND_UDP_DISCOVERY_LISTEN_TOTAL, 0, "result" => "unknown");
    histogram!(BACKEND_UDP_DISCOVERY_LISTEN_DURATION_SECONDS, 0.0_f64);
    counter!(BACKEND_UDP_DISCOVERY_REQUESTS_TOTAL, 0);
    counter!(BACKEND_UDP_DISCOVERY_RESPONSES_TOTAL, 0, "result" => "unknown");
    counter!(BACKEND_AUDIO_CHUNKS_TOTAL, 0);
    counter!(BACKEND_AUDIO_CHUNK_BYTES_TOTAL, 0);
    histogram!(BACKEND_AUDIO_CHUNK_DURATION_SECONDS, 0.0_f64);
    histogram!(BACKEND_STT_FLUSH_DURATION_SECONDS, 0.0_f64);
    counter!(BACKEND_AUDIO_FINALIZE_TOTAL, 0, "status" => "unknown");
    counter!(BACKEND_AUDIO_SESSION_TIMEOUT_TOTAL, 0);
    histogram!(BACKEND_TURN_FIRST_TOKEN_DURATION_SECONDS, 0.0_f64);
    histogram!(BACKEND_TURN_PARTIAL_TRANSCRIPT_DURATION_SECONDS, 0.0_f64);
    counter!(BACKEND_TURN_SPECULATIVE_RESTARTS_TOTAL, 0);
    counter!(BACKEND_TURN_CANCELLATIONS_TOTAL, 0, "reason" => "unknown");
    histogram!(
        BACKEND_LLM_PROVIDER_DURATION_SECONDS,
        0.0_f64,
        "provider" => "unknown"
    );
    histogram!(FRONTEND_RPC_DURATION_SECONDS, 0.0_f64, "endpoint" => "unknown");
    histogram!(FRONTEND_SKILL_DURATION_SECONDS, 0.0_f64, "skill" => "unknown");
    histogram!(FRONTEND_TTS_PLAYBACK_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_OPEN_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_OPEN_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_WAKE_UP_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_WAKE_UP_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_SEARCH_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_SEARCH_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_INGEST_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_INGEST_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_ADD_MEMORY_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_ADD_MEMORY_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_KG_QUERY_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_KG_QUERY_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_KG_ADD_TOTAL, 0, "result" => "unknown");
    histogram!(PALACE_KG_ADD_DURATION_SECONDS, 0.0_f64);
    counter!(PALACE_ERRORS_TOTAL, 0, "operation" => "unknown");
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

pub fn record_backend_turn_total(path: &str, result: &str) {
    counter!(
        BACKEND_TURN_TOTAL,
        1,
        "path" => path.to_string(),
        "result" => result.to_string()
    );
}

pub fn record_backend_turn_duration(path: &str, duration: Duration) {
    histogram!(
        BACKEND_TURN_DURATION_SECONDS,
        duration.as_secs_f64(),
        "path" => path.to_string()
    );
}

pub fn record_backend_turn_stage_duration(stage: &str, duration: Duration) {
    histogram!(
        BACKEND_TURN_STAGE_DURATION_SECONDS,
        duration.as_secs_f64(),
        "stage" => stage.to_string()
    );
}

pub fn record_backend_http_request(
    route: &str,
    method: &str,
    status_code: u16,
    duration: Duration,
) {
    let status_class = format!("{}xx", status_code / 100);
    let result = if status_code < 400 {
        "success"
    } else {
        "error"
    };
    counter!(
        BACKEND_HTTP_REQUESTS_TOTAL,
        1,
        "route" => route.to_string(),
        "method" => method.to_string(),
        "result" => result.to_string(),
        "status_class" => status_class
    );
    histogram!(
        BACKEND_HTTP_REQUEST_DURATION_SECONDS,
        duration.as_secs_f64(),
        "route" => route.to_string(),
        "method" => method.to_string()
    );
}

pub fn record_backend_skill_execute(skill: &str, result: &str, error_kind: Option<&str>) {
    counter!(
        BACKEND_SKILL_EXECUTE_TOTAL,
        1,
        "skill" => skill.to_string(),
        "result" => result.to_string(),
        "error_kind" => error_kind.unwrap_or("none").to_string()
    );
}

pub fn record_backend_skill_execute_duration(skill: &str, duration: Duration) {
    histogram!(
        BACKEND_SKILL_EXECUTE_DURATION_SECONDS,
        duration.as_secs_f64(),
        "skill" => skill.to_string()
    );
}

pub fn record_backend_dependency_request(
    dependency: &str,
    operation: &str,
    result: &str,
    error_kind: Option<&str>,
) {
    counter!(
        BACKEND_DEPENDENCY_REQUESTS_TOTAL,
        1,
        "dependency" => dependency.to_string(),
        "operation" => operation.to_string(),
        "result" => result.to_string(),
        "error_kind" => error_kind.unwrap_or("none").to_string()
    );
}

pub fn record_backend_dependency_request_duration(
    dependency: &str,
    operation: &str,
    duration: Duration,
) {
    histogram!(
        BACKEND_DEPENDENCY_REQUEST_DURATION_SECONDS,
        duration.as_secs_f64(),
        "dependency" => dependency.to_string(),
        "operation" => operation.to_string()
    );
}

pub fn record_backend_udp_discovery_listen_total(result: &str) {
    counter!(
        BACKEND_UDP_DISCOVERY_LISTEN_TOTAL,
        1,
        "result" => result.to_string()
    );
}

pub fn record_backend_udp_discovery_listen_duration(duration: Duration) {
    histogram!(
        BACKEND_UDP_DISCOVERY_LISTEN_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

pub fn record_backend_udp_discovery_request_total() {
    counter!(BACKEND_UDP_DISCOVERY_REQUESTS_TOTAL, 1);
}

pub fn record_backend_udp_discovery_response_total(result: &str) {
    counter!(
        BACKEND_UDP_DISCOVERY_RESPONSES_TOTAL,
        1,
        "result" => result.to_string()
    );
}

pub fn record_backend_audio_chunk(bytes: usize, duration: Duration) {
    counter!(BACKEND_AUDIO_CHUNKS_TOTAL, 1);
    counter!(BACKEND_AUDIO_CHUNK_BYTES_TOTAL, bytes as u64);
    histogram!(BACKEND_AUDIO_CHUNK_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_backend_stt_flush_duration(duration: Duration) {
    histogram!(BACKEND_STT_FLUSH_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_backend_audio_finalize(status: &str) {
    counter!(
        BACKEND_AUDIO_FINALIZE_TOTAL,
        1,
        "status" => status.to_string()
    );
}

pub fn record_backend_audio_session_timeout() {
    counter!(BACKEND_AUDIO_SESSION_TIMEOUT_TOTAL, 1);
}

pub fn record_backend_turn_first_token_duration(duration: Duration) {
    histogram!(
        BACKEND_TURN_FIRST_TOKEN_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

pub fn record_backend_turn_partial_transcript_duration(duration: Duration) {
    histogram!(
        BACKEND_TURN_PARTIAL_TRANSCRIPT_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

pub fn record_backend_turn_speculative_restart() {
    counter!(BACKEND_TURN_SPECULATIVE_RESTARTS_TOTAL, 1);
}

pub fn record_backend_turn_cancellation(reason: &str) {
    counter!(
        BACKEND_TURN_CANCELLATIONS_TOTAL,
        1,
        "reason" => reason.to_string()
    );
}

pub fn record_backend_llm_provider_duration(provider: &str, duration: Duration) {
    histogram!(
        BACKEND_LLM_PROVIDER_DURATION_SECONDS,
        duration.as_secs_f64(),
        "provider" => provider.to_string()
    );
}

pub fn record_frontend_rpc_duration(endpoint: &str, duration: Duration) {
    histogram!(
        FRONTEND_RPC_DURATION_SECONDS,
        duration.as_secs_f64(),
        "endpoint" => endpoint.to_string()
    );
}

pub fn record_frontend_skill_duration(skill: &str, duration: Duration) {
    histogram!(
        FRONTEND_SKILL_DURATION_SECONDS,
        duration.as_secs_f64(),
        "skill" => skill.to_string()
    );
}

pub fn record_frontend_tts_playback_duration(duration: Duration) {
    histogram!(
        FRONTEND_TTS_PLAYBACK_DURATION_SECONDS,
        duration.as_secs_f64()
    );
}

/// Record an intent classification call.
pub fn record_intent_classifier() {
    counter!(VOICE_INTENT_CLASSIFIER_TOTAL, 1);
}

/// Record an intent decision rejected by contract validation (invalid action for the chosen skill).
pub fn record_intent_validation_rejected(skill: &str) {
    counter!(VOICE_INTENT_VALIDATION_REJECTED_TOTAL, 1, "skill" => skill.to_string());
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

/// Record media skill result (success or error).
pub fn record_media_skill(result: &str) {
    counter!(VOICE_MEDIA_SKILL_TOTAL, 1, "result" => result.to_string());
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

/// Record calculator skill result (success or error).
pub fn record_calculator_skill(result: &str) {
    counter!(VOICE_CALCULATOR_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record unit conversion skill result (success or error).
pub fn record_unit_conversion_skill(result: &str) {
    counter!(
        VOICE_UNIT_CONVERSION_SKILL_TOTAL,
        1,
        "result" => result.to_string()
    );
}

/// Record currency skill result (success or error).
pub fn record_currency_skill(result: &str) {
    counter!(VOICE_CURRENCY_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record air quality skill result (success or error).
pub fn record_air_quality_skill(result: &str) {
    counter!(
        VOICE_AIR_QUALITY_SKILL_TOTAL,
        1,
        "result" => result.to_string()
    );
}

/// Record dictionary skill result (success or error).
pub fn record_dictionary_skill(result: &str) {
    counter!(VOICE_DICTIONARY_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record translate skill result (success or error).
pub fn record_translate_skill(result: &str) {
    counter!(VOICE_TRANSLATE_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record calendar skill outcome.
///
/// For frontend-dispatched skills, `result` is one of `dispatched`, `not_supported`,
/// `result_ok`, `result_error`.
pub fn record_calendar_skill(result: &str) {
    counter!(VOICE_CALENDAR_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record meeting notes skill result (success or error).
pub fn record_meeting_notes_skill(result: &str) {
    counter!(
        VOICE_MEETING_NOTES_SKILL_TOTAL,
        1,
        "result" => result.to_string()
    );
}

/// Record email skill outcome.
///
/// For frontend-dispatched skills, `result` is one of `dispatched`, `not_supported`,
/// `result_ok`, `result_error`.
pub fn record_email_skill(result: &str) {
    counter!(VOICE_EMAIL_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record briefing skill result (success or error).
pub fn record_briefing_skill(result: &str) {
    counter!(VOICE_BRIEFING_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record journal skill result (success or error).
pub fn record_journal_skill(result: &str) {
    counter!(VOICE_JOURNAL_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record screen OCR skill outcome.
///
/// `result` covers both the dispatch path (`dispatched`, `not_supported`) and the
/// finalize path (`result_ok`, `result_error`, `parse_error`, `decode_error`).
pub fn record_screen_ocr_skill(result: &str) {
    counter!(VOICE_SCREEN_OCR_SKILL_TOTAL, 1, "result" => result.to_string());
}

/// Record one streamed news-summary chunk outcome (`ok` or `error`).
pub fn record_news_summary_chunk(result: &str) {
    counter!(
        VOICE_NEWS_SUMMARY_CHUNK_TOTAL,
        1,
        "result" => result.to_string()
    );
}

/// Record total wall-clock duration of a news summary stream.
pub fn record_news_summary_duration(duration: Duration) {
    histogram!(VOICE_NEWS_SUMMARY_DURATION_SECONDS, duration.as_secs_f64());
}

/// Record policy denial (reason: e.g. "emergency_stop", "budget_exhausted").
pub fn record_policy_denied(reason: &str) {
    counter!(VOICE_POLICY_DENIED_TOTAL, 1, "reason" => reason.to_string());
}

/// Record startup location preload result (success or failure).
pub fn record_location_preload(result: &str) {
    counter!(VOICE_LOCATION_PRELOAD_TOTAL, 1, "result" => result.to_string());
}

/// Record startup model preload result by component (`stt` or `llm`).
pub fn record_model_preload(component: &str, result: &str) {
    counter!(
        VOICE_MODEL_PRELOAD_TOTAL,
        1,
        "component" => component.to_string(),
        "result" => result.to_string()
    );
}

/// Record startup model preload latency by component (`stt` or `llm`).
pub fn record_model_preload_duration(component: &str, duration: Duration) {
    histogram!(
        VOICE_MODEL_PRELOAD_DURATION_SECONDS,
        duration.as_secs_f64(),
        "component" => component.to_string()
    );
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

pub fn record_palace_open(result: &str, duration: Duration) {
    counter!(PALACE_OPEN_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_OPEN_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_wake_up(result: &str, duration: Duration) {
    counter!(PALACE_WAKE_UP_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_WAKE_UP_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_search(result: &str, duration: Duration) {
    counter!(PALACE_SEARCH_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_SEARCH_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_ingest(result: &str, duration: Duration) {
    counter!(PALACE_INGEST_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_INGEST_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_add_memory(result: &str, duration: Duration) {
    counter!(PALACE_ADD_MEMORY_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_ADD_MEMORY_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_kg_query(result: &str, duration: Duration) {
    counter!(PALACE_KG_QUERY_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_KG_QUERY_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_kg_add(result: &str, duration: Duration) {
    counter!(PALACE_KG_ADD_TOTAL, 1, "result" => result.to_string());
    histogram!(PALACE_KG_ADD_DURATION_SECONDS, duration.as_secs_f64());
}

pub fn record_palace_error(operation: &str) {
    counter!(PALACE_ERRORS_TOTAL, 1, "operation" => operation.to_string());
}
