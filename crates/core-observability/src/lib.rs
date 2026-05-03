//! JSON logging and metrics for the voice assistant.
//!
//! Provides structured JSON logging and Prometheus-style metrics with
//! baseline telemetry: voice_sessions_total, voice_errors_total, voice_stage_duration_seconds.

mod exporter;
mod log;
mod metrics;

pub use exporter::{init_prometheus_exporter, ExporterInitError, ExporterInitState};
pub use log::init_json_logging;
pub use metrics::{
    record_air_quality_skill, record_app_switcher_skill, record_backend_audio_chunk,
    record_backend_audio_finalize, record_backend_audio_session_timeout,
    record_backend_dependency_request, record_backend_dependency_request_duration,
    record_backend_http_request, record_backend_llm_provider_duration,
    record_backend_skill_execute, record_backend_skill_execute_duration,
    record_backend_stt_flush_duration, record_backend_turn_cancellation,
    record_backend_turn_duration, record_backend_turn_first_token_duration,
    record_backend_turn_partial_transcript_duration, record_backend_turn_speculative_restart,
    record_backend_turn_stage_duration, record_backend_turn_total,
    record_backend_udp_discovery_listen_duration, record_backend_udp_discovery_listen_total,
    record_backend_udp_discovery_request_total, record_backend_udp_discovery_response_total,
    record_briefing_skill, record_calculator_skill, record_calendar_skill,
    record_cancellation_success, record_computer_skill, record_currency_skill,
    record_dictionary_skill, record_distance_skill, record_email_skill,
    record_endpointing_wait_duration, record_error, record_first_audio_latency,
    record_first_token_latency, record_frontend_rpc_duration, record_frontend_skill_duration,
    record_frontend_tts_playback_duration, record_intent_classifier, record_intent_routed,
    record_intent_validation_rejected, record_interruption, record_journal_skill,
    record_llm_first_token_latency, record_llm_stream_tail_duration, record_location_contract,
    record_location_contract_duration, record_location_preload, record_media_execute,
    record_media_execute_duration, record_media_skill, record_meeting_notes_skill,
    record_memory_fact_recall, record_memory_fact_recall_duration, record_memory_fact_store,
    record_memory_fact_store_duration, record_memory_load, record_memory_load_duration,
    record_memory_load_error, record_memory_save, record_memory_save_duration,
    record_memory_save_error, record_message_skill, record_mic_to_stt_duration,
    record_model_preload, record_model_preload_duration, record_news_summary_chunk,
    record_news_summary_duration, record_palace_add_memory, record_palace_error,
    record_palace_ingest, record_palace_kg_add, record_palace_kg_query, record_palace_open,
    record_palace_search, record_palace_wake_up, record_pod_audio_frame, record_pod_connection,
    record_pod_disconnect, record_pod_egress_device_lock_poison, record_pod_egress_queue_drop,
    record_pod_egress_send_error, record_pod_tts_chunk, record_policy_denied,
    record_reminder_skill, record_screen_ocr_skill, record_screenshot_skill, record_session_start,
    record_shopping_list_skill, record_shutdown_signal, record_skill_duration,
    record_smart_home_execute, record_smart_home_execute_duration, record_smart_home_skill,
    record_speech_voiced_duration, record_stage_duration, record_time_skill, record_timer_skill,
    record_translate_skill, record_tts_first_audio_latency, record_tts_flush_duration,
    record_turn_time_to_first_audio, record_unit_conversion_skill, record_volume_skill,
    record_weather_skill, register_metrics, Stage,
};

#[cfg(test)]
mod tests {
    use super::{
        init_prometheus_exporter, record_backend_turn_stage_duration,
        record_llm_stream_tail_duration, record_mic_to_stt_duration, record_session_start,
        record_skill_duration, record_tts_first_audio_latency, record_tts_flush_duration,
        register_metrics, ExporterInitState,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::OnceLock;
    use std::thread;
    use std::time::{Duration, Instant};

    fn shared_bind() -> Result<&'static str, std::io::Error> {
        static BIND: OnceLock<String> = OnceLock::new();
        if let Some(bind) = BIND.get() {
            return Ok(bind.as_str());
        }
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        drop(listener);
        let _ = BIND.set(addr.to_string());
        match BIND.get() {
            Some(bind) => Ok(bind.as_str()),
            None => Err(std::io::Error::other("failed to initialize test bind")),
        }
    }

    fn scrape(bind: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect(bind)?;
        stream.set_read_timeout(Some(Duration::from_millis(250)))?;
        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    #[test]
    fn prometheus_exporter_serves_metrics_text() {
        let bind = match shared_bind() {
            Ok(value) => value,
            Err(error) => panic!("failed to reserve local bind: {error}"),
        };
        let state = match init_prometheus_exporter(bind) {
            Ok(value) => value,
            Err(error) => panic!("failed to initialize exporter: {error}"),
        };
        assert!(state == ExporterInitState::Started || state == ExporterInitState::AlreadyRunning);
        register_metrics();
        record_session_start();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match scrape(bind) {
                Ok(payload) => {
                    if !payload.trim().is_empty() {
                        break;
                    }
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        panic!("failed to scrape metrics endpoint before timeout: {error}");
                    }
                }
            }
            if Instant::now() >= deadline {
                panic!("metrics endpoint did not return payload before timeout");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn prometheus_exporter_is_initialized_once_per_bind() {
        let bind = match shared_bind() {
            Ok(value) => value,
            Err(error) => panic!("failed to reserve local bind: {error}"),
        };
        let _ = match init_prometheus_exporter(bind) {
            Ok(value) => value,
            Err(error) => panic!("failed to initialize exporter: {error}"),
        };
        let state = match init_prometheus_exporter(bind) {
            Ok(value) => value,
            Err(error) => panic!("failed to initialize exporter second time: {error}"),
        };
        assert_eq!(state, ExporterInitState::AlreadyRunning);
    }

    #[test]
    fn new_skill_helpers_emit_without_panic() {
        use super::{
            record_air_quality_skill, record_briefing_skill, record_calculator_skill,
            record_calendar_skill, record_currency_skill, record_dictionary_skill,
            record_email_skill, record_journal_skill, record_meeting_notes_skill,
            record_news_summary_chunk, record_news_summary_duration, record_screen_ocr_skill,
            record_translate_skill, record_unit_conversion_skill, register_metrics,
        };
        register_metrics();
        record_calculator_skill("unknown");
        record_unit_conversion_skill("unknown");
        record_currency_skill("unknown");
        record_air_quality_skill("unknown");
        record_dictionary_skill("unknown");
        record_translate_skill("unknown");
        record_meeting_notes_skill("unknown");
        record_briefing_skill("unknown");
        record_journal_skill("unknown");
        record_calendar_skill("dispatched");
        record_email_skill("dispatched");
        record_screen_ocr_skill("dispatched");
        record_news_summary_chunk("ok");
        record_news_summary_duration(Duration::from_millis(500));
    }

    #[test]
    fn new_turn_timing_metrics_are_emitted() {
        let bind = match shared_bind() {
            Ok(value) => value,
            Err(error) => panic!("failed to reserve local bind: {error}"),
        };
        let _ = match init_prometheus_exporter(bind) {
            Ok(value) => value,
            Err(error) => panic!("failed to initialize exporter: {error}"),
        };
        register_metrics();
        record_mic_to_stt_duration(Duration::from_millis(120));
        record_llm_stream_tail_duration(Duration::from_millis(320));
        record_tts_first_audio_latency(Duration::from_millis(80));
        record_tts_flush_duration(Duration::from_millis(42));
        record_skill_duration("intent_path", Duration::from_millis(50));
        record_backend_turn_stage_duration("classify_intent", Duration::from_millis(20));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match scrape(bind) {
                Ok(payload) => {
                    if !payload.is_empty()
                        && payload.contains("voice_mic_to_stt_duration")
                        && payload.contains("voice_llm_stream_tail_duration")
                        && payload.contains("voice_tts_first_audio_latency")
                        && payload.contains("voice_tts_flush_duration")
                        && payload.contains("voice_skill_duration")
                        && payload.contains("backend_turn_stage_duration")
                    {
                        break;
                    }
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        panic!("failed to scrape metrics endpoint before timeout: {error}");
                    }
                }
            }
            if Instant::now() >= deadline {
                panic!("timing metrics not visible before timeout");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}
