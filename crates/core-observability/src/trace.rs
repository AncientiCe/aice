//! Per-turn tracing: every stage a turn passes through, with when it began and how long it took.
//!
//! A turn is traced by one [`TurnTrace`]. Stage durations already reported through the
//! metrics helpers are copied onto the trace of the turn being processed (see
//! [`record_active_stage`]), so a slow turn can be explained from a single log line that
//! carries its whole timeline, including the time no stage accounts for.

use crate::metrics::{record_slow_turn, record_turn_unaccounted_duration};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tracing::warn;

/// Upper bound on spans kept per turn, so a runaway turn cannot grow without limit.
const MAX_SPANS: usize = 256;

/// A turn this long or longer is logged with its full timeline unless configured otherwise.
const DEFAULT_SLOW_TURN_THRESHOLD_MS: u64 = 3_000;

static SLOW_TURN_THRESHOLD_MS: AtomicU64 = AtomicU64::new(DEFAULT_SLOW_TURN_THRESHOLD_MS);

/// Set how long a turn may take before it is reported as slow.
pub fn set_slow_turn_threshold(threshold: Duration) {
    let millis = u64::try_from(threshold.as_millis()).unwrap_or(u64::MAX);
    SLOW_TURN_THRESHOLD_MS.store(millis, Ordering::Relaxed);
}

/// How long a turn may take before it is reported as slow.
pub fn slow_turn_threshold() -> Duration {
    Duration::from_millis(SLOW_TURN_THRESHOLD_MS.load(Ordering::Relaxed))
}

tokio::task_local! {
    static ACTIVE: TurnTrace;
}

/// One stage of a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceSpan {
    pub stage: String,
    /// When the stage began, measured from the start of the turn.
    pub offset: Duration,
    pub duration: Duration,
}

/// A turn that took longer than the configured threshold.
#[derive(Clone, Debug)]
pub struct SlowTurn {
    pub turn_id: String,
    pub component: &'static str,
    pub total: Duration,
    pub slowest_stage: String,
    /// Part of the turn that no recorded stage covers.
    pub unaccounted: Duration,
    pub spans: Vec<TraceSpan>,
}

impl SlowTurn {
    /// The timeline as a JSON array of `{stage, offset_ms, duration_ms}`.
    pub fn timeline_json(&self) -> String {
        let spans: Vec<serde_json::Value> = self
            .spans
            .iter()
            .map(|span| {
                serde_json::json!({
                    "stage": span.stage,
                    "offset_ms": span.offset.as_millis(),
                    "duration_ms": span.duration.as_millis(),
                })
            })
            .collect();
        serde_json::to_string(&spans).unwrap_or_default()
    }
}

struct Inner {
    component: &'static str,
    turn_id: String,
    started: Instant,
    /// Microseconds after `started` at which the guest stopped speaking.
    speech_end_micros: AtomicU64,
    spans: Mutex<Vec<TraceSpan>>,
}

/// The stages of one turn. Cheap to clone; clones share the same timeline.
#[derive(Clone)]
pub struct TurnTrace {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for TurnTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnTrace")
            .field("component", &self.inner.component)
            .field("turn_id", &self.inner.turn_id)
            .finish()
    }
}

impl TurnTrace {
    /// Start tracing a turn now. `component` names the process (`backend`, `gateway`).
    pub fn new(component: &'static str, turn_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                component,
                turn_id: turn_id.into(),
                started: Instant::now(),
                speech_end_micros: AtomicU64::new(0),
                spans: Mutex::new(Vec::new()),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<TraceSpan>> {
        match self.inner.spans.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Record a stage that has just ended and lasted `duration`.
    pub fn record(&self, stage: &str, duration: Duration) {
        let offset = self.inner.started.elapsed().saturating_sub(duration);
        let mut spans = self.lock();
        if spans.len() < MAX_SPANS {
            spans.push(TraceSpan {
                stage: stage.to_string(),
                offset,
                duration,
            });
        }
    }

    /// The guest has stopped speaking: from here on the turn is the wait for the answer.
    pub fn mark_speech_end(&self) {
        let micros = u64::try_from(self.inner.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.inner
            .speech_end_micros
            .store(micros, Ordering::Relaxed);
    }

    /// The stages recorded so far, in the order they ended.
    pub fn spans(&self) -> Vec<TraceSpan> {
        self.lock().clone()
    }

    /// Run `future` with this trace active: stage durations reported inside it join the trace.
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        ACTIVE.scope(self.clone(), future).await
    }

    /// End the turn. A turn whose answer took `threshold` or longer after the guest stopped
    /// speaking is counted, logged with its full timeline and returned.
    pub fn finish(&self, threshold: Duration) -> Option<SlowTurn> {
        let window_start =
            Duration::from_micros(self.inner.speech_end_micros.load(Ordering::Relaxed));
        let total = self.inner.started.elapsed().saturating_sub(window_start);
        let spans = self.spans();
        let unaccounted = total.saturating_sub(covered(&spans, window_start));
        record_turn_unaccounted_duration(self.inner.component, unaccounted);
        if total < threshold {
            return None;
        }
        let slowest_stage = spans
            .iter()
            .filter(|span| span.offset + span.duration > window_start)
            .max_by_key(|span| span.duration)
            .map(|span| span.stage.clone())
            .unwrap_or_else(|| "none".to_string());
        record_slow_turn(self.inner.component, &slowest_stage);
        let slow = SlowTurn {
            turn_id: self.inner.turn_id.clone(),
            component: self.inner.component,
            total,
            slowest_stage,
            unaccounted,
            spans,
        };
        warn!(
            turn_id = %slow.turn_id,
            component = slow.component,
            total_ms = slow.total.as_millis() as u64,
            slowest_stage = %slow.slowest_stage,
            unaccounted_ms = slow.unaccounted.as_millis() as u64,
            timeline = %slow.timeline_json(),
            "slow_turn"
        );
        Some(slow)
    }

    #[cfg(test)]
    fn poison_for_test(&self) {
        let _guard = self.lock();
        panic!("poison the trace lock");
    }
}

/// Copy a stage onto the trace of the turn being processed, if there is one.
pub fn record_active_stage(stage: &str, duration: Duration) {
    let _ = ACTIVE.try_with(|trace| trace.record(stage, duration));
}

/// Time at or after `from` covered by at least one span; overlapping spans count once.
fn covered(spans: &[TraceSpan], from: Duration) -> Duration {
    let mut intervals: Vec<(Duration, Duration)> = spans
        .iter()
        .map(|span| (span.offset.max(from), span.offset + span.duration))
        .filter(|(start, end)| end > start)
        .collect();
    intervals.sort();
    let mut total = Duration::ZERO;
    let mut current: Option<(Duration, Duration)> = None;
    for (start, end) in intervals {
        match current {
            Some((cur_start, cur_end)) if start <= cur_end => {
                current = Some((cur_start, cur_end.max(end)));
            }
            Some((cur_start, cur_end)) => {
                total += cur_end - cur_start;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((start, end)) = current {
        total += end - start;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::{record_active_stage, set_slow_turn_threshold, slow_turn_threshold, TurnTrace};
    use std::time::Duration;

    #[tokio::test]
    async fn stages_recorded_inside_scope_land_on_the_trace() {
        let trace = TurnTrace::new("backend", "pod-1-7");
        trace
            .scope(async {
                record_active_stage("classify_intent", Duration::from_millis(40));
                record_active_stage("skill_execute", Duration::from_millis(15));
            })
            .await;
        let stages: Vec<String> = trace.spans().into_iter().map(|span| span.stage).collect();
        assert_eq!(stages, vec!["classify_intent", "skill_execute"]);
    }

    #[tokio::test]
    async fn stages_recorded_outside_any_scope_are_ignored() {
        let trace = TurnTrace::new("backend", "pod-1-8");
        record_active_stage("classify_intent", Duration::from_millis(40));
        assert!(trace.spans().is_empty());
    }

    #[tokio::test]
    async fn a_fast_turn_produces_no_slow_report() {
        let trace = TurnTrace::new("backend", "pod-1-9");
        trace.record("classify_intent", Duration::from_millis(1));
        assert!(trace.finish(Duration::from_secs(60)).is_none());
    }

    #[tokio::test]
    async fn a_slow_turn_reports_its_slowest_stage_and_unaccounted_time() {
        let trace = TurnTrace::new("backend", "pod-1-10");
        tokio::time::sleep(Duration::from_millis(60)).await;
        trace.record("palace_memory_context", Duration::from_millis(5));
        trace.record("classifier_llm_roundtrip", Duration::from_millis(30));
        let report = trace
            .finish(Duration::from_millis(10))
            .unwrap_or_else(|| panic!("turn over threshold is reported"));
        assert_eq!(report.turn_id, "pod-1-10");
        assert_eq!(report.component, "backend");
        assert_eq!(report.slowest_stage, "classifier_llm_roundtrip");
        assert!(report.total >= Duration::from_millis(60));
        assert!(
            report.unaccounted >= Duration::from_millis(20),
            "time outside every stage must be surfaced, got {:?}",
            report.unaccounted
        );
        assert!(report
            .timeline_json()
            .contains("\"classifier_llm_roundtrip\""));
    }

    #[tokio::test]
    async fn overlapping_stages_are_not_double_counted_as_accounted_time() {
        let trace = TurnTrace::new("backend", "pod-1-11");
        tokio::time::sleep(Duration::from_millis(50)).await;
        // classify_intent contains the classifier round trip.
        trace.record("classifier_llm_roundtrip", Duration::from_millis(30));
        trace.record("classify_intent", Duration::from_millis(40));
        let report = trace
            .finish(Duration::from_millis(1))
            .unwrap_or_else(|| panic!("slow turn is reported"));
        assert!(
            report.unaccounted >= Duration::from_millis(8),
            "overlap counted twice would leave {:?}",
            report.unaccounted
        );
    }

    #[test]
    fn the_trace_never_panics_when_the_lock_is_poisoned() {
        let trace = TurnTrace::new("gateway", "pod-1-12");
        let shared = trace.clone();
        let _ = std::thread::spawn(move || {
            shared.poison_for_test();
        })
        .join();
        trace.record("tts_synthesis", Duration::from_millis(3));
        assert!(trace.finish(Duration::from_secs(60)).is_none());
    }

    #[test]
    fn the_slow_turn_threshold_is_configurable() {
        set_slow_turn_threshold(Duration::from_millis(1234));
        assert_eq!(slow_turn_threshold(), Duration::from_millis(1234));
    }

    #[tokio::test]
    async fn time_spent_speaking_does_not_make_a_turn_slow() {
        let trace = TurnTrace::new("backend", "pod-1-13");
        tokio::time::sleep(Duration::from_millis(80)).await;
        trace.record("speech", Duration::from_millis(80));
        trace.mark_speech_end();
        trace.record("classifier_llm_roundtrip", Duration::from_millis(1));
        assert!(trace.finish(Duration::from_millis(40)).is_none());
    }

    #[tokio::test]
    async fn a_slow_answer_after_speech_is_measured_from_speech_end() {
        let trace = TurnTrace::new("backend", "pod-1-14");
        tokio::time::sleep(Duration::from_millis(80)).await;
        trace.record("speech", Duration::from_millis(80));
        trace.mark_speech_end();
        tokio::time::sleep(Duration::from_millis(60)).await;
        trace.record("classifier_llm_roundtrip", Duration::from_millis(40));
        let report = trace
            .finish(Duration::from_millis(40))
            .unwrap_or_else(|| panic!("slow answer is reported"));
        assert!(report.total >= Duration::from_millis(60));
        assert!(
            report.total < Duration::from_millis(120),
            "speech must not count towards the wait, got {:?}",
            report.total
        );
        assert!(report.timeline_json().contains("\"speech\""));
        assert!(
            report.unaccounted >= Duration::from_millis(15),
            "got {:?}",
            report.unaccounted
        );
    }
}
