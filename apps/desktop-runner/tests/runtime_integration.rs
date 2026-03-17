//! Runtime integration: gate, empty input, needs-search yes/no, interrupt, intent+weather.

use async_trait::async_trait;
use core_audio::{AudioCapture, CaptureError};
use core_config::Config;
use core_orchestrator::{IntentClassifier, IntentDecision, LlmStream, SttStream, TtsSink};
use core_search::MockSearch;
use core_skills::{
    DistanceResult, MediaResult, MockDistanceSkill, MockMediaSkill, MockSmartHomeSkill,
    MockTimeSkill, MockWeatherSkill, ResolvedLocation, SmartHomeResult, TimeResult, WeatherResult,
    WeatherSkill,
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
        self.last_user_text.lock().unwrap().clone()
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
        *self.last_user_text.lock().unwrap() = user_text.to_string();
        Ok(Box::new(stream::iter(vec![self.response.clone()])))
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
        .unwrap();

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
        .unwrap();

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
        .unwrap();

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
            .unwrap();

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
        .unwrap();

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
        .unwrap();

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
        .unwrap();
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
        .unwrap();

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
        .unwrap();
    std::fs::write(
        temp.path(),
        "const unsigned char audio_chocobo[] = { 0x01, 0x00, 0x02, 0x00 };",
    )
    .unwrap();
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
        .unwrap();

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
        .unwrap();
    std::fs::write(
        temp.path(),
        "const unsigned char audio_chocobo[] = { 0x10, 0x00, 0x20, 0x00 };",
    )
    .unwrap();
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
        .unwrap();

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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .expect("runtime_continuous_loop_processes_multiple_turns timed out")
    .unwrap();

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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .expect("runtime_continuous_loop_activates_on_wake_phrase timed out")
    .unwrap();

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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        )
        .await
        .unwrap();

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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        )
        .await
        .unwrap();

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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await;

    let stats = result.expect("runtime should not hang waiting for full turn window");
    let stats = stats.expect("runtime error");
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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .expect("runtime should complete once silence pause is observed")
    .expect("runtime error");

    assert_eq!(result.turns_completed, 1);
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
                resolved_location: None,
                memory: None,
                policy: None,
            },
        },
    );
    let stats = tokio::time::timeout(std::time::Duration::from_millis(400), run)
        .await
        .expect_err("second echo turn should be ignored and not complete");
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
                    resolved_location: None,
                    memory: None,
                    policy: None,
                },
            },
        ),
    )
    .await
    .expect("runtime_does_not_ignore_stop_as_echo timed out")
    .unwrap();

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
                resolved_location: None,
                memory: None,
                policy: None,
            },
        },
    );

    let _ = tokio::time::timeout(std::time::Duration::from_millis(400), run)
        .await
        .expect_err("second turn without wake phrase should not be processed");
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
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(
        llm.last_user_text()
            .contains(&weather_result.to_prompt_context()),
        "LLM should receive weather context in prompt, got: {}",
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
    let llm = RecordLlm::new("Sunny and 22 in Rome.");
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
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

    assert_eq!(outcome, RuntimeTurnOutcome::Complete);
    assert!(tts.text().contains("22"));
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
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
        resolved_location: None::<&ResolvedLocation>,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
        resolved_location: Some(&resolved),
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
        resolved_location: None,
        memory: None,
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
        resolved_location: None::<&ResolvedLocation>,
        memory: Some(Arc::clone(&memory)),
        policy: None,
    };
    let (_tx, rx) = tokio::sync::broadcast::channel(1);

    let outcome = runtime
        .run_one_turn_with_skills(&mut stt, &llm, &mut tts, None::<&MockSearch>, rx, &skills)
        .await
        .unwrap();

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
