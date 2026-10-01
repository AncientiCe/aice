use aice_backend::{
    spawn_server_with_audio, AudioIngressConfig, AudioTranscriber, BackendEngine,
    BackendEngineDecision,
};
use async_trait::async_trait;
use core_observability::{
    init_prometheus_exporter, record_backend_http_request, record_backend_llm_provider_duration,
    record_backend_turn_cancellation, record_backend_turn_first_token_duration,
    record_backend_turn_partial_transcript_duration, record_backend_turn_speculative_restart,
    record_backend_turn_stage_duration, register_metrics, set_slow_turn_threshold,
    ExporterInitState,
};
use core_runtime_protocol::{
    FrontendSkillResultRequest, TurnRequest, TurnStreamClientMessage, TurnStreamServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

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
    stream.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
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

fn wait_for_metrics(bind: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_payload = String::new();
    loop {
        if let Ok(payload) = scrape(bind) {
            last_payload = payload.clone();
            if payload.contains(needle) {
                return payload;
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "metrics payload did not contain '{needle}' before timeout; last payload:\n{last_payload}"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn observability_json_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../ops/observability/grafana/provisioning/dashboards/json")
}

#[test]
fn backend_http_metrics_cover_core_routes_and_errors() {
    let metrics_bind = shared_bind().unwrap_or_else(|error| panic!("reserve bind failed: {error}"));
    let state = init_prometheus_exporter(metrics_bind)
        .unwrap_or_else(|error| panic!("failed to initialize exporter: {error}"));
    assert!(
        matches!(
            state,
            ExporterInitState::Started | ExporterInitState::AlreadyRunning
        ),
        "exporter should be available for metrics assertions"
    );
    register_metrics();

    record_backend_http_request("/healthz", "GET", 200, Duration::from_millis(3));
    record_backend_http_request("/turns/stream", "GET", 101, Duration::from_millis(1));
    record_backend_http_request("not_found", "GET", 404, Duration::from_millis(2));

    let payload = wait_for_metrics(metrics_bind, "backend_http_request_duration_seconds");
    assert!(payload.contains("backend_http_request_duration_seconds"));
    assert!(payload.contains("route=\"/healthz\""));
    assert!(payload.contains("route=\"/turns/stream\""));
    assert!(payload.contains("route=\"not_found\""));
    assert!(payload.contains("status_class=\"4xx\""));
    assert!(payload.contains("status_class=\"2xx\""));
}

#[test]
fn backend_turn_stage_metrics_cover_latency_breakdown_stages() {
    let metrics_bind = shared_bind().unwrap_or_else(|error| panic!("reserve bind failed: {error}"));
    let state = init_prometheus_exporter(metrics_bind)
        .unwrap_or_else(|error| panic!("failed to initialize exporter: {error}"));
    assert!(
        matches!(
            state,
            ExporterInitState::Started | ExporterInitState::AlreadyRunning
        ),
        "exporter should be available for metrics assertions"
    );
    register_metrics();

    record_backend_turn_stage_duration("decode_request", Duration::from_millis(2));
    record_backend_turn_stage_duration("merge_chunks", Duration::from_millis(1));
    record_backend_turn_stage_duration("capability_resolution", Duration::from_millis(1));
    record_backend_turn_stage_duration("classifier_prompt_build", Duration::from_millis(3));
    record_backend_turn_stage_duration("classifier_llm_roundtrip", Duration::from_millis(15));
    record_backend_turn_stage_duration("intent_parse_validate", Duration::from_millis(1));
    record_backend_turn_stage_duration("response_serialize", Duration::from_millis(2));

    let payload = wait_for_metrics(metrics_bind, "backend_turn_stage_duration_seconds");
    assert!(payload.contains("stage=\"decode_request\""));
    assert!(payload.contains("stage=\"merge_chunks\""));
    assert!(payload.contains("stage=\"capability_resolution\""));
    assert!(payload.contains("stage=\"classifier_prompt_build\""));
    assert!(payload.contains("stage=\"classifier_llm_roundtrip\""));
    assert!(payload.contains("stage=\"intent_parse_validate\""));
    assert!(payload.contains("stage=\"response_serialize\""));
}

#[test]
fn backend_streaming_latency_metrics_are_emitted() {
    let metrics_bind = shared_bind().unwrap_or_else(|error| panic!("reserve bind failed: {error}"));
    let state = init_prometheus_exporter(metrics_bind)
        .unwrap_or_else(|error| panic!("failed to initialize exporter: {error}"));
    assert!(
        matches!(
            state,
            ExporterInitState::Started | ExporterInitState::AlreadyRunning
        ),
        "exporter should be available for metrics assertions"
    );
    register_metrics();

    record_backend_turn_partial_transcript_duration(Duration::from_millis(40));
    record_backend_turn_first_token_duration(Duration::from_millis(95));
    record_backend_turn_speculative_restart();
    record_backend_turn_cancellation("transcript_divergence");
    record_backend_llm_provider_duration("cradle", Duration::from_millis(12));

    let payload = wait_for_metrics(metrics_bind, "backend_turn_first_token_duration_seconds");
    assert!(payload.contains("backend_turn_first_token_duration_seconds"));
    assert!(payload.contains("backend_turn_partial_transcript_duration_seconds"));
    assert!(payload.contains("backend_turn_speculative_restarts_total"));
    assert!(payload.contains("backend_turn_cancellations_total"));
    assert!(payload.contains("backend_llm_provider_duration_seconds"));
    assert!(payload.contains("provider=\"cradle\""));
}

#[test]
fn dashboard_pack_is_backend_only() {
    let dir = observability_json_dir();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("failed to read dashboard dir {dir:?}: {error}"));

    let mut uids = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read entry: {error}"));
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read dashboard file {path:?}: {error}"));
        let value: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|error| panic!("invalid dashboard JSON in {path:?}: {error}"));
        if let Some(uid) = value.get("uid").and_then(|v| v.as_str()) {
            uids.push(uid.to_string());
        }
    }

    assert!(
        !dir.join("frontend-timings.json").exists(),
        "frontend dashboard file must be removed"
    );
    assert!(
        !uids.iter().any(|uid| uid == "aice-frontend-timings"),
        "frontend dashboard UID must be removed"
    );
}

struct SlowClassifierEngine;

#[async_trait]
impl BackendEngine for SlowClassifierEngine {
    async fn process_turn(
        &self,
        _request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        let started = Instant::now();
        tokio::time::sleep(Duration::from_millis(80)).await;
        record_backend_turn_stage_duration("classifier_llm_roundtrip", started.elapsed());
        Ok(BackendEngineDecision::Chat("ok".to_string()))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("done".to_string())
    }
}

struct HelloTranscriber;

#[async_trait]
impl AudioTranscriber for HelloTranscriber {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("hello".to_string())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_backend_turn_is_counted_with_its_slowest_stage() {
    let metrics_bind = shared_bind().unwrap_or_else(|error| panic!("reserve bind failed: {error}"));
    let _ = init_prometheus_exporter(metrics_bind)
        .unwrap_or_else(|error| panic!("failed to initialize exporter: {error}"));
    register_metrics();
    set_slow_turn_threshold(Duration::from_millis(20));

    let engine: Arc<dyn BackendEngine> = Arc::new(SlowClassifierEngine);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(HelloTranscriber);
    let handle = spawn_server_with_audio(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("spawn failed: {error}"));
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/turns/stream", handle.bind))
        .await
        .unwrap_or_else(|error| panic!("ws connect failed: {error}"));
    let (mut write, mut read) = ws.split();
    let send = |message: &TurnStreamClientMessage| {
        Message::Text(
            serde_json::to_string(message).unwrap_or_else(|error| panic!("encode failed: {error}")),
        )
    };
    write
        .send(send(&TurnStreamClientMessage::TurnStart {
            session_id: "s-trace".to_string(),
            device_id: Some("device-trace".to_string()),
            turn_id: "turn-trace-1".to_string(),
            supported_frontend_intents: vec![],
            schema_version: None,
        }))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));
    write
        .send(Message::Binary(vec![1, 0, 2, 0, 3, 0]))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));
    write
        .send(send(&TurnStreamClientMessage::TurnDone))
        .await
        .unwrap_or_else(|error| panic!("send failed: {error}"));

    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), read.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for the turn to finish"));
        let Some(Ok(Message::Text(text))) = message else {
            continue;
        };
        if let Ok(TurnStreamServerEvent::Done { .. }) = serde_json::from_str(&text) {
            break;
        }
    }

    let payload = wait_for_metrics(metrics_bind, "voice_slow_turns_total");
    assert!(payload.contains("component=\"backend\""));
    assert!(payload.contains("slowest_stage=\"classifier_llm_roundtrip\""));
    assert!(payload.contains("voice_turn_unaccounted_duration_seconds"));
    handle.shutdown().await;
}

#[test]
fn guest_journey_dashboard_shows_where_a_slow_turn_lost_its_time() {
    let path = observability_json_dir().join("guest-journey.json");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read dashboard {path:?}: {error}"));
    let dashboard: serde_json::Value = serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("invalid dashboard JSON: {error}"));
    assert_eq!(dashboard["uid"], "aice-guest-journey");

    let mut expressions = String::new();
    for panel in dashboard["panels"].as_array().cloned().unwrap_or_default() {
        for target in panel["targets"].as_array().cloned().unwrap_or_default() {
            expressions.push_str(target["expr"].as_str().unwrap_or(""));
            expressions.push(' ');
        }
    }
    for metric in [
        "voice_turn_time_to_first_audio_seconds",
        "pod_bridge_backend_wait_seconds",
        "pod_bridge_synthesis_duration_seconds",
        "backend_turn_stage_duration_seconds",
        "voice_turn_unaccounted_duration_seconds",
        "voice_slow_turns_total",
        "property_ticket_ack_duration_seconds",
        "property_ticket_resolve_duration_seconds",
        "property_sla_breaches_total",
    ] {
        assert!(
            expressions.contains(metric),
            "guest journey dashboard has no panel for {metric}"
        );
    }

    let prometheus = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../ops/observability/prometheus/prometheus.yml"),
    )
    .unwrap_or_else(|error| panic!("failed to read prometheus config: {error}"));
    for job in ["aice-backend", "pod-gateway", "aice-hotels"] {
        assert!(
            prometheus.contains(&format!("job_name: {job}")),
            "prometheus does not scrape {job}"
        );
    }
}
