//! Admission control: bounded concurrent turns and STT jobs, with a spoken
//! busy reply instead of an unbounded wait.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aice_backend::{
    AdmissionSettings, AdmittedEngine, AdmittedTranscriber, AudioTranscriber, BackendEngine,
    BackendEngineDecision, BUSY_REPLY,
};
use async_trait::async_trait;
use core_runtime_protocol::{FrontendSkillResultRequest, TurnRequest};

/// Explicit test double: takes `work` per turn and records peak concurrency.
struct SlowEngine {
    work: Duration,
    running: AtomicUsize,
    peak: AtomicUsize,
}

impl SlowEngine {
    fn new(work: Duration) -> Self {
        Self {
            work,
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    async fn busy(&self) {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.work).await;
        self.running.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl BackendEngine for SlowEngine {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        self.busy().await;
        Ok(BackendEngineDecision::Chat(request.transcript))
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        _intent_id: &str,
        _request: FrontendSkillResultRequest,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.busy().await;
        Ok("done".to_string())
    }
}

#[async_trait]
impl AudioTranscriber for SlowEngine {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.busy().await;
        Ok("heard".to_string())
    }
}

fn request(text: &str) -> TurnRequest {
    TurnRequest {
        session_id: "s".to_string(),
        device_id: None,
        turn_id: None,
        transcript: text.to_string(),
        finalize: true,
        context: None,
    }
}

fn settings(turns: usize, stt: usize, max_wait: Duration) -> AdmissionSettings {
    AdmissionSettings {
        max_concurrent_turns: turns,
        max_concurrent_stt: stt,
        max_wait,
    }
}

#[tokio::test]
async fn turns_beyond_the_limit_wait_their_turn() {
    let slow = Arc::new(SlowEngine::new(Duration::from_millis(100)));
    let inner: Arc<dyn BackendEngine> = slow.clone();
    let engine = Arc::new(AdmittedEngine::new(
        inner,
        &settings(2, 1, Duration::from_secs(5)),
    ));
    let mut tasks = Vec::new();
    for index in 0..6 {
        let engine = Arc::clone(&engine);
        tasks.push(tokio::spawn(async move {
            engine.process_turn(request(&format!("room {index}"))).await
        }));
    }
    for task in tasks {
        let decision = task
            .await
            .unwrap_or_else(|error| panic!("join: {error}"))
            .unwrap_or_else(|error| panic!("turn: {error}"));
        assert!(matches!(decision, BackendEngineDecision::Chat(text) if text.starts_with("room")));
    }
    assert_eq!(slow.peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_turn_that_waits_too_long_gets_a_busy_reply() {
    let slow = Arc::new(SlowEngine::new(Duration::from_millis(500)));
    let inner: Arc<dyn BackendEngine> = slow.clone();
    let engine = Arc::new(AdmittedEngine::new(
        inner,
        &settings(1, 1, Duration::from_millis(100)),
    ));
    let first = {
        let engine = Arc::clone(&engine);
        tokio::spawn(async move { engine.process_turn(request("first")).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = engine
        .process_turn(request("second"))
        .await
        .unwrap_or_else(|error| panic!("turn: {error}"));
    assert!(matches!(second, BackendEngineDecision::Chat(text) if text == BUSY_REPLY));
    let first = first
        .await
        .unwrap_or_else(|error| panic!("join: {error}"))
        .unwrap_or_else(|error| panic!("turn: {error}"));
    assert!(matches!(first, BackendEngineDecision::Chat(text) if text == "first"));
}

#[tokio::test]
async fn stt_jobs_are_bounded_and_time_out() {
    let slow = Arc::new(SlowEngine::new(Duration::from_millis(300)));
    let inner: Arc<dyn AudioTranscriber> = slow.clone();
    let stt = Arc::new(AdmittedTranscriber::new(
        inner,
        &settings(4, 1, Duration::from_millis(100)),
    ));
    let first = {
        let stt = Arc::clone(&stt);
        tokio::spawn(async move { stt.transcribe(vec![0; 16], 16_000, 1).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        stt.transcribe(vec![0; 16], 16_000, 1).await.is_err(),
        "second STT job times out while the only worker is busy"
    );
    let heard = first
        .await
        .unwrap_or_else(|error| panic!("join: {error}"))
        .unwrap_or_else(|error| panic!("stt: {error}"));
    assert_eq!(heard, "heard");
    assert_eq!(slow.peak.load(Ordering::SeqCst), 1);
}
