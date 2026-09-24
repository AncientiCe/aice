//! Admission control for a shared backend.
//!
//! Many rooms share one set of LLM and STT workers. Turns and STT jobs take a
//! permit before they run; beyond the limit they queue, and a turn that waits
//! longer than `max_wait` gets a spoken busy reply instead of silence.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use core_observability::{
    record_backend_queue_depth, record_backend_queue_rejection, record_backend_queue_wait,
};
use core_runtime_protocol::{FrontendSkillResultRequest, TurnRequest};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{AudioTranscriber, BackendEngine, BackendEngineDecision};

type DynError = Box<dyn std::error::Error + Send + Sync>;

/// Spoken when a turn could not start within `max_wait`.
pub const BUSY_REPLY: &str = "I'm helping other rooms right now. Please ask again in a moment.";

#[derive(Clone, Debug)]
pub struct AdmissionSettings {
    /// Turns (classification, skills, answer generation) running at once.
    pub max_concurrent_turns: usize,
    /// Speech-to-text jobs running at once.
    pub max_concurrent_stt: usize,
    /// Longest a turn or STT job waits for a permit.
    pub max_wait: Duration,
}

impl AdmissionSettings {
    pub fn from_config(config: &core_config::Config) -> Self {
        Self {
            max_concurrent_turns: config.service.max_concurrent_turns.max(1),
            max_concurrent_stt: config.stt.workers.max(1),
            max_wait: Duration::from_millis(config.service.turn_queue_max_wait_ms.max(1)),
        }
    }
}

/// A counting gate with queue metrics for one stage (`turn` or `stt`).
struct Gate {
    stage: &'static str,
    permits: Arc<Semaphore>,
    capacity: usize,
    max_wait: Duration,
}

impl Gate {
    fn new(stage: &'static str, capacity: usize, max_wait: Duration) -> Self {
        let capacity = capacity.max(1);
        Self {
            stage,
            permits: Arc::new(Semaphore::new(capacity)),
            capacity,
            max_wait,
        }
    }

    async fn enter(&self) -> Option<OwnedSemaphorePermit> {
        let started = Instant::now();
        let waiting = self
            .capacity
            .saturating_sub(self.permits.available_permits());
        record_backend_queue_depth(self.stage, waiting);
        let permit = tokio::time::timeout(self.max_wait, Arc::clone(&self.permits).acquire_owned())
            .await
            .ok()
            .and_then(Result::ok);
        record_backend_queue_wait(self.stage, started.elapsed());
        if permit.is_none() {
            record_backend_queue_rejection(self.stage);
        }
        permit
    }
}

/// A `BackendEngine` that runs at most `max_concurrent_turns` turns at once.
pub struct AdmittedEngine {
    inner: Arc<dyn BackendEngine>,
    gate: Gate,
}

impl AdmittedEngine {
    pub fn new(inner: Arc<dyn BackendEngine>, settings: &AdmissionSettings) -> Self {
        Self {
            inner,
            gate: Gate::new("turn", settings.max_concurrent_turns, settings.max_wait),
        }
    }
}

#[async_trait]
impl BackendEngine for AdmittedEngine {
    async fn process_turn(&self, request: TurnRequest) -> Result<BackendEngineDecision, DynError> {
        let Some(_permit) = self.gate.enter().await else {
            return Ok(BackendEngineDecision::Chat(BUSY_REPLY.to_string()));
        };
        self.inner.process_turn(request).await
    }

    async fn finalize_frontend_skill(
        &self,
        turn_id: &str,
        intent_id: &str,
        request: FrontendSkillResultRequest,
    ) -> Result<String, DynError> {
        let Some(_permit) = self.gate.enter().await else {
            return Ok(BUSY_REPLY.to_string());
        };
        self.inner
            .finalize_frontend_skill(turn_id, intent_id, request)
            .await
    }
}

/// An `AudioTranscriber` that runs at most `max_concurrent_stt` jobs at once.
pub struct AdmittedTranscriber {
    inner: Arc<dyn AudioTranscriber>,
    gate: Gate,
}

impl AdmittedTranscriber {
    pub fn new(inner: Arc<dyn AudioTranscriber>, settings: &AdmissionSettings) -> Self {
        Self {
            inner,
            gate: Gate::new("stt", settings.max_concurrent_stt, settings.max_wait),
        }
    }
}

#[async_trait]
impl AudioTranscriber for AdmittedTranscriber {
    async fn transcribe(
        &self,
        samples: Vec<i16>,
        sample_rate_hz: u32,
        channels: u16,
    ) -> Result<String, DynError> {
        let Some(_permit) = self.gate.enter().await else {
            return Err("speech recognition is busy".into());
        };
        self.inner
            .transcribe(samples, sample_rate_hz, channels)
            .await
    }
}

/// Fixed set of expensive workers (for example Whisper contexts), each used
/// by one job at a time.
pub struct WorkerPool<T> {
    workers: Vec<Arc<std::sync::Mutex<T>>>,
    free: std::sync::Mutex<Vec<usize>>,
    permits: Arc<Semaphore>,
}

impl<T: Send + 'static> WorkerPool<T> {
    pub fn new(workers: Vec<T>) -> Self {
        let count = workers.len();
        Self {
            workers: workers
                .into_iter()
                .map(|worker| Arc::new(std::sync::Mutex::new(worker)))
                .collect(),
            free: std::sync::Mutex::new((0..count).collect()),
            permits: Arc::new(Semaphore::new(count)),
        }
    }

    /// Run `job` on a free worker on the blocking pool.
    pub async fn run<R, F>(&self, job: F) -> Result<R, DynError>
    where
        R: Send + 'static,
        F: FnOnce(&mut T) -> Result<R, DynError> + Send + 'static,
    {
        let _permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|error| -> DynError { error.to_string().into() })?;
        let index = self
            .free
            .lock()
            .map_err(|error| -> DynError { error.to_string().into() })?
            .pop()
            .ok_or_else(|| -> DynError { "worker pool has no free worker".into() })?;
        let worker = Arc::clone(&self.workers[index]);
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = worker
                .lock()
                .map_err(|error| -> DynError { format!("worker poisoned: {error}").into() })?;
            job(&mut guard)
        })
        .await
        .map_err(|error| -> DynError { format!("worker task: {error}").into() })
        .and_then(|value| value);
        if let Ok(mut free) = self.free.lock() {
            free.push(index);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerPool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn the_pool_never_hands_one_worker_to_two_jobs() {
        let pool = Arc::new(WorkerPool::new(vec![0usize, 0usize]));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = Arc::clone(&pool);
            let running = Arc::clone(&running);
            let peak = Arc::clone(&peak);
            tasks.push(tokio::spawn(async move {
                pool.run(move |uses: &mut usize| {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    *uses += 1;
                    running.fetch_sub(1, Ordering::SeqCst);
                    Ok(*uses)
                })
                .await
            }));
        }
        let mut total = 0;
        for task in tasks {
            if let Ok(Ok(_)) = task.await {
                total += 1;
            }
        }
        assert_eq!(total, 8);
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }
}
