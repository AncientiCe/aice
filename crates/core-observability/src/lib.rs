//! JSON logging and metrics for the voice assistant.
//!
//! Provides structured JSON logging and Prometheus-style metrics with
//! baseline telemetry: voice_sessions_total, voice_errors_total, voice_stage_duration_seconds.

mod log;
mod metrics;

pub use log::init_json_logging;
pub use metrics::{
    record_assistant_skill, record_cancellation_success, record_computer_skill,
    record_distance_skill, record_endpointing_wait_duration, record_error,
    record_first_audio_latency, record_first_token_latency, record_intent_classifier,
    record_intent_routed, record_interruption, record_llm_first_token_latency,
    record_location_preload, record_media_execute, record_media_execute_duration,
    record_media_skill, record_memory_fact_recall, record_memory_fact_recall_duration,
    record_memory_fact_store, record_memory_fact_store_duration, record_memory_load,
    record_memory_load_duration, record_memory_load_error, record_memory_save,
    record_memory_save_duration, record_memory_save_error, record_memory_skill,
    record_message_skill, record_pod_audio_frame, record_pod_connection, record_pod_disconnect,
    record_pod_egress_queue_drop, record_pod_egress_send_error, record_pod_tts_chunk,
    record_policy_denied, record_reminder_skill, record_session_start, record_shopping_list_skill,
    record_shutdown_signal, record_smart_home_execute, record_smart_home_execute_duration,
    record_smart_home_skill, record_speech_voiced_duration, record_stage_duration,
    record_time_skill, record_timer_skill, record_turn_time_to_first_audio, record_weather_skill,
    register_metrics, Stage,
};
