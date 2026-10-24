//! Capacity gate: many simulated rooms through the real bridge, backend
//! (with admission control), and facilitator. Every turn must be answered.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aice_backend::{
    spawn_server_with_options, AdmissionSettings, AdmittedEngine, AudioIngressConfig,
    AudioTranscriber, BackendEngine, BackendEngineDecision, DeviceAuth, ServerOptions,
};
use async_trait::async_trait;
use core_runtime_protocol::{FrontendSkillResultRequest, TurnRequest};
use pod_gateway::{spawn_bridge, BridgeSettings, Speech, VadSettings};
use property_facilitator::{serve, Pack, PropertyClient, Settings};
use room_loadtest::{provision_pods, run_load, speech_like_pcm, LoadSettings};

const SERVICE_TOKEN: &str = "loadtest-service-token-0123456789-abc";

/// Explicit test double: 150 ms of "thinking" per turn, peak concurrency recorded.
struct Thinking {
    running: AtomicUsize,
    peak: AtomicUsize,
}

#[async_trait]
impl BackendEngine for Thinking {
    async fn process_turn(
        &self,
        _request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(BackendEngineDecision::Chat("Right away.".to_string()))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(String::new())
    }
}

struct Heard;

#[async_trait]
impl AudioTranscriber for Heard {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("please send towels".to_string())
    }
}

/// Explicit test double: 0.25 s of audio per answer.
struct QuarterSecond;

impl Speech for QuarterSecond {
    fn synthesize(&self, _text: &str) -> Result<Vec<u8>, String> {
        Ok(vec![0u8; 8_000])
    }
}

fn temp_db() -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!(
        "aice-loadtest-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_rooms_are_all_answered_with_two_turn_workers() {
    let db = temp_db();
    let mut settings = Settings::default_for(Pack::Hotels);
    settings.bind = "127.0.0.1:0".to_string();
    settings.database_path = db.clone();
    settings.service_token = SERVICE_TOKEN.to_string();
    let facilitator = serve(settings)
        .await
        .unwrap_or_else(|error| panic!("facilitator: {error}"));
    let tokens = provision_pods(&facilitator.url, None, &db, "load", 8)
        .await
        .unwrap_or_else(|error| panic!("provision: {error}"));
    assert_eq!(tokens.len(), 8);

    let thinking = Arc::new(Thinking {
        running: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });
    let inner: Arc<dyn BackendEngine> = thinking.clone();
    let admission = AdmissionSettings {
        max_concurrent_turns: 2,
        max_concurrent_stt: 2,
        max_wait: Duration::from_secs(10),
    };
    let engine: Arc<dyn BackendEngine> = Arc::new(AdmittedEngine::new(inner, &admission));
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(Heard);
    let client = PropertyClient::new(format!("{}/mcp", facilitator.url), SERVICE_TOKEN)
        .unwrap_or_else(|error| panic!("client: {error}"));
    let backend = spawn_server_with_options(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
        ServerOptions {
            device_auth: Some(Arc::new(DeviceAuth::new(client))),
            tls: None,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("backend: {error}"));
    let bridge = spawn_bridge(
        "127.0.0.1:0",
        BridgeSettings {
            backend_url: format!("ws://{}/turns/stream", backend.bind),
            backend_tls: None,
            pod_tls: None,
            vad: VadSettings {
                end_silence: Duration::from_millis(300),
                ..VadSettings::default()
            },
        },
        Arc::new(QuarterSecond),
    )
    .await
    .unwrap_or_else(|error| panic!("bridge: {error}"));

    let report = run_load(&LoadSettings {
        bridge_url: format!("ws://{}/", bridge.bind),
        server_name: None,
        ca_file: None,
        tokens,
        utterance: speech_like_pcm(Duration::from_millis(600)),
        turns_per_room: 2,
        answer_timeout: Duration::from_secs(20),
    })
    .await;

    assert_eq!(report.failures, 0, "{report}");
    assert_eq!(report.answered, 16, "{report}");
    assert!(thinking.peak.load(Ordering::SeqCst) <= 2, "admission held");
    let p95 = report.percentile(95.0).unwrap_or(Duration::MAX);
    assert!(p95 < Duration::from_secs(5), "{report}");
    backend.shutdown().await;
    let _ = std::fs::remove_file(db);
}
