//! Runtime integration: gate, empty input, needs-search yes/no, interrupt, intent+weather.

use async_trait::async_trait;
use core_audio::{AudioCapture, CaptureError};
use core_config::Config;
use core_orchestrator::{IntentClassifier, IntentDecision, LlmStream, SttStream, TtsSink};
use core_policy::{ActionRequest, PolicyDecision, PolicyEngine};
use core_search::MockSearch;
use core_skills::{
    DistanceResult, DistanceSkillError, MediaResult, MessageResult, MessageSkillError,
    MockDistanceSkill, MockMediaSkill, MockMessageSkill, MockSmartHomeSkill, MockTimeSkill,
    MockWeatherSkill, ResolvedLocation, SmartHomeResult, TimeResult, WeatherResult, WeatherSkill,
    WeatherSkillError,
};
use desktop_runner::{
    ContinuousRunOptions, DesktopRuntime, MemoryStore, RuntimeTurnOutcome, SkillRunContext,
};
use futures::stream;
use serial_test::serial;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
    fn must_err(self) -> E;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }

    fn must_err(self) -> E {
        match self {
            Ok(_) => panic!("expected Err(..) in test, got Ok"),
            Err(error) => error,
        }
    }
}

struct MockStt(String);

#[async_trait]
impl SttStream for MockStt {
    async fn push_audio(
        &mut self,
        _pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.0.clone())
    }
}

struct QueueStt {
    transcripts: VecDeque<String>,
}

impl QueueStt {
    fn new(items: Vec<&str>) -> Self {
        Self {
            transcripts: items.into_iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[async_trait]
impl SttStream for QueueStt {
    async fn push_audio(
        &mut self,
        _pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.transcripts.pop_front().unwrap_or_default())
    }
}

struct CountingStt {
    transcripts: VecDeque<String>,
    pushed_samples: usize,
}

impl CountingStt {
    fn new(items: Vec<&str>) -> Self {
        Self {
            transcripts: items.into_iter().map(|s| s.to_string()).collect(),
            pushed_samples: 0,
        }
    }
}

#[async_trait]
impl SttStream for CountingStt {
    async fn push_audio(
        &mut self,
        pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.pushed_samples += pcm.len();
        Ok(())
    }

    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.transcripts.pop_front().unwrap_or_default())
    }
}

struct MockLlm(&'static str);

#[async_trait]
impl LlmStream for MockLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(Box::new(stream::iter(vec![self.0.to_string()])))
    }
}

struct FailLlm;

#[async_trait]
impl LlmStream for FailLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Err("llm should not be called for local time intent".into())
    }
}

/// Mock intent classifier that returns a fixed decision.
struct MockIntentClassifier(IntentDecision);

#[async_trait]
impl IntentClassifier for MockIntentClassifier {
    async fn classify(
        &self,
        _user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.0.clone())
    }
}

/// LLM that records the last user message and returns a fixed response (for testing weather path).
struct RecordLlm {
    response: String,
    last_user_text: Arc<std::sync::Mutex<String>>,
}

impl RecordLlm {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            last_user_text: Arc::new(std::sync::Mutex::new(String::new())),
        }
    }
    fn last_user_text(&self) -> String {
        self.last_user_text.lock().must().clone()
    }
}

#[async_trait]
impl LlmStream for RecordLlm {
    async fn chat_stream(
        &self,
        user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        *self.last_user_text.lock().must() = user_text.to_string();
        Ok(Box::new(stream::iter(vec![self.response.clone()])))
    }
}

struct HistoryCountLlm {
    last_history_len: Arc<std::sync::Mutex<usize>>,
}

impl HistoryCountLlm {
    fn new() -> Self {
        Self {
            last_history_len: Arc::new(std::sync::Mutex::new(0)),
        }
    }

    fn last_history_len(&self) -> usize {
        *self.last_history_len.lock().must()
    }
}

#[async_trait]
impl LlmStream for HistoryCountLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        *self.last_history_len.lock().must() = history.len();
        Ok(Box::new(stream::iter(vec!["ok".to_string()])))
    }
}

struct QueueLlm {
    responses: Arc<Mutex<VecDeque<String>>>,
}

impl QueueLlm {
    fn new(items: Vec<&str>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(
                items.into_iter().map(|s| s.to_string()).collect(),
            )),
        }
    }
}

#[async_trait]
impl LlmStream for QueueLlm {
    async fn chat_stream(
        &self,
        _user_text: &str,
        _history: &[(String, String)],
        _system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn futures::Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let mut guard = self.responses.lock().await;
        let next = guard.pop_front().unwrap_or_default();
        Ok(Box::new(stream::iter(vec![next])))
    }
}

struct RecordingWeatherSkill {
    result: WeatherResult,
    last_location: Arc<std::sync::Mutex<Option<String>>>,
}

impl RecordingWeatherSkill {
    fn new(result: WeatherResult) -> Self {
        Self {
            result,
            last_location: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn last_location(&self) -> Option<String> {
        self.last_location.lock().must().clone()
    }
}

#[async_trait]
impl WeatherSkill for RecordingWeatherSkill {
    async fn execute(
        &self,
        location: Option<&str>,
        _default_location: Option<&ResolvedLocation>,
    ) -> Result<WeatherResult, core_skills::WeatherSkillError> {
        *self.last_location.lock().must() = location.map(|s| s.to_string());
        Ok(self.result.clone())
    }
}

struct PanicWeatherSkill;

#[async_trait]
impl WeatherSkill for PanicWeatherSkill {
    async fn execute(
        &self,
        _location: Option<&str>,
        _default_location: Option<&ResolvedLocation>,
    ) -> Result<WeatherResult, core_skills::WeatherSkillError> {
        panic!("weather skill should not be called when location normalization is ambiguous");
    }
}

#[derive(Default)]
struct DenyPolicy;

impl PolicyEngine for DenyPolicy {
    fn emergency_stop(&self) -> bool {
        false
    }

    fn allow_action(&self, _request: &ActionRequest) -> PolicyDecision {
        PolicyDecision::Deny("denied by test policy".to_string())
    }
}

struct MockTts {
    spoken: Vec<String>,
    raw_pcm: Vec<Vec<u8>>,
    stops_requested: usize,
}

struct ScriptedCapture {
    chunks: VecDeque<Vec<i16>>,
}

impl ScriptedCapture {
    fn with_chunk_count(count: usize, samples_per_chunk: usize) -> Self {
        let mut chunks = VecDeque::new();
        for _ in 0..count {
            chunks.push_back(vec![1_200_i16; samples_per_chunk]);
        }
        Self { chunks }
    }

    fn with_chunks(chunks: Vec<Vec<i16>>) -> Self {
        Self {
            chunks: chunks.into_iter().collect(),
        }
    }
}

impl AudioCapture for ScriptedCapture {
    fn read_chunk(&mut self, timeout: Duration) -> Result<Vec<i16>, CaptureError> {
        if let Some(chunk) = self.chunks.pop_front() {
            return Ok(chunk);
        }
        std::thread::sleep(timeout);
        Err(CaptureError::Timeout)
    }
}

impl MockTts {
    fn new() -> Self {
        Self {
            spoken: Vec::new(),
            raw_pcm: Vec::new(),
            stops_requested: 0,
        }
    }
    fn text(&self) -> String {
        self.spoken.join("")
    }
}

#[async_trait]
impl TtsSink for MockTts {
    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.spoken.push(text.to_string());
        Ok(())
    }
    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn request_stop_playback(&mut self) {
        self.stops_requested += 1;
    }

    async fn play_pcm_bytes(
        &mut self,
        pcm: &[u8],
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.raw_pcm.push(pcm.to_vec());
        Ok(true)
    }
}

#[tokio::test]
async fn runtime_gate_closed_when_wake_enabled_and_not_activated() {
    let config = Config {
        wake_word: core_config::WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut runtime = DesktopRuntime::new(config);
    let mut stt = MockStt("hello".to_string());
    let llm = MockLlm("Hi");
    let mut tts = MockTts::new();
    let search: Option<MockSearch> = None;
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, search.as_ref(), rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::GateClosed);
    assert!(tts.text().is_empty());
}

#[tokio::test]
async fn runtime_empty_input_returns_empty_input() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("".to_string());
    let llm = MockLlm("Hi");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::EmptyInput);
}

#[tokio::test]
async fn runtime_stop_voice_command_stops_playback_and_does_not_call_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("stop the music".to_string());
    let llm = MockLlm("should not be used");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.spoken.is_empty(),
        "stop intent must not trigger LLM or TTS"
    );
    assert!(
        tts.stops_requested >= 1,
        "stop intent must request playback stop, got {}",
        tts.stops_requested
    );
}

#[tokio::test]
async fn runtime_stop_variants_all_trigger_stop() {
    let phrases = [
        "stop",
        "computer stop",
        "computer, stop",
        "stop playing",
        "be quiet",
        "shut up",
        "that's enough",
        "stop it",
    ];
    for phrase in phrases {
        let config = Config::default();
        let mut runtime = DesktopRuntime::new(config);
        runtime.activate_wake();
        let mut stt = MockStt(phrase.to_string());
        let llm = MockLlm("unused");
        let mut tts = MockTts::new();
        let (_tx, rx) = tokio::sync::broadcast::channel(1);

        let outcome = runtime
            .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
            .await
            .must();

        assert_eq!(
            outcome,
            RuntimeTurnOutcome::Complete,
            "phrase: {:?}",
            phrase
        );
        assert!(tts.stops_requested >= 1, "phrase: {:?}", phrase);
    }
}

#[tokio::test]
async fn runtime_needs_search_user_yes_speaks_search_result() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what is X?".to_string());
    let llm = MockLlm("Not sure. [NEED_SEARCH: what is X]");
    let mut tts = MockTts::new();
    let search = MockSearch::new("X is 42.");
    let user_said_yes = Arc::new(AtomicBool::new(true));
    let u = Arc::clone(&user_said_yes);
    runtime = runtime.with_user_confirm(Arc::new(move |_local, _query| {
        let u = Arc::clone(&u);
        Box::pin(async move { u.load(Ordering::SeqCst) })
    }));
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, Some(&search), rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("42"));
}

#[tokio::test]
async fn runtime_needs_search_user_no_speaks_local_only() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("something".to_string());
    let llm = MockLlm("Maybe. [NEED_SEARCH: optional]");
    let mut tts = MockTts::new();
    let search = MockSearch::new("never used");
    let user_said_yes = Arc::new(AtomicBool::new(false));
    let u = Arc::clone(&user_said_yes);
    runtime = runtime.with_user_confirm(Arc::new(move |_local, _query| {
        let u = Arc::clone(&u);
        Box::pin(async move { u.load(Ordering::SeqCst) })
    }));
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, Some(&search), rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("Maybe"));
}

#[tokio::test]
async fn runtime_speak_cancel_returns_interrupted() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("hi".to_string());
    let llm = MockLlm("A very long response that goes on and on");
    let mut tts = MockTts::new();
    let (tx, rx) = tokio::sync::broadcast::channel(1);
    let _ = tx.send(());
    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();
    assert_eq!(outcome, RuntimeTurnOutcome::Interrupted);
}

#[tokio::test]
async fn runtime_local_time_query_bypasses_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what time is it".to_string());
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("The current time is"));
}

#[tokio::test]
#[serial]
async fn runtime_play_chocobo_bypasses_llm_and_streams_raw_pcm() {
    let temp = tempfile::Builder::new()
        .prefix("aice-chocobo-")
        .suffix(".c")
        .tempfile()
        .must();
    std::fs::write(
        temp.path(),
        "const unsigned char audio_chocobo[] = { 0x01, 0x00, 0x02, 0x00 };",
    )
    .must();
    std::env::set_var("AICE_CHOCOBO_C_PATH", temp.path());

    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("play chocobo".to_string());
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();

    std::env::remove_var("AICE_CHOCOBO_C_PATH");
    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(tts.raw_pcm.len(), 1);
    assert_eq!(tts.raw_pcm[0], vec![1, 0, 2, 0]);
}

#[tokio::test]
#[serial]
async fn runtime_play_chocobo_variant_phrase_bypasses_llm() {
    let temp = tempfile::Builder::new()
        .prefix("aice-chocobo-")
        .suffix(".c")
        .tempfile()
        .must();
    std::fs::write(
        temp.path(),
        "const unsigned char audio_chocobo[] = { 0x10, 0x00, 0x20, 0x00 };",
    )
    .must();
    std::env::set_var("AICE_CHOCOBO_C_PATH", temp.path());

    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("please play that chocobo song".to_string());
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx)
        .await
        .must();

    std::env::remove_var("AICE_CHOCOBO_C_PATH");
    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(tts.raw_pcm.len(), 1);
    assert_eq!(tts.raw_pcm[0], vec![16, 0, 32, 0]);
}

#[tokio::test]
async fn runtime_continuous_loop_processes_multiple_turns() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    config.audio.speech_end_silence_ms = 1;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_200_i16; 320],
        vec![0_i16; 320],
        vec![1_200_i16; 320],
        vec![0_i16; 320],
    ]);
    let mut stt = QueueStt::new(vec!["hello", "second"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let stats = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(2),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .must()
    .must();

    assert_eq!(stats.turns_completed, 2);
    assert!(tts.text().contains("ok"));
}

#[tokio::test]
async fn runtime_continuous_loop_activates_on_wake_phrase() {
    let mut config = Config {
        wake_word: core_config::WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            cooldown_secs: 10,
            ..Default::default()
        },
        ..Default::default()
    };
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    config.audio.speech_end_silence_ms = 1;
    config.audio.idle_sleep_ms = 1;

    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_200_i16; 320],
        vec![0_i16; 320],
        vec![1_200_i16; 320],
        vec![0_i16; 320],
    ]);
    let mut stt = QueueStt::new(vec!["hello there", "computer what time is it"]);
    let llm = MockLlm("wake-ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let stats = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .must()
    .must();

    assert_eq!(stats.turns_completed, 1);
    assert_eq!(stats.wake_activations, 1);
}

#[tokio::test]
async fn runtime_continuous_loop_allows_computer_pause_when_gate_closed() {
    let mut config = Config {
        wake_word: core_config::WakeWordConfig {
            enabled: true,
            phrases: vec!["computer".to_string()],
            cooldown_secs: 10,
            ..Default::default()
        },
        ..Default::default()
    };
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;

    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunk_count(1, 320);
    let mut stt = QueueStt::new(vec!["computer pause"]);
    let llm = MockLlm("should-not-be-used");
    let mut tts = MockTts::new();
    let media_skill = MockMediaSkill::ok(MediaResult {
        summary: "Paused.".to_string(),
        now_playing: None,
        state: "paused".to_string(),
    });
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let stats = runtime
        .run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: Some(&media_skill),
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        )
        .await
        .must();

    assert_eq!(stats.turns_completed, 1);
    assert!(
        tts.stops_requested >= 1,
        "pause interrupt should request playback stop"
    );
}

#[tokio::test]
async fn runtime_continuous_loop_counts_interruption() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunk_count(1, 320);
    let mut stt = QueueStt::new(vec!["hello"]);
    let llm = MockLlm("a very long response that goes on and on and on and on");
    let mut tts = MockTts::new();
    let (tx, rx) = tokio::sync::broadcast::channel(1);
    let _ = tx.send(());

    let stats = runtime
        .run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        )
        .await
        .must();

    assert_eq!(stats.turns_interrupted, 1);
}

#[tokio::test]
async fn runtime_continuous_flushes_partial_turn_on_timeout() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 3000;
    config.audio.chunk_timeout_ms = 1;
    config.audio.speech_end_silence_ms = 1;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunk_count(1, 320);
    let mut stt = QueueStt::new(vec!["hello"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await;

    let stats = result.must();
    let stats = stats.must();
    assert_eq!(stats.turns_completed, 1);
}

#[tokio::test]
async fn runtime_continuous_waits_for_silence_threshold_before_flush() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 3000;
    config.audio.chunk_timeout_ms = 20;
    config.audio.speech_end_silence_ms = 200;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunk_count(1, 320);
    let mut stt = QueueStt::new(vec!["hello"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(120),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await;

    assert!(
        result.is_err(),
        "runtime should still be buffering; silence threshold not reached yet"
    );
}

#[tokio::test]
async fn runtime_continuous_does_not_flush_on_turn_window_while_voice_continues() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 20;
    config.audio.speech_end_silence_ms = 250;
    config.audio.speech_rms_threshold = 0.008;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_600_i16; 320],
        vec![1_600_i16; 320],
        vec![1_600_i16; 320],
    ]);
    let mut stt = QueueStt::new(vec!["play favorites playlist"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(120),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await;

    assert!(
        result.is_err(),
        "runtime should not flush by turn_window while voiced chunks keep arriving"
    );
}

#[tokio::test]
async fn runtime_continuous_flushes_after_silent_audio_pause() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 3000;
    config.audio.chunk_timeout_ms = 20;
    config.audio.speech_end_silence_ms = 80;
    config.audio.speech_rms_threshold = 0.008;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_500_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
    ]);
    let mut stt = QueueStt::new(vec!["hello"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(350),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .must()
    .must();

    assert_eq!(result.turns_completed, 1);
}

#[tokio::test]
async fn runtime_continuous_streams_silent_chunks_after_voice_starts() {
    let mut config = Config::default();
    config.audio.turn_window_ms = 3000;
    config.audio.chunk_timeout_ms = 20;
    config.audio.speech_end_silence_ms = 80;
    config.audio.speech_rms_threshold = 0.008;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_500_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
        vec![0_i16; 320],
    ]);
    let mut stt = CountingStt::new(vec!["hello"]);
    let llm = MockLlm("ok");
    let mut tts = MockTts::new();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(350),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(1),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: None,
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .must()
    .must();

    assert_eq!(result.turns_completed, 1);
    assert_eq!(
        stt.pushed_samples, 1600,
        "runtime should stream all chunks after voice starts, including silence"
    );
}

#[tokio::test]
async fn runtime_continuous_higher_silence_threshold_increases_completion_time() {
    async fn run_once(speech_end_silence_ms: u64) -> std::time::Duration {
        let mut config = Config::default();
        config.audio.turn_window_ms = 3000;
        config.audio.chunk_timeout_ms = 20;
        config.audio.speech_end_silence_ms = speech_end_silence_ms;
        config.audio.idle_sleep_ms = 1;
        let mut runtime = DesktopRuntime::new(config);
        let mut capture = ScriptedCapture::with_chunk_count(1, 320);
        let mut stt = QueueStt::new(vec!["hello"]);
        let llm = MockLlm("ok");
        let mut tts = MockTts::new();
        let (_tx, rx) = tokio::sync::broadcast::channel(1);

        let started = std::time::Instant::now();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime.run_continuous(
                &mut capture,
                &mut stt,
                &llm,
                &mut tts,
                ContinuousRunOptions {
                    search: None::<&MockSearch>,
                    cancel_rx: rx,
                    max_turns: Some(1),
                    skills: SkillRunContext {
                        intent_classifier: None,
                        weather_skill: None,
                        time_skill: None,
                        distance_skill: None,
                        smart_home_skill: None,
                        assistant_skill: None,
                        media_skill: None,
                        memory_skill: None,
                        computer_skill: None,
                        app_switcher_skill: None,
                        reminder_skill: None,
                        message_skill: None,
                        timer_skill: None,
                        shopping_list_skill: None,
                        volume_skill: None,
                        resolved_location: None,
                        memory: None,
                        policy: None,
                    },
                },
            ),
        )
        .await
        .must()
        .must();
        started.elapsed()
    }

    let low = run_once(20).await;
    let high = run_once(140).await;
    assert!(
        high > low + std::time::Duration::from_millis(80),
        "higher silence threshold should take longer; low={:?} high={:?}",
        low,
        high
    );
}

#[tokio::test]
async fn runtime_ignores_recent_assistant_echo_without_wake_phrase() {
    let mut config = Config::default();
    config.wake_word.enabled = true;
    config.wake_word.phrases = vec!["computer".to_string()];
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    config.audio.speech_end_silence_ms = 1;
    config.audio.idle_sleep_ms = 1;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut capture = ScriptedCapture::with_chunk_count(2, 320);
    let mut stt = QueueStt::new(vec![
        "computer play blinding lights",
        "Now Playing - blinding lights",
    ]);
    let llm = MockLlm("unused");
    let mut tts = MockTts::new();
    let media_skill = MockMediaSkill::ok(MediaResult {
        summary: "Now Playing - Blinding Lights - The Weeknd".to_string(),
        now_playing: Some("Blinding Lights - The Weeknd".to_string()),
        state: "playing".to_string(),
    });
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let run = runtime.run_continuous(
        &mut capture,
        &mut stt,
        &llm,
        &mut tts,
        ContinuousRunOptions {
            search: None::<&MockSearch>,
            cancel_rx: rx,
            max_turns: Some(2),
            skills: SkillRunContext {
                intent_classifier: None,
                weather_skill: None,
                time_skill: None,
                distance_skill: None,
                smart_home_skill: None,
                assistant_skill: None,
                media_skill: Some(&media_skill),
                memory_skill: None,
                computer_skill: None,
                app_switcher_skill: None,
                reminder_skill: None,
                message_skill: None,
                timer_skill: None,
                shopping_list_skill: None,
                volume_skill: None,
                resolved_location: None,
                memory: None,
                policy: None,
            },
        },
    );
    let stats = tokio::time::timeout(std::time::Duration::from_millis(400), run)
        .await
        .must_err();
    let _ = stats;
}

#[tokio::test]
async fn runtime_does_not_ignore_stop_as_echo() {
    let mut config = Config::default();
    config.wake_word.enabled = true;
    config.wake_word.phrases = vec!["computer".to_string()];
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    config.audio.speech_end_silence_ms = 1;
    config.audio.idle_sleep_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut capture = ScriptedCapture::with_chunks(vec![
        vec![1_200_i16; 320],
        vec![0_i16; 320],
        vec![1_200_i16; 320],
        vec![0_i16; 320],
    ]);
    let mut stt = QueueStt::new(vec!["computer play blinding lights", "stop"]);
    let llm = MockLlm("unused");
    let mut tts = MockTts::new();
    let media_skill = MockMediaSkill::ok(MediaResult {
        summary: "Now Playing - Blinding Lights - The Weeknd".to_string(),
        now_playing: Some("Blinding Lights - The Weeknd".to_string()),
        state: "playing".to_string(),
    });
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let stats = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime.run_continuous(
            &mut capture,
            &mut stt,
            &llm,
            &mut tts,
            ContinuousRunOptions {
                search: None::<&MockSearch>,
                cancel_rx: rx,
                max_turns: Some(2),
                skills: SkillRunContext {
                    intent_classifier: None,
                    weather_skill: None,
                    time_skill: None,
                    distance_skill: None,
                    smart_home_skill: None,
                    assistant_skill: None,
                    media_skill: Some(&media_skill),
                    memory_skill: None,
                    computer_skill: None,
                    app_switcher_skill: None,
                    reminder_skill: None,
                    message_skill: None,
                    timer_skill: None,
                    shopping_list_skill: None,
                    volume_skill: None,
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .must()
    .must();

    assert_eq!(stats.turns_completed, 2);
    assert!(tts.stops_requested >= 1);
}

#[tokio::test]
async fn runtime_requires_wake_phrase_for_each_turn_when_wake_enabled() {
    let mut config = Config::default();
    config.wake_word.enabled = true;
    config.wake_word.phrases = vec!["computer".to_string()];
    config.audio.turn_window_ms = 20;
    config.audio.chunk_timeout_ms = 1;
    let mut runtime = DesktopRuntime::new(config);
    let mut capture = ScriptedCapture::with_chunk_count(2, 320);
    let mut stt = QueueStt::new(vec!["computer play blinding lights", "play favorites"]);
    let llm = MockLlm("unused");
    let mut tts = MockTts::new();
    let media_skill = MockMediaSkill::ok(MediaResult {
        summary: "Now Playing - Blinding Lights - The Weeknd".to_string(),
        now_playing: Some("Blinding Lights - The Weeknd".to_string()),
        state: "playing".to_string(),
    });
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let run = runtime.run_continuous(
        &mut capture,
        &mut stt,
        &llm,
        &mut tts,
        ContinuousRunOptions {
            search: None::<&MockSearch>,
            cancel_rx: rx,
            max_turns: Some(2),
            skills: SkillRunContext {
                intent_classifier: None,
                weather_skill: None,
                time_skill: None,
                distance_skill: None,
                smart_home_skill: None,
                assistant_skill: None,
                media_skill: Some(&media_skill),
                memory_skill: None,
                computer_skill: None,
                app_switcher_skill: None,
                reminder_skill: None,
                message_skill: None,
                timer_skill: None,
                shopping_list_skill: None,
                volume_skill: None,
                resolved_location: None,
                memory: None,
                policy: None,
            },
        },
    );

    let _ = tokio::time::timeout(std::time::Duration::from_millis(400), run)
        .await
        .must_err();
}

// --- Intent + weather skill tests ---

fn default_resolved_location() -> ResolvedLocation {
    ResolvedLocation {
        display_name: "London, UK".to_string(),
        lat: 51.5074,
        lon: -0.1278,
    }
}

#[tokio::test]
async fn intent_weather_uses_default_location_and_streams_llm_answer() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("how's the weather?".to_string());
    let weather_result = WeatherResult {
        location_display: "London, UK".to_string(),
        temp_c: 14.0,
        humidity_pct: Some(72),
        weather_code: 61,
        description: "Rain".to_string(),
    };
    let weather_skill = MockWeatherSkill::ok(weather_result.clone());
    let resolved = default_resolved_location();
    let llm = RecordLlm::new("It's 14 degrees and rainy in London.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather { location: None });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None,
        distance_skill: None,
        smart_home_skill: None,
        assistant_skill: None,
        media_skill: None,
        memory_skill: None,
        computer_skill: None,
        app_switcher_skill: None,
        reminder_skill: None,
        message_skill: None,
        timer_skill: None,
        shopping_list_skill: None,
        volume_skill: None,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&weather_result.to_prompt_context()),
        "LLM should receive weather context in prompt, got: {}",
        llm.last_user_text()
    );
    assert!(
        llm.last_user_text().contains("Do not mention distance"),
        "Weather prompt should forbid unrelated facts, got: {}",
        llm.last_user_text()
    );
    assert!(
        llm.last_user_text()
            .contains("Reply with exactly 1 short sentence"),
        "Weather prompt should force short output, got: {}",
        llm.last_user_text()
    );
    assert!(tts.text().contains("14 degrees"));
}

#[tokio::test]
async fn intent_weather_with_explicit_location_calls_skill_with_location() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("how's the weather in Rome?".to_string());
    let weather_result = WeatherResult {
        location_display: "Rome, Italy".to_string(),
        temp_c: 22.0,
        humidity_pct: Some(65),
        weather_code: 0,
        description: "Clear sky".to_string(),
    };
    let weather_skill = MockWeatherSkill::ok(weather_result);
    let resolved = default_resolved_location();
    let llm = QueueLlm::new(vec![
        r#"{"status":"ok","location":"Rome, Italy"}"#,
        "Sunny and 22 in Rome.",
    ]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather {
        location: Some("Rome".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None,
        distance_skill: None,
        smart_home_skill: None,
        assistant_skill: None,
        media_skill: None,
        memory_skill: None,
        computer_skill: None,
        app_switcher_skill: None,
        reminder_skill: None,
        message_skill: None,
        timer_skill: None,
        shopping_list_skill: None,
        volume_skill: None,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("22"));
}

#[tokio::test]
async fn intent_weather_geocoding_no_results_speaks_short_clarification_without_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what's the weather in Los Angeles?".to_string());
    let weather_skill =
        MockWeatherSkill::err(WeatherSkillError::Geocoding("no results".to_string()));
    let resolved = default_resolved_location();
    let llm = QueueLlm::new(vec![
        r#"{"status":"ok","location":"Los Angeles, United States"}"#,
    ]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather {
        location: Some("Los Angeles".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    let spoken = tts.text();
    assert!(
        spoken
            .to_lowercase()
            .contains("couldn't resolve that location"),
        "Expected a short clarification for unresolved geocoding, got: {}",
        spoken
    );
    assert!(
        spoken.to_lowercase().contains("city and country"),
        "Expected explicit retry guidance, got: {}",
        spoken
    );
    assert!(
        spoken.len() <= 120,
        "Clarification should stay short for voice; got {} chars: {}",
        spoken.len(),
        spoken
    );
}

#[tokio::test]
async fn intent_weather_normalizes_location_contract_before_skill_execute() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what's the weather in LA?".to_string());
    let weather_result = WeatherResult {
        location_display: "Los Angeles, United States".to_string(),
        temp_c: 19.0,
        humidity_pct: Some(54),
        weather_code: 1,
        description: "Partly cloudy".to_string(),
    };
    let weather_skill = RecordingWeatherSkill::new(weather_result);
    let resolved = default_resolved_location();
    let llm = QueueLlm::new(vec![
        r#"{"status":"ok","location":"Los Angeles, United States"}"#,
        "It's around 19 degrees and partly cloudy in Los Angeles.",
    ]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather {
        location: Some("LA".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(
        weather_skill.last_location().as_deref(),
        Some("Los Angeles, United States")
    );
    assert!(tts.text().contains("19 degrees"));
}

#[tokio::test]
async fn intent_weather_location_contract_ambiguous_speaks_clarification_without_skill_call() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what's the weather in LA?".to_string());
    let weather_skill = PanicWeatherSkill;
    let resolved = default_resolved_location();
    let llm = QueueLlm::new(vec![r#"{"status":"ambiguous","location":"LA"}"#]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather {
        location: Some("LA".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.text().to_lowercase().contains("city and country"),
        "expected location clarification, got: {}",
        tts.text()
    );
}

#[tokio::test]
async fn intent_weather_location_contract_ambiguous_uses_valid_hint_city_country() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("what's the weather in LA?".to_string());
    let weather_result = WeatherResult {
        location_display: "Los Angeles, United States".to_string(),
        temp_c: 20.0,
        humidity_pct: Some(50),
        weather_code: 1,
        description: "Partly cloudy".to_string(),
    };
    let weather_skill = RecordingWeatherSkill::new(weather_result);
    let resolved = default_resolved_location();
    let llm = QueueLlm::new(vec![
        r#"{"status":"ambiguous","location":"LA, USA"}"#,
        r#"{"status":"unknown","location":"Los Angeles, United States"}"#,
        "It's around 20 degrees in Los Angeles.",
    ]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillWeather {
        location: Some("Los Angeles, USA".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: Some(&weather_skill),
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(
        weather_skill.last_location().as_deref(),
        Some("Los Angeles, United States")
    );
    assert!(tts.text().contains("20 degrees"));
}

#[tokio::test]
async fn intent_chat_routes_to_chat_path() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("tell me a joke".to_string());
    let llm = RecordLlm::new("Why did the chicken cross the road?");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::Chat);
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(llm.last_user_text(), "tell me a joke");
    assert!(tts.text().contains("chicken"));
}

#[tokio::test]
async fn no_intent_classifier_uses_chat_path() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("hello".to_string());
    let llm = MockLlm("Hi there");
    let mut tts = MockTts::new();
    let skills = SkillRunContext {
        intent_classifier: None::<&dyn IntentClassifier>,
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("Hi there"));
}

#[tokio::test]
async fn intent_time_uses_default_location_and_streams_llm_answer() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    // Use a phrase that does not match local_command "what time is it" so we hit the time skill path.
    let mut stt = MockStt("what's the time?".to_string());
    let time_result = TimeResult {
        location_display: "London, UK".to_string(),
        local_time: "2025-03-15T14:30:00".to_string(),
        timezone: "GMT".to_string(),
    };
    let time_skill = MockTimeSkill::ok(time_result.clone());
    let resolved = default_resolved_location();
    let llm = RecordLlm::new("It's 2:30 PM in London.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillTime { location: None });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: Some(&time_skill),
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.text().contains("2:30"),
        "Time skill path should stream answer to TTS; got: {}",
        tts.text()
    );
}

#[tokio::test]
async fn intent_distance_destination_only_uses_default_origin() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("how far is Paris?".to_string());
    let dist_result = DistanceResult {
        origin_display: "London, UK".to_string(),
        destination_display: "Paris, France".to_string(),
        distance_km: 344.0,
    };
    let distance_skill = MockDistanceSkill::ok(dist_result.clone());
    let resolved = default_resolved_location();
    let llm = RecordLlm::new("Paris is about 344 km from London.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillDistance {
        origin: None,
        destination: Some("Paris".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: Some(&distance_skill),
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&dist_result.to_prompt_context()),
        "LLM should receive distance context, got: {}",
        llm.last_user_text()
    );
    assert!(tts.text().contains("344"));
}

#[tokio::test]
async fn intent_distance_geocoding_no_results_speaks_short_clarification_without_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("how far is Barely?".to_string());
    let distance_skill =
        MockDistanceSkill::err(DistanceSkillError::Geocoding("no results".to_string()));
    let resolved = default_resolved_location();
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillDistance {
        origin: None,
        destination: Some("Barely".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: Some(&distance_skill),
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    let spoken = tts.text();
    assert!(
        spoken.to_lowercase().contains("couldn't find that place"),
        "Expected a short clarification for unresolved geocoding, got: {}",
        spoken
    );
    assert!(
        spoken.to_lowercase().contains("city and country"),
        "Expected explicit retry guidance, got: {}",
        spoken
    );
    assert!(
        spoken.len() <= 120,
        "Clarification should stay short for voice; got {} chars: {}",
        spoken.len(),
        spoken
    );
}

#[tokio::test]
async fn intent_distance_missing_places_speaks_short_clarification_without_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("how far is Ouch's book?".to_string());
    let distance_skill = MockDistanceSkill::err(DistanceSkillError::MissingPlaces);
    let resolved = default_resolved_location();
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillDistance {
        origin: None,
        destination: None,
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: Some(&distance_skill),
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    let spoken = tts.text();
    assert!(
        spoken.to_lowercase().contains("which place"),
        "Expected a short clarification prompt, got: {}",
        spoken
    );
    assert!(
        spoken.to_lowercase().contains("city and country"),
        "Expected explicit retry guidance, got: {}",
        spoken
    );
    assert!(
        spoken.len() <= 120,
        "Clarification should stay short for voice; got {} chars: {}",
        spoken.len(),
        spoken
    );
}

#[tokio::test]
async fn intent_smart_home_uses_skill_and_streams_llm_answer() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("turn off the living room lights".to_string());
    let smart_home_result = SmartHomeResult {
        summary: "Living room lights turned off.".to_string(),
        device_states: vec![core_skills::DeviceState {
            id: "lr-1".to_string(),
            name: "Living room".to_string(),
            state: "off".to_string(),
        }],
    };
    let smart_home_skill = MockSmartHomeSkill::ok(smart_home_result.clone());
    let llm = RecordLlm::new("Living room lights are now off.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillSmartHome {
        target: Some("living room".to_string()),
        action: Some("turn off".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: Some(&smart_home_skill),
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&smart_home_result.to_prompt_context()),
        "LLM should receive smart home context, got: {}",
        llm.last_user_text()
    );
    assert!(tts.text().to_lowercase().contains("off"));
}

#[tokio::test]
async fn chat_turn_with_memory_appends_and_persists_history() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("tell me something".to_string());
    let llm = MockLlm("Here you go.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::Chat);
    let limits = core_config::MemoryConfig::default();
    let mut store = MemoryStore::new(&limits);
    store.push_turn("hi", "hello");
    let memory = Arc::new(Mutex::new(store));
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: Some(Arc::clone(&memory)),
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    let guard = memory.lock().await;
    let history = guard.history();
    assert_eq!(
        history.len(),
        2,
        "memory should have original turn plus new turn"
    );
    assert_eq!(history[0].0, "hi");
    assert_eq!(history[0].1, "hello");
    assert_eq!(history[1].0, "tell me something");
    assert_eq!(history[1].1, "Here you go.");
}

#[tokio::test]
async fn chat_turn_caps_history_to_two_recent_turns_for_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("latest question".to_string());
    let llm = HistoryCountLlm::new();
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::Chat);
    let limits = core_config::MemoryConfig::default();
    let mut store = MemoryStore::new(&limits);
    store.push_turn("u1", "a1");
    store.push_turn("u2", "a2");
    store.push_turn("u3", "a3");
    store.push_turn("u4", "a4");
    let memory = Arc::new(Mutex::new(store));
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: Some(Arc::clone(&memory)),
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(
        llm.last_history_len(),
        2,
        "chat path should pass only two most recent turns into LLM history"
    );
}

// --- Reminder skill tests ---

#[tokio::test]
async fn intent_reminder_no_when_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("remind me to buy milk".to_string());
    let reminder_result = core_skills::ReminderResult {
        summary: "Reminder 'buy milk' created without due date".to_string(),
        title: "buy milk".to_string(),
        when: None,
    };
    let reminder_skill = core_skills::MockReminderSkill::ok(reminder_result.clone());
    let llm = RecordLlm::new("I've set a reminder for you.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillReminder {
        title: Some("buy milk".to_string()),
        when: None,
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: Some(&reminder_skill),
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&reminder_result.to_prompt_context()),
        "LLM should receive reminder context, got: {}",
        llm.last_user_text()
    );
}

#[tokio::test]
async fn intent_reminder_with_when_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("remind me to call mom tomorrow at 5pm".to_string());
    let reminder_result = core_skills::ReminderResult {
        summary: "Reminder 'call mom' created for 20 Mar 2026 at 17:00".to_string(),
        title: "call mom".to_string(),
        when: Some("20 Mar 2026 at 17:00".to_string()),
    };
    let reminder_skill = core_skills::MockReminderSkill::ok(reminder_result.clone());
    let llm = RecordLlm::new("Done, reminder set.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillReminder {
        title: Some("call mom".to_string()),
        when: Some("2026-03-20T17:00".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: Some(&reminder_skill),
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&reminder_result.to_prompt_context()),
        "LLM should receive reminder context with due date, got: {}",
        llm.last_user_text()
    );
}

// --- Timer skill tests ---

#[tokio::test]
async fn intent_timer_named_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("set a pasta timer for 10 minutes".to_string());
    let timer_result = core_skills::TimerResult {
        summary: "Timer 'pasta timer' started for 10 minutes".to_string(),
        timer_name: "pasta timer".to_string(),
        duration_display: "10 minutes".to_string(),
        duration_seconds: 600,
    };
    let timer_skill = core_skills::MockTimerSkill::ok(timer_result.clone());
    let llm = RecordLlm::new("Pasta timer started.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillTimer {
        duration: Some("10 minutes".to_string()),
        name: Some("pasta timer".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: Some(&timer_skill),
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&timer_result.to_prompt_context()),
        "LLM should receive timer context, got: {}",
        llm.last_user_text()
    );
}

// --- Message skill tests ---

#[tokio::test]
async fn intent_message_routes_to_skill_and_speaks_deterministic_success() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("ask my wife how she is".to_string());
    let message_result = MessageResult {
        summary: "Sent iMessage to Jane Doe".to_string(),
        recipient_name: "Jane Doe".to_string(),
        recipient_handle: "+15551234567".to_string(),
        message: "How are you?".to_string(),
    };
    let message_skill = MockMessageSkill::ok(message_result.clone());
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillMessage {
        contact: Some("my wife".to_string()),
        message: Some("How are you?".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: Some(&message_skill),
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(tts.text(), "Sent your message to Jane Doe.");
}

#[tokio::test]
async fn intent_message_contact_not_found_speaks_deterministic_apology() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("ask my wife how she is".to_string());
    let message_skill =
        MockMessageSkill::err(MessageSkillError::ContactNotFound("my wife".to_string()));
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillMessage {
        contact: Some("my wife".to_string()),
        message: Some("How are you?".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: Some(&message_skill),
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(tts.text(), "I'm sorry, I couldn't tell who 'your wife' is.");
}

#[tokio::test]
async fn intent_message_send_failed_speaks_deterministic_error_without_llm() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("ask my wife how she is".to_string());
    let message_skill = MockMessageSkill::err(MessageSkillError::SendFailed(
        "service unavailable".to_string(),
    ));
    let llm = FailLlm;
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillMessage {
        contact: Some("my wife".to_string()),
        message: Some("How are you?".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: Some(&message_skill),
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert_eq!(
        tts.text(),
        "I'm sorry, I couldn't send an iMessage to 'your wife' right now."
    );
}

#[tokio::test]
async fn intent_timer_unnamed_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("set a timer for 5 minutes".to_string());
    let timer_result = core_skills::TimerResult {
        summary: "Timer 'first timer' started for 5 minutes".to_string(),
        timer_name: "first timer".to_string(),
        duration_display: "5 minutes".to_string(),
        duration_seconds: 300,
    };
    let timer_skill = core_skills::MockTimerSkill::ok(timer_result.clone());
    let llm = RecordLlm::new("Timer started.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillTimer {
        duration: Some("5 minutes".to_string()),
        name: None,
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: Some(&timer_skill),
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&timer_result.to_prompt_context()),
        "LLM should receive timer context, got: {}",
        llm.last_user_text()
    );
}

// --- Shopping list skill tests ---

#[tokio::test]
async fn intent_shopping_list_add_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("add strawberries and salami to the shopping list".to_string());
    let shopping_result = core_skills::ShoppingListResult {
        summary: "Updated 'Shopping List 19 Mar 2026'".to_string(),
        note_title: "Shopping List 19 Mar 2026".to_string(),
        added: vec!["strawberries".to_string(), "salami".to_string()],
        already_present: vec![],
        removed: vec![],
        not_found: vec![],
    };
    let shopping_skill = core_skills::MockShoppingListSkill::ok(shopping_result.clone());
    let llm = RecordLlm::new("Added to your shopping list.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillShoppingList {
        action: Some("add".to_string()),
        items: Some("strawberries and salami".to_string()),
        when: None,
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: None,
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: Some(&shopping_skill),
        volume_skill: None,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&shopping_result.to_prompt_context()),
        "LLM should receive shopping list context, got: {}",
        llm.last_user_text()
    );
    assert!(tts.text().contains("Added"));
}

// --- App switcher skill tests ---

#[tokio::test]
async fn intent_app_switcher_switch_routes_to_skill() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("switch to safari".to_string());
    let app_switcher_result = core_skills::AppSwitcherResult {
        summary: "Done. I switched to Safari.".to_string(),
        action_done: "activate Safari".to_string(),
        target: Some("Safari".to_string()),
    };
    let app_switcher_skill = core_skills::MockAppSwitcherSkill::ok(app_switcher_result.clone());
    let llm = RecordLlm::new("Switched to Safari.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillAppSwitcher {
        action: Some("switch".to_string()),
        target: Some("Safari".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: Some(&app_switcher_skill),
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&app_switcher_result.to_prompt_context()),
        "LLM should receive app switcher context, got: {}",
        llm.last_user_text()
    );
}

#[tokio::test]
async fn intent_app_switcher_force_quit_requires_confirmation_and_yes_executes() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = QueueStt::new(vec!["force quit safari", "yes do it"]);
    let app_switcher_result = core_skills::AppSwitcherResult {
        summary: "Done. I force-quit Safari.".to_string(),
        action_done: "force quit Safari".to_string(),
        target: Some("Safari".to_string()),
    };
    let app_switcher_skill = core_skills::MockAppSwitcherSkill::ok(app_switcher_result.clone());
    let llm = QueueLlm::new(vec![
        r#"{"confirm":"yes"}"#,
        "I force-quit Safari as requested.",
    ]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillAppSwitcher {
        action: Some("force_quit".to_string()),
        target: Some("Safari".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: Some(&app_switcher_skill),
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx1) = tokio::sync::broadcast::channel(1);
    let (_tx2, rx2) = tokio::sync::broadcast::channel(1);

    let first_outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx1, &skills)
        .await
        .must();
    assert_eq!(first_outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.text().to_lowercase().contains("confirm"),
        "first turn should ask confirmation, got: {}",
        tts.text()
    );

    let second_outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx2, &skills)
        .await
        .must();
    assert_eq!(second_outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.text().to_lowercase().contains("force-quit"),
        "second turn should execute force quit after yes confirmation, got: {}",
        tts.text()
    );
}

#[tokio::test]
async fn intent_app_switcher_force_quit_confirmation_no_cancels() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = QueueStt::new(vec!["force quit safari", "no"]);
    let app_switcher_result = core_skills::AppSwitcherResult {
        summary: "Done. I force-quit Safari.".to_string(),
        action_done: "force quit Safari".to_string(),
        target: Some("Safari".to_string()),
    };
    let app_switcher_skill = core_skills::MockAppSwitcherSkill::ok(app_switcher_result);
    let llm = QueueLlm::new(vec![r#"{"confirm":"no"}"#]);
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillAppSwitcher {
        action: Some("force_quit".to_string()),
        target: Some("Safari".to_string()),
    });
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: Some(&app_switcher_skill),
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx1) = tokio::sync::broadcast::channel(1);
    let (_tx2, rx2) = tokio::sync::broadcast::channel(1);

    let first_outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx1, &skills)
        .await
        .must();
    assert_eq!(first_outcome, RuntimeTurnOutcome::Complete);

    let second_outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx2, &skills)
        .await
        .must();
    assert_eq!(second_outcome, RuntimeTurnOutcome::Complete);
    assert!(
        tts.text().to_lowercase().contains("cancelled"),
        "second turn should cancel force quit after no confirmation, got: {}",
        tts.text()
    );
}

#[tokio::test]
async fn intent_app_switcher_force_quit_policy_denied_falls_back_to_chat() {
    let config = Config::default();
    let mut runtime = DesktopRuntime::new(config);
    runtime.activate_wake();
    let mut stt = MockStt("force quit safari".to_string());
    let app_switcher_result = core_skills::AppSwitcherResult {
        summary: "Done. I force-quit Safari.".to_string(),
        action_done: "force quit Safari".to_string(),
        target: Some("Safari".to_string()),
    };
    let app_switcher_skill = core_skills::MockAppSwitcherSkill::ok(app_switcher_result);
    let llm = RecordLlm::new("I cannot do that right now.");
    let mut tts = MockTts::new();
    let classifier = MockIntentClassifier(IntentDecision::SkillAppSwitcher {
        action: Some("force_quit".to_string()),
        target: Some("Safari".to_string()),
    });
    let deny_policy = DenyPolicy;
    let skills = SkillRunContext {
        intent_classifier: Some(&classifier),
        weather_skill: None::<&dyn WeatherSkill>,
        time_skill: None::<&dyn core_skills::TimeSkill>,
        distance_skill: None::<&dyn core_skills::DistanceSkill>,
        smart_home_skill: None::<&dyn core_skills::SmartHomeSkill>,
        assistant_skill: None::<&dyn core_skills::AssistantSkill>,
        media_skill: None::<&dyn core_skills::MediaSkill>,
        memory_skill: None::<&dyn core_skills::MemorySkill>,
        computer_skill: None::<&dyn core_skills::ComputerSkill>,
        app_switcher_skill: Some(&app_switcher_skill),
        reminder_skill: None::<&dyn core_skills::ReminderSkill>,
        message_skill: None::<&dyn core_skills::MessageSkill>,
        timer_skill: None::<&dyn core_skills::TimerSkill>,
        shopping_list_skill: None::<&dyn core_skills::ShoppingListSkill>,
        volume_skill: None::<&dyn core_skills::VolumeSkill>,
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: Some(&deny_policy),
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .must();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text().contains("force quit safari"),
        "policy denial should fall back to chat path"
    );
}
