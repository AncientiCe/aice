pub mod discovery_broadcast;
pub mod llm_adapters;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{Datelike, NaiveDate, Utc};
use core_config::{Config, WakeWordConfig};
use core_llm::CradleLlmStream;
use core_observability::{
    record_air_quality_skill, record_backend_audio_chunk, record_backend_dependency_request,
    record_backend_dependency_request_duration, record_backend_http_request,
    record_backend_llm_provider_duration, record_backend_skill_execute,
    record_backend_skill_execute_duration, record_backend_turn_cancellation,
    record_backend_turn_duration, record_backend_turn_first_token_duration,
    record_backend_turn_partial_transcript_duration, record_backend_turn_stage_duration,
    record_backend_turn_total, record_briefing_skill, record_calculator_skill,
    record_calendar_skill, record_currency_skill, record_dictionary_skill, record_email_skill,
    record_journal_skill, record_meeting_notes_skill, record_model_preload,
    record_model_preload_duration, record_palace_error, record_palace_ingest, record_palace_open,
    record_palace_wake_up, record_screen_ocr_skill, record_translate_skill,
    record_unit_conversion_skill,
};
use core_orchestrator::{
    intent_classifier_few_shots_for_skills, intent_classifier_json_schema_for_skills,
    intent_classifier_system_prompt_for_skills, parse_intent, validate_intent_decision,
    IntentClassifier, IntentDecision, LlmCallOptions, LlmStream,
};
use core_runtime_protocol::{
    FrontendSkillIntent, FrontendSkillResultRequest, TurnRequest, TurnStreamClientMessage,
    TurnStreamServerEvent,
};
use core_skills::{
    collect_news_summaries, AirQualityError, AirQualityLocation, AirQualityResult, AirQualitySkill,
    BriefingError, BriefingQuery, BriefingResult, BriefingSkill, CalculatorResult, CalculatorSkill,
    CalculatorSkillError, ComposedBriefingSkill, ConversionResult, CurrencyError, CurrencyQuery,
    CurrencyResult, CurrencySkill, DictionaryError, DictionaryResult, DictionarySkill,
    DistanceResult, DistanceSkill, FuelPriceLookupError, FuelPriceLookupQuery,
    FuelPriceLookupResult, FuelPriceLookupSkill, HolidayLookupError, HolidayLookupResult,
    HolidayLookupSkill, HolidayQuery, HoroscopeDailyError, HoroscopeDailyQuery,
    HoroscopeDailyResult, HoroscopeDailySkill, HttpAirQualitySkill, HttpCurrencySkill,
    HttpDictionarySkill, HttpFuelPriceLookupSkill, HttpHolidayLookupSkill, HttpHoroscopeDailySkill,
    HttpNewsHeadlinesSkill, HttpSportsLiveSkill, JournalAction, JournalError, JournalResult,
    JournalSkill, LlmMeetingNotesSkill, LlmTranslateSkill, LocalCalculatorSkill, LocalJournalSkill,
    LocalUnitConversionSkill, MeetingNotesError, MeetingNotesLlm, MeetingNotesQuery,
    MeetingNotesResult, MeetingNotesSkill, NewsHeadlinesError, NewsHeadlinesQuery,
    NewsHeadlinesResult, NewsHeadlinesSkill, NewsSummaryLlm, OpenMeteoDistanceSkill,
    OpenMeteoTimeSkill, OpenMeteoWeatherSkill, ResolvedLocation, ScreenOcrLlm, Sentiment,
    SportsLiveError, SportsLiveQuery, SportsLiveResult, SportsLiveSkill, SqliteJournalStore,
    SummarizedHeadline, TimeResult, TimeSkill, TranslateError, TranslateQuery, TranslateResult,
    TranslateSkill, TranslationLlm, UnitConversionError, UnitConversionSkill, WeatherResult,
    WeatherSkill, ENABLED_SKILL_IDS,
};
use core_stt::WhisperSttStream;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mempalace::palace::Palace;
use serde_json::{json, Value};
use skill_chain::EffectiveCapabilities;
use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, error, info, warn};

pub type DynError = Box<dyn Error + Send + Sync>;
type RespBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;
type PalaceHandle = Arc<std::sync::Mutex<Palace>>;
type FrontendSessions = Arc<Mutex<HashMap<FrontendSessionKey, FrontendSessionState>>>;
type ClassifierPromptCache = Arc<RwLock<HashMap<String, ClassifierPromptArtifacts>>>;

const MAX_BINARY_FRAME_BYTES: usize = 32_768;

/// Minimum pending samples before running incremental STT on append-only
/// buffered audio. At 16 kHz this is 100 ms.
const STT_DEBOUNCE_SAMPLES: usize = 1_600;
/// Minimum wall-clock gap between incremental STT runs to avoid repeatedly
/// re-transcribing the full turn buffer under sustained audio chunk ingress.
const STT_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(1_200);
/// Intents the backend dispatches to a connected frontend (e.g. `aice-macos`)
/// rather than executing in-process.
///
/// Frontends advertise their actually-supported subset per session via
/// `TurnStreamClientMessage::TurnStart::supported_frontend_intents`; the
/// backend then gates outbound `FrontendSkillIntent` events with
/// `frontend_intent_allowed`. This list only constrains *which* intent names
/// the classifier may target as frontend skills, regardless of which
/// frontend(s) happen to be connected.
const FRONTEND_CLASSIFIER_SKILLS: [&str; 13] = [
    "skill_smart_home",
    "skill_media",
    "skill_computer",
    "skill_screenshot",
    "skill_app_switcher",
    "skill_reminder",
    "skill_timer",
    "skill_shopping_list",
    "skill_message",
    "skill_volume",
    "skill_calendar",
    "skill_email",
    "skill_screen_ocr",
];

#[derive(Clone, Debug)]
struct ClassifierPromptArtifacts {
    system_prompt: String,
    compact_few_shots: Vec<(String, String)>,
    json_schema: Option<serde_json::Value>,
}

/// Hardcoded classifier output token cap. Kept small for decode-speed on
/// memory-bandwidth-limited devices (each token costs ~25ms on M4 Mini with 7B).
const CLASSIFIER_MAX_OUTPUT_TOKENS: u32 = 24;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FrontendSessionKey {
    device_id: String,
    session_id: String,
}

#[derive(Clone, Debug)]
struct FrontendSessionState {
    _supported_frontend_intents: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AudioIngressConfig {
    pub wake_word: WakeWordConfig,
}

impl AudioIngressConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            wake_word: config.wake_word.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BackendEngineDecision {
    Chat(String),
    BackendSkill(String),
    FrontendSkillIntent(FrontendSkillIntent),
}

#[async_trait]
pub trait BackendEngine: Send + Sync {
    async fn process_turn(&self, request: TurnRequest) -> Result<BackendEngineDecision, DynError>;

    async fn finalize_frontend_skill(
        &self,
        turn_id: &str,
        intent_id: &str,
        request: FrontendSkillResultRequest,
    ) -> Result<String, DynError>;
}

#[async_trait]
pub trait AudioTranscriber: Send + Sync {
    async fn transcribe(
        &self,
        samples: Vec<i16>,
        sample_rate_hz: u32,
        channels: u16,
    ) -> Result<String, DynError>;
}

pub struct WhisperAudioTranscriber {
    stt: Arc<std::sync::Mutex<WhisperSttStream>>,
}

impl WhisperAudioTranscriber {
    pub fn new(model_path: impl Into<String>, preload_on_startup: bool) -> Result<Self, DynError> {
        let path: String = model_path.into();
        let mut stt = WhisperSttStream::new(Path::new(&path))
            .map_err(|error| format!("failed to load whisper model: {error}"))?;
        if preload_on_startup {
            let t0 = Instant::now();
            match stt.warm_up() {
                Ok(()) => {
                    record_model_preload_duration("stt", t0.elapsed());
                    record_model_preload("stt", "success");
                }
                Err(error) => {
                    record_model_preload_duration("stt", t0.elapsed());
                    record_model_preload("stt", "error");
                    tracing::warn!(%error, "stt preload failed; continuing without startup warmup");
                }
            }
        }
        Ok(Self {
            stt: Arc::new(std::sync::Mutex::new(stt)),
        })
    }
}

#[async_trait]
impl AudioTranscriber for WhisperAudioTranscriber {
    async fn transcribe(
        &self,
        samples: Vec<i16>,
        sample_rate_hz: u32,
        channels: u16,
    ) -> Result<String, DynError> {
        if sample_rate_hz != 16_000 || channels != 1 {
            return Err(format!(
                "unsupported audio format: sample_rate_hz={sample_rate_hz}, channels={channels}"
            )
            .into());
        }
        let stt = self.stt.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = stt
                .lock()
                .map_err(|error| format!("stt mutex poisoned: {error}"))?;
            guard
                .transcribe_blocking(&samples)
                .map(|t| t.trim().to_string())
                .map_err(|error| -> DynError { error.into() })
        })
        .await
        .map_err(|error| -> DynError { format!("stt task join: {error}").into() })?
    }
}

pub struct NullAudioTranscriber;

#[async_trait]
impl AudioTranscriber for NullAudioTranscriber {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, DynError> {
        Ok(String::new())
    }
}

pub struct ServerHandle {
    pub bind: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ServerHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn spawn_server(
    bind: &str,
    engine: Arc<dyn BackendEngine>,
) -> Result<ServerHandle, DynError> {
    spawn_server_with_audio(
        bind,
        engine,
        Arc::new(NullAudioTranscriber),
        AudioIngressConfig::default(),
    )
    .await
}

pub async fn spawn_server_with_audio(
    bind: &str,
    engine: Arc<dyn BackendEngine>,
    transcriber: Arc<dyn AudioTranscriber>,
    audio_config: AudioIngressConfig,
) -> Result<ServerHandle, DynError> {
    let listener = TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let frontend_sessions: FrontendSessions = Arc::new(Mutex::new(HashMap::new()));
    let audio_config = Arc::new(audio_config);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else {
                        continue;
                    };
                    let io = TokioIo::new(stream);
                    let engine = engine.clone();
                    let frontend_sessions = frontend_sessions.clone();
                    let transcriber = transcriber.clone();
                    let audio_config = audio_config.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let engine = engine.clone();
                            let frontend_sessions = frontend_sessions.clone();
                            let transcriber = transcriber.clone();
                            let audio_config = audio_config.clone();
                            async move {
                                handle_request(
                                    req,
                                    engine,
                                    frontend_sessions,
                                    transcriber,
                                    audio_config,
                                )
                                .await
                            }
                        });
                        if let Err(error) = http1::Builder::new()
                            .serve_connection(io, service)
                            .with_upgrades()
                            .await
                        {
                            warn!(%error, "backend connection failed");
                        }
                    });
                }
            }
        }
    });

    Ok(ServerHandle {
        bind: local.to_string(),
        shutdown_tx: Some(shutdown_tx),
    })
}

fn full_body(body: String) -> RespBody {
    Full::new(Bytes::from(body)).boxed()
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<RespBody> {
    let body = match serde_json::to_string(&value) {
        Ok(v) => v,
        Err(_) => "{}".to_string(),
    };
    let mut response = Response::new(full_body(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response
}

fn plain_response(status: StatusCode, body: &str) -> Response<RespBody> {
    let mut response = Response::new(full_body(body.to_string()));
    *response.status_mut() = status;
    response
}

fn with_backend_http_metrics(
    method: &Method,
    route: &str,
    started_at: Instant,
    response: Response<RespBody>,
) -> Response<RespBody> {
    record_backend_http_request(
        route,
        method.as_str(),
        response.status().as_u16(),
        started_at.elapsed(),
    );
    response
}

fn normalize_frontend_intent(intent: &str) -> Option<String> {
    let value = intent.trim().to_lowercase();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn normalize_supported_frontend_intents(intents: &[String]) -> Vec<String> {
    let mut normalized = intents
        .iter()
        .filter_map(|intent| normalize_frontend_intent(intent))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn build_frontend_session_key(device_id: &str, session_id: &str) -> Option<FrontendSessionKey> {
    let device = device_id.trim();
    let session = session_id.trim();
    if device.is_empty() || session.is_empty() {
        return None;
    }
    Some(FrontendSessionKey {
        device_id: device.to_string(),
        session_id: session.to_string(),
    })
}

async fn register_ws_session(
    sessions: &FrontendSessions,
    device_id: &str,
    session_id: &str,
    supported_frontend_intents: &[String],
) {
    if let Some(key) = build_frontend_session_key(device_id, session_id) {
        let intents = normalize_supported_frontend_intents(supported_frontend_intents);
        let mut guard = sessions.lock().await;
        guard.insert(
            key,
            FrontendSessionState {
                _supported_frontend_intents: intents,
            },
        );
    }
}

async fn remove_ws_session(sessions: &FrontendSessions, device_id: &str, session_id: &str) {
    if let Some(key) = build_frontend_session_key(device_id, session_id) {
        let mut guard = sessions.lock().await;
        guard.remove(&key);
    }
}

fn frontend_intent_allowed(intent: &str, supported_frontend_intents: Option<&[String]>) -> bool {
    let Some(intents) = supported_frontend_intents else {
        return true;
    };
    let Some(normalized) = normalize_frontend_intent(intent) else {
        return false;
    };
    intents.iter().any(|allowed| allowed == &normalized)
}

fn build_intent_classification_prompt(user_text: &str) -> String {
    format!(
        "Classify this user request. Reply with only the JSON object.\nUser request: \"{}\"",
        user_text.trim()
    )
}

fn frontend_classifier_skills_from_context(context: Option<&Value>) -> Vec<String> {
    let Some(value) = context else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let Some(intents) = object.get("frontend_supported_intents") else {
        return Vec::new();
    };
    let Some(intents_array) = intents.as_array() else {
        return Vec::new();
    };
    let mut normalized: Vec<String> = intents_array
        .iter()
        .filter_map(|entry| entry.as_str())
        .filter_map(normalize_frontend_intent)
        .filter(|intent| FRONTEND_CLASSIFIER_SKILLS.contains(&intent.as_str()))
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn build_available_classifier_skills(context: Option<&Value>) -> Vec<String> {
    let frontend_intents = frontend_classifier_skills_from_context(context);
    let backend_skills: Vec<String> = ENABLED_SKILL_IDS
        .iter()
        .map(|skill| skill.to_string())
        .collect();
    let capabilities = EffectiveCapabilities::from_handshake(frontend_intents, backend_skills);
    capabilities.classifier_enabled_skills
}

fn classifier_cache_key(available_skills: &[&str]) -> String {
    let mut normalized = available_skills
        .iter()
        .map(|skill| skill.trim().to_ascii_lowercase())
        .filter(|skill| !skill.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized.join("|")
}

fn build_classifier_prompt_artifacts(available_skills: &[&str]) -> ClassifierPromptArtifacts {
    ClassifierPromptArtifacts {
        system_prompt: intent_classifier_system_prompt_for_skills(available_skills),
        compact_few_shots: intent_classifier_few_shots_for_skills(available_skills),
        json_schema: Some(intent_classifier_json_schema_for_skills(available_skills)),
    }
}

fn parse_validated_intent(raw: &str) -> Option<IntentDecision> {
    parse_intent(raw.trim()).ok().map(validate_intent_decision)
}

fn apply_backend_wake_word(config: &WakeWordConfig, transcript: String) -> String {
    let trimmed = transcript.trim();
    if !config.enabled {
        return trimmed.to_string();
    }
    if trimmed.is_empty() {
        return String::new();
    }
    let lowered = trimmed.to_lowercase();
    for phrase in &config.phrases {
        let normalized_phrase = phrase.trim().to_lowercase();
        if normalized_phrase.is_empty() {
            continue;
        }
        if let Some(remainder) = lowered.strip_prefix(&normalized_phrase) {
            let remainder = &trimmed[trimmed.len() - remainder.len()..];
            return remainder
                .trim_matches(|c: char| c == ',' || c == ':' || c.is_whitespace())
                .to_string();
        }
    }
    String::new()
}

#[derive(Debug)]
struct WsTurnState {
    session_id: String,
    device_id: Option<String>,
    turn_id: String,
    supported_frontend_intents: Vec<String>,
    samples: Vec<i16>,
    last_incremental_stt_at: Option<Instant>,
    transcript_accum: String,
    started_at: Instant,
    done_requested: bool,
    first_partial_recorded: bool,
    first_token_recorded: bool,
    active_generation: u64,
    active_transcript: Option<String>,
    completed_generation: Option<u64>,
    active_task: Option<tokio::task::JoinHandle<()>>,
    awaiting_skill_result: bool,
}

impl WsTurnState {
    fn new(
        session_id: String,
        device_id: Option<String>,
        turn_id: String,
        supported_frontend_intents: Vec<String>,
    ) -> Self {
        Self {
            session_id,
            device_id,
            turn_id,
            supported_frontend_intents,
            samples: Vec::new(),
            last_incremental_stt_at: None,
            transcript_accum: String::new(),
            started_at: Instant::now(),
            done_requested: false,
            first_partial_recorded: false,
            first_token_recorded: false,
            active_generation: 0,
            active_transcript: None,
            completed_generation: None,
            active_task: None,
            awaiting_skill_result: false,
        }
    }
}

#[derive(Debug)]
enum TurnStreamInternalEvent {
    SpeculativeFinished {
        turn_id: String,
        generation: u64,
        result: Result<BackendEngineDecision, String>,
        duration: Duration,
    },
}

fn websocket_upgrade_requested(req: &Request<Incoming>) -> bool {
    let Some(upgrade) = req.headers().get(hyper::header::UPGRADE) else {
        return false;
    };
    let Some(connection) = req.headers().get(hyper::header::CONNECTION) else {
        return false;
    };
    let upgrade_value = upgrade.to_str().unwrap_or_default().to_ascii_lowercase();
    let connection_value = connection.to_str().unwrap_or_default().to_ascii_lowercase();
    upgrade_value.contains("websocket") && connection_value.contains("upgrade")
}

fn websocket_switching_response(accept_key: String) -> Response<RespBody> {
    let mut response = Response::new(full_body(String::new()));
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    response.headers_mut().insert(
        hyper::header::UPGRADE,
        hyper::header::HeaderValue::from_static("websocket"),
    );
    response.headers_mut().insert(
        hyper::header::CONNECTION,
        hyper::header::HeaderValue::from_static("Upgrade"),
    );
    if let Ok(value) = hyper::header::HeaderValue::from_str(&accept_key) {
        response.headers_mut().insert("Sec-WebSocket-Accept", value);
    }
    response
}

async fn emit_turn_stream_event(
    ws: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    event: &TurnStreamServerEvent,
) -> Result<(), DynError> {
    ws.send(Message::Text(serde_json::to_string(event)?))
        .await
        .map_err(|error| format!("ws send failed: {error}"))?;
    Ok(())
}

async fn emit_decision_events(
    ws: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    turn_id: &str,
    decision: BackendEngineDecision,
) -> Result<bool, DynError> {
    match decision {
        BackendEngineDecision::Chat(text) | BackendEngineDecision::BackendSkill(text) => {
            info!(
                turn_id,
                token_chars = text.chars().count(),
                "streaming_response_to_frontend"
            );
            emit_turn_stream_event(
                ws,
                &TurnStreamServerEvent::Token {
                    turn_id: turn_id.to_string(),
                    text,
                },
            )
            .await?;
            Ok(true)
        }
        BackendEngineDecision::FrontendSkillIntent(intent) => {
            info!(
                turn_id,
                intent = %intent.intent,
                "streaming_response_to_frontend"
            );
            emit_turn_stream_event(ws, &TurnStreamServerEvent::FrontendSkillIntent(intent)).await?;
            Ok(false)
        }
    }
}

fn spawn_speculative_turn(
    state: &mut WsTurnState,
    engine: Arc<dyn BackendEngine>,
    transcript: String,
    tx: mpsc::UnboundedSender<TurnStreamInternalEvent>,
) {
    state.active_generation = state.active_generation.saturating_add(1);
    let generation = state.active_generation;
    let turn_id = state.turn_id.clone();
    info!(
        turn_id = %turn_id,
        generation,
        since_turn_start_ms = state.started_at.elapsed().as_millis(),
        "calling_llm_for_classification"
    );
    let context = if state.supported_frontend_intents.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "frontend_supported_intents": state.supported_frontend_intents,
        }))
    };
    let request = TurnRequest {
        session_id: state.session_id.clone(),
        device_id: state.device_id.clone(),
        turn_id: Some(turn_id.clone()),
        transcript,
        finalize: false,
        context,
    };
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let result = engine
            .process_turn(request)
            .await
            .map_err(|error| error.to_string());
        record_backend_llm_provider_duration("cradle", started.elapsed());
        let _ = tx.send(TurnStreamInternalEvent::SpeculativeFinished {
            turn_id,
            generation,
            result,
            duration: started.elapsed(),
        });
    });
    state.active_task = Some(task);
}

async fn process_binary_audio_frame(
    ws: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    state: &mut WsTurnState,
    _engine: Arc<dyn BackendEngine>,
    transcriber: &Arc<dyn AudioTranscriber>,
    audio_config: &AudioIngressConfig,
    _internal_tx: &mpsc::UnboundedSender<TurnStreamInternalEvent>,
    raw: &[u8],
) -> Result<(), DynError> {
    if state.done_requested {
        // Frontend already declared turn completion; ignore trailing audio frames
        // so we can finalize quickly.
        return Ok(());
    }
    if !raw.len().is_multiple_of(2) {
        emit_turn_stream_event(
            ws,
            &TurnStreamServerEvent::Error {
                turn_id: Some(state.turn_id.clone()),
                message: "binary frame byte length must be even (i16 LE)".to_string(),
            },
        )
        .await?;
        return Ok(());
    }
    if raw.len() > MAX_BINARY_FRAME_BYTES {
        emit_turn_stream_event(
            ws,
            &TurnStreamServerEvent::Error {
                turn_id: Some(state.turn_id.clone()),
                message: format!(
                    "binary frame too large: {} bytes (max {})",
                    raw.len(),
                    MAX_BINARY_FRAME_BYTES
                ),
            },
        )
        .await?;
        return Ok(());
    }
    let chunk_started = Instant::now();
    for frame in raw.chunks_exact(2) {
        state.samples.push(i16::from_le_bytes([frame[0], frame[1]]));
    }
    record_backend_audio_chunk(raw.len(), chunk_started.elapsed());

    let first_incremental_decode = state.last_incremental_stt_at.is_none();
    if !first_incremental_decode && state.samples.len() < STT_DEBOUNCE_SAMPLES {
        return Ok(());
    }
    if !first_incremental_decode {
        if let Some(last) = state.last_incremental_stt_at {
            if last.elapsed() < STT_DEBOUNCE_INTERVAL {
                return Ok(());
            }
        }
    }

    let stt_started = Instant::now();
    state.last_incremental_stt_at = Some(stt_started);
    let pending_samples = std::mem::take(&mut state.samples);
    let transcript = transcriber
        .transcribe(pending_samples, 16_000, 1)
        .await
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let stt_elapsed = stt_started.elapsed();
    record_backend_turn_stage_duration("stt_incremental", stt_elapsed);
    info!(
        turn_id = %state.turn_id,
        stt_elapsed_ms = stt_elapsed.as_millis(),
        since_turn_start_ms = state.started_at.elapsed().as_millis(),
        "turn_stt_ready"
    );

    if !transcript.is_empty() {
        if !state.transcript_accum.is_empty() {
            state.transcript_accum.push(' ');
        }
        state.transcript_accum.push_str(&transcript);
    }
    let transcript =
        apply_backend_wake_word(&audio_config.wake_word, state.transcript_accum.clone());
    if transcript.is_empty() {
        return Ok(());
    }

    emit_turn_stream_event(
        ws,
        &TurnStreamServerEvent::PartialTranscript {
            turn_id: state.turn_id.clone(),
            text: transcript.clone(),
            stable: true,
        },
    )
    .await?;
    if !state.first_partial_recorded {
        state.first_partial_recorded = true;
        record_backend_turn_partial_transcript_duration(state.started_at.elapsed());
    }

    if state.active_transcript.as_deref() != Some(transcript.as_str()) {
        state.active_transcript = Some(transcript);
    }
    Ok(())
}

async fn handle_turn_stream_socket(
    mut ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    engine: Arc<dyn BackendEngine>,
    transcriber: Arc<dyn AudioTranscriber>,
    frontend_sessions: FrontendSessions,
    audio_config: Arc<AudioIngressConfig>,
) -> Result<(), DynError> {
    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel::<TurnStreamInternalEvent>();
    let mut turn: Option<WsTurnState> = None;
    let mut ws_session_id: Option<String> = None;
    let mut ws_device_id: Option<String> = None;

    let result = async {
        loop {
            tokio::select! {
                maybe_msg = ws.next() => {
                    let Some(msg) = maybe_msg else {
                        break;
                    };
                    let msg = msg.map_err(|error| format!("ws read failed: {error}"))?;
                    match msg {
                        Message::Binary(raw) => {
                            let Some(state) = turn.as_mut() else {
                                emit_turn_stream_event(
                                    &mut ws,
                                    &TurnStreamServerEvent::Error {
                                        turn_id: None,
                                        message: "binary audio before turn_start".to_string(),
                                    },
                                )
                                .await?;
                                continue;
                            };
                            process_binary_audio_frame(
                                &mut ws,
                                state,
                                engine.clone(),
                                &transcriber,
                                &audio_config,
                                &internal_tx,
                                &raw,
                            )
                            .await?;
                        }
                        Message::Text(payload) => {
                            let incoming: TurnStreamClientMessage = match serde_json::from_str(payload.as_ref()) {
                                Ok(value) => value,
                                Err(error) => {
                                    emit_turn_stream_event(
                                        &mut ws,
                                        &TurnStreamServerEvent::Error {
                                            turn_id: turn.as_ref().map(|state| state.turn_id.clone()),
                                            message: format!("invalid message: {error}"),
                                        },
                                    )
                                    .await?;
                                    continue;
                                }
                            };

                            match incoming {
                                TurnStreamClientMessage::TurnStart {
                                    session_id,
                                    device_id,
                                    turn_id,
                                    supported_frontend_intents,
                                    ..
                                } => {
                                    if let Some(mut previous) = turn.take() {
                                        if let Some(task) = previous.active_task.take() {
                                            task.abort();
                                        }
                                    }
                                    let dev = device_id.clone().unwrap_or_else(|| "unknown".to_string());
                                    register_ws_session(
                                        &frontend_sessions,
                                        &dev,
                                        &session_id,
                                        &supported_frontend_intents,
                                    )
                                    .await;
                                    ws_session_id = Some(session_id.clone());
                                    ws_device_id = Some(dev);
                                    let intents = normalize_supported_frontend_intents(&supported_frontend_intents);
                                    info!(
                                        turn_id = %turn_id,
                                        session_id = %session_id,
                                        supported_frontend_intents = intents.len(),
                                        "turn_start"
                                    );
                                    turn = Some(WsTurnState::new(session_id, device_id, turn_id, intents));
                                }
                                TurnStreamClientMessage::TurnDone => {
                                    let raw_since_turn_start_ms = turn
                                        .as_ref()
                                        .map(|state| state.started_at.elapsed().as_millis());
                                    if let Some(ms) = raw_since_turn_start_ms {
                                        info!(
                                            since_turn_start_ms = ms,
                                            has_active_turn = true,
                                            "turn_done_received_raw"
                                        );
                                    } else {
                                        info!(has_active_turn = false, "turn_done_received_raw");
                                    }
                                    let Some(state) = turn.as_mut() else {
                                        emit_turn_stream_event(
                                            &mut ws,
                                            &TurnStreamServerEvent::Error {
                                                turn_id: None,
                                                message: "turn_done before turn_start".to_string(),
                                            },
                                        )
                                        .await?;
                                        continue;
                                    };
                                    info!(
                                        turn_id = %state.turn_id,
                                        since_turn_start_ms = state.started_at.elapsed().as_millis(),
                                        "turn_done_received"
                                    );
                                    state.done_requested = true;
                                    if !state.samples.is_empty() {
                                        let final_transcript_chunk = transcriber
                                            .transcribe(std::mem::take(&mut state.samples), 16_000, 1)
                                            .await
                                            .map(|value| value.trim().to_string())
                                            .unwrap_or_default();
                                        if !final_transcript_chunk.is_empty() {
                                            if !state.transcript_accum.is_empty() {
                                                state.transcript_accum.push(' ');
                                            }
                                            state.transcript_accum.push_str(&final_transcript_chunk);
                                        }
                                        let final_transcript = apply_backend_wake_word(
                                            &audio_config.wake_word,
                                            state.transcript_accum.clone(),
                                        );
                                        if !final_transcript.is_empty() {
                                            state.active_transcript = Some(final_transcript);
                                        }
                                    }
                                    if state.active_task.is_none() && !state.awaiting_skill_result {
                                        let transcript = state
                                            .active_transcript
                                            .clone()
                                            .unwrap_or_else(|| {
                                                apply_backend_wake_word(
                                                    &audio_config.wake_word,
                                                    state.transcript_accum.clone(),
                                                )
                                            });
                                        if !transcript.is_empty() {
                                            state.completed_generation = None;
                                            spawn_speculative_turn(
                                                state,
                                                engine.clone(),
                                                transcript,
                                                internal_tx.clone(),
                                            );
                                        } else {
                                            info!(
                                                turn_id = %state.turn_id,
                                                since_turn_start_ms = state.started_at.elapsed().as_millis(),
                                                "turn_done"
                                            );
                                            emit_turn_stream_event(
                                                &mut ws,
                                                &TurnStreamServerEvent::Done {
                                                    turn_id: state.turn_id.clone(),
                                                },
                                            )
                                            .await?;
                                            turn = None;
                                        }
                                    }
                                }
                                TurnStreamClientMessage::TurnCancel => {
                                    if let Some(mut state) = turn.take() {
                                        if let Some(task) = state.active_task.take() {
                                            task.abort();
                                        }
                                        record_backend_turn_cancellation("client_cancel");
                                        emit_turn_stream_event(
                                            &mut ws,
                                            &TurnStreamServerEvent::Done {
                                                turn_id: state.turn_id,
                                            },
                                        )
                                        .await?;
                                    } else {
                                        emit_turn_stream_event(
                                            &mut ws,
                                            &TurnStreamServerEvent::Error {
                                                turn_id: None,
                                                message: "turn_cancel before turn_start".to_string(),
                                            },
                                        )
                                        .await?;
                                    }
                                }
                                TurnStreamClientMessage::FrontendSkillResult {
                                    turn_id,
                                    intent_id,
                                    result,
                                } => {
                                    let since_turn_start_ms = turn
                                        .as_ref()
                                        .filter(|state| state.turn_id == turn_id)
                                        .map(|state| state.started_at.elapsed().as_millis());
                                    if let Some(ms) = since_turn_start_ms {
                                        info!(
                                            turn_id = %turn_id,
                                            since_turn_start_ms = ms,
                                            "frontend_skill_interpret_request"
                                        );
                                    } else {
                                        info!(turn_id = %turn_id, "frontend_skill_interpret_request");
                                    }
                                    let skill_started = Instant::now();
                                    let finalize_result = engine
                                        .finalize_frontend_skill(&turn_id, &intent_id, result)
                                        .await;
                                    let finalize_elapsed = skill_started.elapsed();
                                    record_backend_turn_duration("frontend_skill_finalize", finalize_elapsed);
                                    match finalize_result {
                                        Ok(text) => {
                                            if let Some(ms) = since_turn_start_ms {
                                                info!(
                                                    turn_id = %turn_id,
                                                    since_turn_start_ms = ms,
                                                    llm_reasoning_elapsed_ms = finalize_elapsed.as_millis(),
                                                    "frontend_skill_interpret_response"
                                                );
                                            } else {
                                                info!(
                                                    turn_id = %turn_id,
                                                    llm_reasoning_elapsed_ms = finalize_elapsed.as_millis(),
                                                    "frontend_skill_interpret_response"
                                                );
                                            }
                                            record_backend_turn_total("frontend_skill_finalize", "success");
                                            emit_turn_stream_event(
                                                &mut ws,
                                                &TurnStreamServerEvent::Token {
                                                    turn_id: turn_id.clone(),
                                                    text,
                                                },
                                            )
                                            .await?;
                                        }
                                        Err(error) => {
                                            if let Some(ms) = since_turn_start_ms {
                                                info!(
                                                    turn_id = %turn_id,
                                                    since_turn_start_ms = ms,
                                                    llm_reasoning_elapsed_ms = finalize_elapsed.as_millis(),
                                                    "frontend_skill_interpret_response_error"
                                                );
                                            } else {
                                                info!(
                                                    turn_id = %turn_id,
                                                    llm_reasoning_elapsed_ms = finalize_elapsed.as_millis(),
                                                    "frontend_skill_interpret_response_error"
                                                );
                                            }
                                            record_backend_turn_total("frontend_skill_finalize", "error");
                                            emit_turn_stream_event(
                                                &mut ws,
                                                &TurnStreamServerEvent::Error {
                                                    turn_id: Some(turn_id.clone()),
                                                    message: format!("finalize error: {error}"),
                                                },
                                            )
                                            .await?;
                                        }
                                    }
                                    if let Some(ms) = since_turn_start_ms {
                                        info!(
                                            turn_id = %turn_id,
                                            since_turn_start_ms = ms,
                                            "turn_done"
                                        );
                                    } else {
                                        info!(turn_id = %turn_id, "turn_done");
                                    }
                                    emit_turn_stream_event(
                                        &mut ws,
                                        &TurnStreamServerEvent::Done { turn_id },
                                    )
                                    .await?;
                                    turn = None;
                                }
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = ws.send(Message::Pong(payload)).await;
                        }
                        Message::Close(_) => break,
                        _ => continue,
                    }
                }
                maybe_internal = internal_rx.recv() => {
                    let Some(internal) = maybe_internal else {
                        continue;
                    };
                    let Some(state) = turn.as_mut() else {
                        continue;
                    };
                    match internal {
                        TurnStreamInternalEvent::SpeculativeFinished { turn_id, generation, result, duration } => {
                            if state.turn_id != turn_id || generation != state.active_generation {
                                continue;
                            }
                            record_backend_turn_stage_duration("speculative_generate", duration);
                            let since_turn_start_ms = state.started_at.elapsed().as_millis();
                            state.active_task = None;
                            state.completed_generation = Some(generation);
                            match result {
                                Ok(decision) => {
                                    info!(
                                        turn_id = %state.turn_id,
                                        generation,
                                        decision = ?decision,
                                        classify_elapsed_ms = duration.as_millis(),
                                        since_turn_start_ms,
                                        "llm_classified_response"
                                    );
                                    let frontend_caps: Option<Vec<String>> = if state.supported_frontend_intents.is_empty() {
                                        None
                                    } else {
                                        Some(state.supported_frontend_intents.clone())
                                    };
                                    if let BackendEngineDecision::FrontendSkillIntent(ref intent) = decision {
                                        if !frontend_intent_allowed(&intent.intent, frontend_caps.as_deref()) {
                                            record_backend_turn_total("frontend_skill_capability_gate", "fallback");
                                            emit_turn_stream_event(
                                                &mut ws,
                                                &TurnStreamServerEvent::Token {
                                                    turn_id: state.turn_id.clone(),
                                                    text: "That action is not available on this active frontend.".to_string(),
                                                },
                                            )
                                            .await?;
                                            if !state.first_token_recorded {
                                                state.first_token_recorded = true;
                                                record_backend_turn_first_token_duration(state.started_at.elapsed());
                                            }
                                        } else {
                                            emit_decision_events(&mut ws, state.turn_id.as_str(), decision).await?;
                                            state.awaiting_skill_result = true;
                                        }
                                    } else {
                                        let emitted_token = emit_decision_events(&mut ws, state.turn_id.as_str(), decision).await?;
                                        if emitted_token && !state.first_token_recorded {
                                            state.first_token_recorded = true;
                                            record_backend_turn_first_token_duration(state.started_at.elapsed());
                                        }
                                    }
                                }
                                Err(error) => {
                                    info!(
                                        turn_id = %state.turn_id,
                                        generation,
                                        classify_elapsed_ms = duration.as_millis(),
                                        since_turn_start_ms,
                                        error = %error,
                                        "llm_classified_response_error"
                                    );
                                    emit_turn_stream_event(
                                        &mut ws,
                                        &TurnStreamServerEvent::Error {
                                            turn_id: Some(state.turn_id.clone()),
                                            message: format!("backend error: {error}"),
                                        },
                                    )
                                    .await?;
                                }
                            }
                            if state.done_requested && !state.awaiting_skill_result {
                                info!(
                                    turn_id = %state.turn_id,
                                    since_turn_start_ms = state.started_at.elapsed().as_millis(),
                                    "turn_done"
                                );
                                emit_turn_stream_event(
                                    &mut ws,
                                    &TurnStreamServerEvent::Done {
                                        turn_id: state.turn_id.clone(),
                                    },
                                )
                                .await?;
                                turn = None;
                            }
                        }
                    }
                }
            }
        }
        Ok::<(), DynError>(())
    }
    .await;

    if let (Some(sid), Some(did)) = (ws_session_id, ws_device_id) {
        remove_ws_session(&frontend_sessions, &did, &sid).await;
    }

    result
}

async fn handle_request(
    req: Request<Incoming>,
    engine: Arc<dyn BackendEngine>,
    frontend_sessions: FrontendSessions,
    transcriber: Arc<dyn AudioTranscriber>,
    audio_config: Arc<AudioIngressConfig>,
) -> Result<Response<RespBody>, Infallible> {
    let request_started_at = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if method == Method::GET && path == "/healthz" {
        return Ok(with_backend_http_metrics(
            &method,
            "/healthz",
            request_started_at,
            plain_response(StatusCode::OK, "ok"),
        ));
    }

    if method == Method::GET && path == "/metrics" {
        return Ok(with_backend_http_metrics(
            &method,
            "/metrics",
            request_started_at,
            plain_response(
                StatusCode::OK,
                "metrics served on configured exporter endpoint",
            ),
        ));
    }

    if method == Method::GET && path == "/turns/stream" {
        if !websocket_upgrade_requested(&req) {
            return Ok(with_backend_http_metrics(
                &method,
                "/turns/stream",
                request_started_at,
                json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error":"expected websocket upgrade request"}),
                ),
            ));
        }
        let ws_key = req
            .headers()
            .get("Sec-WebSocket-Key")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let Some(ws_key) = ws_key else {
            return Ok(with_backend_http_metrics(
                &method,
                "/turns/stream",
                request_started_at,
                json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error":"missing Sec-WebSocket-Key"}),
                ),
            ));
        };
        let accept_key = derive_accept_key(ws_key.as_bytes());
        let on_upgrade = hyper::upgrade::on(req);
        let ws_engine = engine.clone();
        let ws_transcriber = transcriber.clone();
        let ws_sessions = frontend_sessions.clone();
        let ws_audio_config = audio_config.clone();
        tokio::spawn(async move {
            let upgraded = match on_upgrade.await {
                Ok(value) => value,
                Err(error) => {
                    warn!(%error, "websocket upgrade failed");
                    return;
                }
            };
            let io = TokioIo::new(upgraded);
            let ws = WebSocketStream::from_raw_socket(
                io,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            if let Err(error) = handle_turn_stream_socket(
                ws,
                ws_engine,
                ws_transcriber,
                ws_sessions,
                ws_audio_config,
            )
            .await
            {
                warn!(%error, "turn stream websocket session failed");
            }
        });
        return Ok(with_backend_http_metrics(
            &method,
            "/turns/stream",
            request_started_at,
            websocket_switching_response(accept_key),
        ));
    }

    Ok(with_backend_http_metrics(
        &method,
        "not_found",
        request_started_at,
        plain_response(StatusCode::NOT_FOUND, "not found"),
    ))
}

pub struct AiceBackendEngine {
    llm: Arc<CradleLlmStream>,
    weather_skill: Arc<OpenMeteoWeatherSkill>,
    time_skill: OpenMeteoTimeSkill,
    distance_skill: OpenMeteoDistanceSkill,
    sports_live_skill: HttpSportsLiveSkill,
    holiday_lookup_skill: HttpHolidayLookupSkill,
    fuel_price_lookup_skill: HttpFuelPriceLookupSkill,
    horoscope_daily_skill: HttpHoroscopeDailySkill,
    news_headlines_skill: Arc<HttpNewsHeadlinesSkill>,
    calculator_skill: LocalCalculatorSkill,
    unit_conversion_skill: LocalUnitConversionSkill,
    currency_skill: HttpCurrencySkill,
    air_quality_skill: HttpAirQualitySkill,
    air_quality_default_location: Option<AirQualityLocation>,
    dictionary_skill: HttpDictionarySkill,
    translate_skill: LlmTranslateSkill,
    meeting_notes_skill: LlmMeetingNotesSkill,
    journal_skill: Option<Arc<LocalJournalSkill>>,
    screen_ocr_llm: Arc<llm_adapters::ScreenOcrLlmAdapter>,
    news_summary_llm: Arc<llm_adapters::NewsSummaryLlmAdapter>,
    news_summary_streaming_enabled: bool,
    palace: PalaceHandle,
    turn_counter: Arc<std::sync::atomic::AtomicU64>,
    resolved_location: Option<ResolvedLocation>,
    skip_secondary_llm_for_skill_answers: bool,
    classifier_prompt_cache: ClassifierPromptCache,
    classifier_num_ctx: Option<u32>,
    classifier_llm: Arc<CradleLlmStream>,
}

/// LLM-backed intent classifier reused by backend and compatibility wrappers.
pub struct LlmIntentClassifier<'a, L> {
    pub llm: &'a L,
    pub system_prompt: String,
}

impl<'a, L> LlmIntentClassifier<'a, L> {
    pub fn new(llm: &'a L) -> Self {
        Self {
            llm,
            system_prompt: intent_classifier_system_prompt_for_skills(ENABLED_SKILL_IDS),
        }
    }
}

#[async_trait]
impl<L> IntentClassifier for LlmIntentClassifier<'_, L>
where
    L: LlmStream + Send + Sync,
{
    async fn classify(
        &self,
        user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>> {
        let prompt = format!(
            "Classify this user request. Reply with only the JSON object.\nUser request: \"{}\"",
            user_text.trim()
        );
        let classification_options = LlmCallOptions::for_classification();
        let mut stream = self
            .llm
            .chat_stream(
                &prompt,
                &[],
                Some(self.system_prompt.as_str()),
                Some(&classification_options),
            )
            .await?;
        let mut raw = String::new();
        while let Some(token) = stream.next().await {
            raw.push_str(&token);
        }
        let decision = parse_intent(raw.trim())?;
        Ok(validate_intent_decision(decision))
    }
}

impl AiceBackendEngine {
    pub async fn from_config(config: &Config) -> Result<Self, DynError> {
        let llm = CradleLlmStream::new(
            config.ollama_url.clone(),
            config.model.clone(),
            config.llm.short_replies,
            config.llm.max_output_tokens,
            config.llm.system_prompt.clone(),
            config.llm.model_keep_alive.clone(),
        );
        if config.llm.preload_model_on_startup {
            let t0 = Instant::now();
            match llm.warm_up().await {
                Ok(()) => {
                    record_model_preload_duration("llm", t0.elapsed());
                    record_model_preload("llm", "success");
                }
                Err(error) => {
                    record_model_preload_duration("llm", t0.elapsed());
                    record_model_preload("llm", "error");
                    tracing::warn!(%error, "llm preload failed; continuing without startup warmup");
                }
            }
        }
        let weather_skill = Arc::new(OpenMeteoWeatherSkill::new());
        let resolved_location = resolve_startup_location(config, weather_skill.as_ref()).await;

        let palace = if config.memory.enabled {
            let db_path = config.memory.palace_db_path.clone();
            let identity_path = config.memory.palace_identity_path.clone();
            let open_start = Instant::now();
            match Palace::open_paths(Path::new(&db_path), Path::new(&identity_path)) {
                Ok(p) => {
                    record_palace_open("success", open_start.elapsed());
                    info!(
                        palace_db = %db_path,
                        palace_identity = %identity_path,
                        "memory palace opened"
                    );
                    Arc::new(std::sync::Mutex::new(p))
                }
                Err(error) => {
                    record_palace_open("error", open_start.elapsed());
                    record_palace_error("open");
                    warn!(%error, "failed to open memory palace; falling back to in-memory");
                    match Palace::open_in_memory() {
                        Ok(p) => Arc::new(std::sync::Mutex::new(p)),
                        Err(fatal) => {
                            error!(%fatal, "in-memory palace also failed");
                            return Err(format!("palace init failed: {fatal}").into());
                        }
                    }
                }
            }
        } else {
            info!("memory palace disabled; using in-memory instance");
            match Palace::open_in_memory() {
                Ok(p) => Arc::new(std::sync::Mutex::new(p)),
                Err(fatal) => {
                    error!(%fatal, "in-memory palace failed");
                    return Err(format!("palace init failed: {fatal}").into());
                }
            }
        };

        let llm_arc = Arc::new(llm);
        let classifier_llm = if let Some(ref url) = config.llm.classifier_ollama_url {
            let cls_llm = CradleLlmStream::new(
                url.clone(),
                config.model.clone(),
                false,
                CLASSIFIER_MAX_OUTPUT_TOKENS,
                None,
                config.llm.model_keep_alive.clone(),
            );
            if config.llm.preload_model_on_startup {
                let t0 = Instant::now();
                match cls_llm.warm_up().await {
                    Ok(()) => {
                        record_model_preload_duration("classifier_llm", t0.elapsed());
                        record_model_preload("classifier_llm", "success");
                    }
                    Err(error) => {
                        record_model_preload_duration("classifier_llm", t0.elapsed());
                        record_model_preload("classifier_llm", "error");
                        tracing::warn!(%error, "classifier llm preload failed");
                    }
                }
            }
            info!(classifier_url = %url, "using dedicated classifier Ollama instance");
            Arc::new(cls_llm)
        } else {
            Arc::clone(&llm_arc)
        };

        let translation_llm: Arc<dyn TranslationLlm> = Arc::new(
            llm_adapters::TranslationLlmAdapter::new(Arc::clone(&llm_arc)),
        );
        let translate_skill = LlmTranslateSkill::new(translation_llm);

        let meeting_llm: Arc<dyn MeetingNotesLlm> = Arc::new(
            llm_adapters::MeetingNotesLlmAdapter::new(Arc::clone(&llm_arc)),
        );
        let meeting_notes_skill = LlmMeetingNotesSkill::new(meeting_llm);

        let air_quality_default_location =
            resolved_location.as_ref().map(|loc| AirQualityLocation {
                display_name: loc.display_name.clone(),
                lat: loc.lat,
                lon: loc.lon,
            });

        let journal_skill = if config.journal.enabled {
            match SqliteJournalStore::open(&config.journal.sqlite_path) {
                Ok(store) => {
                    info!(
                        journal_db = %config.journal.sqlite_path,
                        "journal store opened"
                    );
                    Some(Arc::new(LocalJournalSkill::new(Arc::new(store))))
                }
                Err(error) => {
                    warn!(%error, journal_db = %config.journal.sqlite_path, "failed to open journal store; journal skill disabled");
                    None
                }
            }
        } else {
            None
        };

        let news_headlines_skill = Arc::new(HttpNewsHeadlinesSkill::new());
        let screen_ocr_llm = Arc::new(llm_adapters::ScreenOcrLlmAdapter::new(Arc::clone(&llm_arc)));
        let news_summary_llm = Arc::new(llm_adapters::NewsSummaryLlmAdapter::new(Arc::clone(
            &llm_arc,
        )));

        Ok(Self {
            llm: llm_arc,
            weather_skill,
            time_skill: OpenMeteoTimeSkill::new(),
            distance_skill: OpenMeteoDistanceSkill::new(),
            sports_live_skill: HttpSportsLiveSkill::new(),
            holiday_lookup_skill: HttpHolidayLookupSkill::new(),
            fuel_price_lookup_skill: HttpFuelPriceLookupSkill::new(),
            horoscope_daily_skill: HttpHoroscopeDailySkill::new(),
            news_headlines_skill,
            calculator_skill: LocalCalculatorSkill::new(),
            unit_conversion_skill: LocalUnitConversionSkill::new(),
            currency_skill: HttpCurrencySkill::new(),
            air_quality_skill: HttpAirQualitySkill::new(),
            air_quality_default_location,
            dictionary_skill: HttpDictionarySkill::new(),
            translate_skill,
            meeting_notes_skill,
            journal_skill,
            screen_ocr_llm,
            news_summary_llm,
            news_summary_streaming_enabled: config.news.enable_summary_streaming,
            palace,
            turn_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            resolved_location,
            skip_secondary_llm_for_skill_answers: config.llm.skip_secondary_llm_for_skill_answers,
            classifier_prompt_cache: Arc::new(RwLock::new(HashMap::new())),
            classifier_num_ctx: config.llm.classifier_num_ctx,
            classifier_llm,
        })
    }

    async fn collect_llm(
        &self,
        operation: &str,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<String, DynError> {
        let history_len = history.len();
        debug!(
            operation,
            llm_input = %user_text.trim(),
            history_len,
            has_system_prompt_override = system_prompt_override.is_some(),
            "llm_input"
        );
        let mut stream = self
            .llm
            .chat_stream(user_text, history, system_prompt_override, call_options)
            .await?;
        let llm_started = Instant::now();
        let mut output = String::new();
        while let Some(token) = stream.next().await {
            output.push_str(&token);
        }
        record_backend_llm_provider_duration("cradle", llm_started.elapsed());
        debug!(
            operation,
            llm_output = %output.trim(),
            "llm_output"
        );
        Ok(output)
    }

    async fn classify_intent(
        &self,
        user_text: &str,
        available_skills: &[&str],
    ) -> Result<IntentDecision, DynError> {
        let prompt = build_intent_classification_prompt(user_text);
        let prompt_build_started = Instant::now();
        let key = classifier_cache_key(available_skills);
        let artifacts = if let Ok(cache) = self.classifier_prompt_cache.read() {
            cache.get(&key).cloned()
        } else {
            None
        }
        .unwrap_or_else(|| {
            let built = build_classifier_prompt_artifacts(available_skills);
            if let Ok(mut cache) = self.classifier_prompt_cache.write() {
                cache.entry(key).or_insert_with(|| built.clone());
            }
            built
        });
        let few_shot_history = artifacts.compact_few_shots.clone();
        record_backend_turn_stage_duration(
            "classifier_prompt_build",
            prompt_build_started.elapsed(),
        );

        let mut options = LlmCallOptions::for_classification();
        options.max_output_tokens = Some(CLASSIFIER_MAX_OUTPUT_TOKENS.max(1));
        options.format_json_schema = artifacts.json_schema.clone();
        options.num_ctx = self.classifier_num_ctx;

        debug!(
            operation = "intent_classification",
            llm_input = %prompt.trim(),
            history_len = few_shot_history.len(),
            "classifier_llm_input"
        );
        let llm_started = Instant::now();
        let raw = self
            .classifier_llm
            .chat_once(
                &prompt,
                few_shot_history.as_slice(),
                Some(artifacts.system_prompt.as_str()),
                Some(&options),
            )
            .await
            .map_err(|e| -> DynError { e });
        record_backend_llm_provider_duration("classifier", llm_started.elapsed());
        if let Ok(ref output) = raw {
            debug!(
                operation = "intent_classification",
                llm_output = %output.trim(),
                "classifier_llm_output"
            );
        }
        record_backend_turn_stage_duration("classifier_llm_roundtrip", llm_started.elapsed());

        let parse_started = Instant::now();
        let decision = raw.as_ref().ok().and_then(|r| parse_validated_intent(r));
        record_backend_turn_stage_duration("intent_parse_validate", parse_started.elapsed());

        if let Some(d) = decision {
            return Ok(d);
        }

        raw?;
        Ok(IntentDecision::Chat)
    }

    async fn compose_skill_answer(
        &self,
        user_text: &str,
        context: &str,
    ) -> Result<String, DynError> {
        let prompt = format!(
            "User: \"{}\"\\nData: {}.\\nReply in at most 2 short voice-friendly sentences.",
            user_text.trim(),
            context
        );
        self.collect_llm("skill_answer_composer", &prompt, &[], None, None)
            .await
    }
}

fn next_backend_turn_id(counter: &std::sync::atomic::AtomicU64) -> String {
    format!(
        "turn-{}",
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn format_degrees_c(temp_c: f64) -> String {
    let rounded = temp_c.round();
    if (temp_c - rounded).abs() < 0.05 {
        format!("{}°C", rounded as i64)
    } else {
        format!("{temp_c:.1}°C")
    }
}

fn compose_time_answer(result: &TimeResult) -> String {
    format!(
        "In {}, it is {} ({}).",
        result.location_display, result.local_time, result.timezone
    )
}

fn compose_weather_answer(result: &WeatherResult) -> String {
    let humidity = result
        .humidity_pct
        .map(|h| format!("{h}% humidity"))
        .unwrap_or_else(|| "humidity unavailable".to_string());
    format!(
        "In {}, it is {}, {}, with {}.",
        result.location_display,
        format_degrees_c(result.temp_c),
        humidity,
        result.description.to_lowercase()
    )
}

fn compose_distance_answer(result: &DistanceResult) -> String {
    format!(
        "The straight-line distance from {} to {} is {} km.",
        result.origin_display,
        result.destination_display,
        result.distance_km.round() as i64
    )
}

fn compose_sports_live_answer(result: &SportsLiveResult) -> String {
    result.to_prompt_context()
}

fn compose_holiday_lookup_answer(result: &HolidayLookupResult) -> String {
    result.to_prompt_context()
}

fn compose_fuel_price_lookup_answer(result: &FuelPriceLookupResult) -> String {
    result.to_prompt_context()
}

fn compose_horoscope_daily_answer(result: &HoroscopeDailyResult) -> String {
    result.to_prompt_context()
}

fn compose_news_headlines_answer(result: &NewsHeadlinesResult) -> String {
    result.to_prompt_context()
}

fn compose_news_summary_answer(items: &[SummarizedHeadline]) -> String {
    if items.is_empty() {
        return "No headlines found.".to_string();
    }
    let lines: Vec<String> = items
        .iter()
        .map(|item| match item.summary.as_deref() {
            Some(text) if !text.trim().is_empty() => {
                format!("{}: {}", item.headline.title, text.trim())
            }
            _ => item.headline.title.clone(),
        })
        .collect();
    format!("Top headlines:\n- {}", lines.join("\n- "))
}

fn compose_calculator_answer(result: &CalculatorResult) -> String {
    result.to_prompt_context()
}

fn compose_unit_conversion_answer(result: &ConversionResult) -> String {
    result.to_prompt_context()
}

fn compose_currency_answer(result: &CurrencyResult) -> String {
    result.to_prompt_context()
}

fn compose_air_quality_answer(result: &AirQualityResult) -> String {
    result.to_prompt_context()
}

fn compose_dictionary_answer(result: &DictionaryResult) -> String {
    result.to_prompt_context()
}

fn compose_translate_answer(result: &TranslateResult) -> String {
    result.to_prompt_context()
}

fn compose_meeting_notes_answer(result: &MeetingNotesResult) -> String {
    result.to_prompt_context()
}

fn compose_briefing_answer(result: &BriefingResult) -> String {
    result.to_prompt_context()
}

fn compose_journal_answer(result: &JournalResult) -> String {
    result.to_prompt_context()
}

fn parse_naive_date(input: Option<String>) -> Option<NaiveDate> {
    input
        .and_then(|value| NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok())
        .or_else(|| Some(Utc::now().date_naive()))
}

fn infer_country_code(resolved_location: Option<&ResolvedLocation>) -> Option<String> {
    let country = resolved_location?
        .display_name
        .rsplit(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let normalized = country.to_ascii_lowercase();
    let code = match normalized.as_str() {
        "germany" => "DE",
        "united states" | "united states of america" | "usa" => "US",
        "united kingdom" | "uk" => "GB",
        "france" => "FR",
        "spain" => "ES",
        "italy" => "IT",
        "austria" => "AT",
        "netherlands" => "NL",
        "poland" => "PL",
        "switzerland" => "CH",
        "belgium" => "BE",
        _ if country.len() == 2 => country,
        _ => return None,
    };
    Some(code.to_string())
}

fn sports_live_error_kind(error: &SportsLiveError) -> &'static str {
    match error {
        SportsLiveError::InvalidQuery(_) => "invalid_query",
        SportsLiveError::ProviderUnavailable(_) => "provider_unavailable",
        SportsLiveError::UpstreamTimeout => "upstream_timeout",
        SportsLiveError::UpstreamParse(_) => "upstream_parse",
    }
}

fn holiday_lookup_error_kind(error: &HolidayLookupError) -> &'static str {
    match error {
        HolidayLookupError::InvalidCountry(_) => "invalid_country",
        HolidayLookupError::InvalidQuery(_) => "invalid_query",
        HolidayLookupError::ProviderUnavailable(_) => "provider_unavailable",
        HolidayLookupError::UpstreamTimeout => "upstream_timeout",
        HolidayLookupError::UpstreamParse(_) => "upstream_parse",
    }
}

fn fuel_price_lookup_error_kind(error: &FuelPriceLookupError) -> &'static str {
    match error {
        FuelPriceLookupError::InvalidCountry(_) => "invalid_country",
        FuelPriceLookupError::UnsupportedCountry(_) => "unsupported_country",
        FuelPriceLookupError::MissingApiKey => "missing_api_key",
        FuelPriceLookupError::ProviderUnavailable(_) => "provider_unavailable",
        FuelPriceLookupError::UpstreamTimeout => "upstream_timeout",
        FuelPriceLookupError::UpstreamParse(_) => "upstream_parse",
    }
}

fn horoscope_daily_error_kind(error: &HoroscopeDailyError) -> &'static str {
    match error {
        HoroscopeDailyError::InvalidSign(_) => "invalid_sign",
        HoroscopeDailyError::UnsupportedDate(_) => "unsupported_date",
        HoroscopeDailyError::ProviderUnavailable(_) => "provider_unavailable",
        HoroscopeDailyError::UpstreamTimeout => "upstream_timeout",
        HoroscopeDailyError::UpstreamParse(_) => "upstream_parse",
    }
}

fn news_headlines_error_kind(error: &NewsHeadlinesError) -> &'static str {
    match error {
        NewsHeadlinesError::InvalidQuery(_) => "invalid_query",
        NewsHeadlinesError::ProviderUnavailable(_) => "provider_unavailable",
        NewsHeadlinesError::UpstreamTimeout => "upstream_timeout",
        NewsHeadlinesError::UpstreamParse(_) => "upstream_parse",
    }
}

fn calculator_error_kind(error: &CalculatorSkillError) -> &'static str {
    match error {
        CalculatorSkillError::EmptyExpression => "empty_expression",
        CalculatorSkillError::ParseError(_) => "parse_error",
        CalculatorSkillError::NonFinite => "non_finite",
    }
}

fn unit_conversion_error_kind(error: &UnitConversionError) -> &'static str {
    match error {
        UnitConversionError::UnknownUnit(_) => "unknown_unit",
        UnitConversionError::DimensionMismatch { .. } => "dimension_mismatch",
        UnitConversionError::InvalidValue(_) => "invalid_value",
        UnitConversionError::ParseError(_) => "parse_error",
    }
}

fn currency_error_kind(error: &CurrencyError) -> &'static str {
    match error {
        CurrencyError::InvalidQuery(_) => "invalid_query",
        CurrencyError::UnsupportedCurrency(_) => "unsupported_currency",
        CurrencyError::ProviderUnavailable(_) => "provider_unavailable",
        CurrencyError::UpstreamTimeout => "upstream_timeout",
        CurrencyError::UpstreamParse(_) => "upstream_parse",
    }
}

fn air_quality_error_kind(error: &AirQualityError) -> &'static str {
    match error {
        AirQualityError::InvalidQuery(_) => "invalid_query",
        AirQualityError::Geocoding(_) => "geocoding",
        AirQualityError::ProviderUnavailable(_) => "provider_unavailable",
        AirQualityError::UpstreamTimeout => "upstream_timeout",
        AirQualityError::UpstreamParse(_) => "upstream_parse",
        AirQualityError::NoDefaultLocation => "no_default_location",
    }
}

fn dictionary_error_kind(error: &DictionaryError) -> &'static str {
    match error {
        DictionaryError::InvalidQuery(_) => "invalid_query",
        DictionaryError::NotFound(_) => "not_found",
        DictionaryError::ProviderUnavailable(_) => "provider_unavailable",
        DictionaryError::UpstreamTimeout => "upstream_timeout",
        DictionaryError::UpstreamParse(_) => "upstream_parse",
    }
}

fn translate_error_kind(error: &TranslateError) -> &'static str {
    match error {
        TranslateError::InvalidQuery(_) => "invalid_query",
        TranslateError::LlmUnavailable(_) => "llm_unavailable",
        TranslateError::EmptyTranslation => "empty_translation",
    }
}

fn meeting_notes_error_kind(error: &MeetingNotesError) -> &'static str {
    match error {
        MeetingNotesError::InvalidQuery(_) => "invalid_query",
        MeetingNotesError::LlmUnavailable(_) => "llm_unavailable",
        MeetingNotesError::InvalidLlmOutput(_) => "invalid_llm_output",
        MeetingNotesError::Reminders(_) => "reminders",
    }
}

fn briefing_error_kind(error: &BriefingError) -> &'static str {
    match error {
        BriefingError::NoSectionsEnabled => "no_sections_enabled",
    }
}

fn journal_error_kind(error: &JournalError) -> &'static str {
    match error {
        JournalError::InvalidQuery(_) => "invalid_query",
        JournalError::Storage(_) => "storage",
        JournalError::NotFound(_) => "not_found",
    }
}

fn compose_direct_skill_answer(context: &str) -> Option<String> {
    let trimmed = context.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[async_trait]
impl BackendEngine for AiceBackendEngine {
    async fn process_turn(&self, request: TurnRequest) -> Result<BackendEngineDecision, DynError> {
        if request.transcript.trim().is_empty() {
            return Ok(BackendEngineDecision::Chat(
                "I did not catch that, please repeat.".to_string(),
            ));
        }

        let request_turn_id = request
            .turn_id
            .clone()
            .unwrap_or_else(|| next_backend_turn_id(&self.turn_counter));
        let request_text = request.transcript.clone();
        let available_skills = build_available_classifier_skills(request.context.as_ref());
        let available_skill_refs: Vec<&str> = available_skills.iter().map(String::as_str).collect();

        let classify_started = Instant::now();
        let decision = match self
            .classify_intent(&request.transcript, available_skill_refs.as_slice())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                warn!(%error, "intent classification failed in backend; falling back to chat");
                IntentDecision::Chat
            }
        };
        let classify_elapsed = classify_started.elapsed();
        record_backend_turn_stage_duration("classify_intent", classify_elapsed);

        let build_frontend_intent = |intent: &str, slots: serde_json::Value| {
            BackendEngineDecision::FrontendSkillIntent(FrontendSkillIntent {
                turn_id: request_turn_id.clone(),
                intent: intent.to_string(),
                slots,
                user_text: request_text.clone(),
            })
        };
        match decision {
            IntentDecision::SkillWeather { location } => {
                let skill_started = Instant::now();
                let result = self
                    .weather_skill
                    .execute(location.as_deref(), self.resolved_location.as_ref())
                    .await
                    .map_err(|error| format!("weather skill failed: {error}"))?;
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                let answer = compose_weather_answer(&result);
                Ok(BackendEngineDecision::BackendSkill(answer))
            }
            IntentDecision::SkillTime { location } => {
                let skill_started = Instant::now();
                let result = self
                    .time_skill
                    .execute(location.as_deref(), self.resolved_location.as_ref())
                    .await
                    .map_err(|error| format!("time skill failed: {error}"))?;
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                let answer = compose_time_answer(&result);
                Ok(BackendEngineDecision::BackendSkill(answer))
            }
            IntentDecision::SkillDistance {
                origin,
                destination,
            } => {
                let skill_started = Instant::now();
                let result = self
                    .distance_skill
                    .execute(
                        origin.as_deref(),
                        destination.as_deref(),
                        self.resolved_location.as_ref(),
                    )
                    .await
                    .map_err(|error| format!("distance skill failed: {error}"))?;
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                let answer = compose_distance_answer(&result);
                Ok(BackendEngineDecision::BackendSkill(answer))
            }
            IntentDecision::SkillSportsLive { query, date } => {
                let skill_started = Instant::now();
                let skill_query = SportsLiveQuery {
                    query: query.unwrap_or_else(|| request.transcript.clone()),
                    date: parse_naive_date(date),
                };
                let result = match self.sports_live_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_backend_skill_execute("skill_sports_live", "success", None);
                        value
                    }
                    Err(error) => {
                        record_backend_skill_execute(
                            "skill_sports_live",
                            "error",
                            Some(sports_live_error_kind(&error)),
                        );
                        return Err(format!("sports-live skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_sports_live", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_sports_live_answer(&result),
                ))
            }
            IntentDecision::SkillHolidayLookup {
                name,
                date,
                country_code,
                region_code,
                year,
            } => {
                let Some(resolved_country) =
                    country_code.or_else(|| infer_country_code(self.resolved_location.as_ref()))
                else {
                    return Ok(BackendEngineDecision::Chat(
                        "Please tell me the country for the holiday lookup.".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let parsed_date = parse_naive_date(date);
                let holiday_year = year.or_else(|| parsed_date.map(|value| value.year()));
                let skill_query = HolidayQuery {
                    holiday_name: name,
                    date: parsed_date,
                    country_code: resolved_country,
                    region_code,
                    year: holiday_year,
                };
                let result = match self.holiday_lookup_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_backend_skill_execute("skill_holiday_lookup", "success", None);
                        value
                    }
                    Err(error) => {
                        record_backend_skill_execute(
                            "skill_holiday_lookup",
                            "error",
                            Some(holiday_lookup_error_kind(&error)),
                        );
                        return Err(format!("holiday-lookup skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_holiday_lookup",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_holiday_lookup_answer(&result),
                ))
            }
            IntentDecision::SkillFuelPriceLookup {
                country_code,
                region,
                fuel_type,
            } => {
                let Some(resolved_country) =
                    country_code.or_else(|| infer_country_code(self.resolved_location.as_ref()))
                else {
                    return Ok(BackendEngineDecision::Chat(
                        "Please tell me the country for the fuel price lookup.".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let skill_query = FuelPriceLookupQuery {
                    country_code: resolved_country,
                    region,
                    fuel_type,
                };
                let result = match self.fuel_price_lookup_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_backend_skill_execute("skill_fuel_price_lookup", "success", None);
                        value
                    }
                    Err(error) => {
                        record_backend_skill_execute(
                            "skill_fuel_price_lookup",
                            "error",
                            Some(fuel_price_lookup_error_kind(&error)),
                        );
                        return Err(format!("fuel-price-lookup skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_fuel_price_lookup",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_fuel_price_lookup_answer(&result),
                ))
            }
            IntentDecision::SkillHoroscopeDaily { sign, date } => {
                let Some(sign) = sign else {
                    return Ok(BackendEngineDecision::Chat(
                        "Please tell me your zodiac sign for the horoscope.".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let skill_query = HoroscopeDailyQuery {
                    sign,
                    date: parse_naive_date(date),
                };
                let result = match self.horoscope_daily_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_backend_skill_execute("skill_horoscope_daily", "success", None);
                        value
                    }
                    Err(error) => {
                        record_backend_skill_execute(
                            "skill_horoscope_daily",
                            "error",
                            Some(horoscope_daily_error_kind(&error)),
                        );
                        return Err(format!("horoscope-daily skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_horoscope_daily",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_horoscope_daily_answer(&result),
                ))
            }
            IntentDecision::SkillNewsHeadlines {
                topic,
                country_code,
                limit,
            } => {
                let skill_started = Instant::now();
                let skill_query = NewsHeadlinesQuery {
                    topic: topic.unwrap_or_else(|| "top headlines".to_string()),
                    country_code: country_code
                        .or_else(|| infer_country_code(self.resolved_location.as_ref())),
                    limit,
                };
                let result = match self.news_headlines_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_backend_skill_execute("skill_news_headlines", "success", None);
                        value
                    }
                    Err(error) => {
                        record_backend_skill_execute(
                            "skill_news_headlines",
                            "error",
                            Some(news_headlines_error_kind(&error)),
                        );
                        return Err(format!("news-headlines skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_news_headlines",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                let answer = if self.news_summary_streaming_enabled && !result.headlines.is_empty()
                {
                    let summary_llm: Arc<dyn NewsSummaryLlm> =
                        Arc::clone(&self.news_summary_llm) as Arc<dyn NewsSummaryLlm>;
                    let summarized =
                        collect_news_summaries(result.headlines.clone(), summary_llm).await;
                    compose_news_summary_answer(&summarized)
                } else {
                    compose_news_headlines_answer(&result)
                };
                Ok(BackendEngineDecision::BackendSkill(answer))
            }
            IntentDecision::SkillSmartHome { target, action } => Ok(build_frontend_intent(
                "skill_smart_home",
                json!({"smart_home_target": target, "smart_home_action": action}),
            )),
            IntentDecision::SkillComputer { action, target } => Ok(build_frontend_intent(
                "skill_computer",
                json!({"computer_action": action, "computer_target": target}),
            )),
            IntentDecision::SkillAppSwitcher { action, target } => Ok(build_frontend_intent(
                "skill_app_switcher",
                json!({"app_switcher_action": action, "app_switcher_target": target}),
            )),
            IntentDecision::SkillReminder { title, when } => Ok(build_frontend_intent(
                "skill_reminder",
                json!({"reminder_title": title, "reminder_when": when}),
            )),
            IntentDecision::SkillMessage {
                command,
                contact,
                message,
            } => Ok(build_frontend_intent(
                "skill_message",
                json!({"message_command": command, "message_contact": contact, "message_text": message}),
            )),
            IntentDecision::SkillTimer { duration, name } => Ok(build_frontend_intent(
                "skill_timer",
                json!({"timer_duration": duration, "timer_name": name}),
            )),
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => Ok(build_frontend_intent(
                "skill_shopping_list",
                json!({"shopping_action": action, "shopping_items": items, "shopping_when": when}),
            )),
            IntentDecision::SkillVolume { action, level } => Ok(build_frontend_intent(
                "skill_volume",
                json!({"volume_action": action, "volume_level": level}),
            )),
            IntentDecision::SkillMedia { action, target } => Ok(build_frontend_intent(
                "skill_media",
                json!({"media_action": action, "media_target": target}),
            )),
            IntentDecision::SkillScreenshot { filename } => Ok(build_frontend_intent(
                "skill_screenshot",
                json!({"screenshot_filename": filename}),
            )),
            IntentDecision::SkillCalculator { expression } => {
                let Some(expr) = expression.filter(|s| !s.trim().is_empty()) else {
                    record_calculator_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "What expression should I calculate?".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let result = match self.calculator_skill.execute(&expr).await {
                    Ok(value) => {
                        record_calculator_skill("success");
                        record_backend_skill_execute("skill_calculator", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = calculator_error_kind(&error);
                        record_calculator_skill("error");
                        record_backend_skill_execute("skill_calculator", "error", Some(kind));
                        return Err(format!("calculator skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_calculator", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_calculator_answer(&result),
                ))
            }
            IntentDecision::SkillUnitConversion {
                query,
                value,
                from_unit,
                to_unit,
            } => {
                let skill_started = Instant::now();
                let result = match (value, from_unit.as_deref(), to_unit.as_deref()) {
                    (Some(v), Some(from), Some(to)) => {
                        self.unit_conversion_skill.execute(v, from, to).await
                    }
                    _ => {
                        let Some(q) = query.as_deref().filter(|s| !s.trim().is_empty()) else {
                            record_unit_conversion_skill("error");
                            return Ok(BackendEngineDecision::Chat(
                                "Tell me which units to convert (e.g. \"5 km to miles\")."
                                    .to_string(),
                            ));
                        };
                        self.unit_conversion_skill.execute_query(q).await
                    }
                };
                let result = match result {
                    Ok(value) => {
                        record_unit_conversion_skill("success");
                        record_backend_skill_execute("skill_unit_conversion", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = unit_conversion_error_kind(&error);
                        record_unit_conversion_skill("error");
                        record_backend_skill_execute("skill_unit_conversion", "error", Some(kind));
                        return Err(format!("unit-conversion skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_unit_conversion",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_unit_conversion_answer(&result),
                ))
            }
            IntentDecision::SkillCurrency {
                amount,
                from_currency,
                to_currency,
            } => {
                let Some(from_currency) = from_currency.filter(|s| !s.trim().is_empty()) else {
                    record_currency_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Which currency are we converting from?".to_string(),
                    ));
                };
                let Some(to_currency) = to_currency.filter(|s| !s.trim().is_empty()) else {
                    record_currency_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Which currency are we converting to?".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let skill_query = CurrencyQuery {
                    amount: amount.unwrap_or(1.0),
                    from_currency,
                    to_currency,
                };
                let result = match self.currency_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_currency_skill("success");
                        record_backend_skill_execute("skill_currency", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = currency_error_kind(&error);
                        record_currency_skill("error");
                        record_backend_skill_execute("skill_currency", "error", Some(kind));
                        return Err(format!("currency skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_currency", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_currency_answer(&result),
                ))
            }
            IntentDecision::SkillAirQuality { location } => {
                let skill_started = Instant::now();
                let result = match self
                    .air_quality_skill
                    .execute(
                        location.as_deref(),
                        self.air_quality_default_location.as_ref(),
                    )
                    .await
                {
                    Ok(value) => {
                        record_air_quality_skill("success");
                        record_backend_skill_execute("skill_air_quality", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = air_quality_error_kind(&error);
                        record_air_quality_skill("error");
                        record_backend_skill_execute("skill_air_quality", "error", Some(kind));
                        return Err(format!("air-quality skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_air_quality", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_air_quality_answer(&result),
                ))
            }
            IntentDecision::SkillDictionary { word } => {
                let Some(word) = word.filter(|s| !s.trim().is_empty()) else {
                    record_dictionary_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Which word should I look up?".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let result = match self.dictionary_skill.execute(&word).await {
                    Ok(value) => {
                        record_dictionary_skill("success");
                        record_backend_skill_execute("skill_dictionary", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = dictionary_error_kind(&error);
                        record_dictionary_skill("error");
                        record_backend_skill_execute("skill_dictionary", "error", Some(kind));
                        return Err(format!("dictionary skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_dictionary", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_dictionary_answer(&result),
                ))
            }
            IntentDecision::SkillTranslate {
                text,
                source_language,
                target_language,
            } => {
                let Some(text) = text.filter(|s| !s.trim().is_empty()) else {
                    record_translate_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "What text should I translate?".to_string(),
                    ));
                };
                let Some(target_language) = target_language.filter(|s| !s.trim().is_empty()) else {
                    record_translate_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Which language should I translate to?".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let skill_query = TranslateQuery {
                    text,
                    source_language,
                    target_language,
                };
                let result = match self.translate_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_translate_skill("success");
                        record_backend_skill_execute("skill_translate", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = translate_error_kind(&error);
                        record_translate_skill("error");
                        record_backend_skill_execute("skill_translate", "error", Some(kind));
                        return Err(format!("translate skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_translate", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_translate_answer(&result),
                ))
            }
            IntentDecision::SkillCalendar {
                action,
                title,
                when,
                days,
                location,
                calendar_name,
            } => {
                record_calendar_skill("dispatched");
                Ok(build_frontend_intent(
                    "skill_calendar",
                    json!({
                        "calendar_action": action,
                        "calendar_title": title,
                        "calendar_when": when,
                        "calendar_days": days,
                        "calendar_location": location,
                        "calendar_name": calendar_name,
                    }),
                ))
            }
            IntentDecision::SkillMeetingNotes {
                transcript,
                title,
                create_reminders,
            } => {
                let Some(transcript) = transcript.filter(|s| !s.trim().is_empty()) else {
                    record_meeting_notes_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Share the meeting transcript and I'll summarize it.".to_string(),
                    ));
                };
                let skill_started = Instant::now();
                let skill_query = MeetingNotesQuery {
                    transcript,
                    title,
                    create_reminders: create_reminders.unwrap_or(false),
                };
                let result = match self.meeting_notes_skill.execute(&skill_query).await {
                    Ok(value) => {
                        record_meeting_notes_skill("success");
                        record_backend_skill_execute("skill_meeting_notes", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = meeting_notes_error_kind(&error);
                        record_meeting_notes_skill("error");
                        record_backend_skill_execute("skill_meeting_notes", "error", Some(kind));
                        return Err(format!("meeting-notes skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration(
                    "skill_meeting_notes",
                    skill_started.elapsed(),
                );
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_meeting_notes_answer(&result),
                ))
            }
            IntentDecision::SkillEmail {
                action,
                query,
                limit,
                mailbox,
            } => {
                record_email_skill("dispatched");
                Ok(build_frontend_intent(
                    "skill_email",
                    json!({
                        "email_action": action,
                        "email_query": query,
                        "email_limit": limit,
                        "email_mailbox": mailbox,
                    }),
                ))
            }
            IntentDecision::SkillBriefing {
                include,
                news_topic,
                news_country,
            } => {
                let (include_weather, include_news) = match include.as_deref() {
                    Some(parts) if !parts.is_empty() => (
                        parts.iter().any(|p| p.eq_ignore_ascii_case("weather")),
                        parts.iter().any(|p| p.eq_ignore_ascii_case("news")),
                    ),
                    _ => (true, true),
                };
                let briefing_query = BriefingQuery {
                    greeting: None,
                    include_weather,
                    include_calendar: false,
                    include_email: false,
                    include_news,
                    weather_location: None,
                    email_limit: 0,
                    news_topic: news_topic.unwrap_or_else(|| "top".to_string()),
                    news_country: news_country
                        .or_else(|| infer_country_code(self.resolved_location.as_ref())),
                    news_limit: 5,
                };
                let briefing_skill = ComposedBriefingSkill::new()
                    .with_weather(
                        Arc::clone(&self.weather_skill) as Arc<dyn WeatherSkill>,
                        self.resolved_location.clone(),
                    )
                    .with_news(
                        Arc::clone(&self.news_headlines_skill) as Arc<dyn NewsHeadlinesSkill>
                    );
                let skill_started = Instant::now();
                let result = match briefing_skill.execute(&briefing_query).await {
                    Ok(value) => {
                        record_briefing_skill("success");
                        record_backend_skill_execute("skill_briefing", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = briefing_error_kind(&error);
                        record_briefing_skill("error");
                        record_backend_skill_execute("skill_briefing", "error", Some(kind));
                        return Err(format!("briefing skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_briefing", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(
                    compose_briefing_answer(&result),
                ))
            }
            IntentDecision::SkillJournal {
                action,
                text,
                sentiment,
                tags,
                query,
                limit,
            } => {
                let Some(journal_skill) = self.journal_skill.as_ref() else {
                    record_journal_skill("error");
                    return Ok(BackendEngineDecision::Chat(
                        "Journal is not enabled in the backend configuration.".to_string(),
                    ));
                };
                let action_str = action.as_deref().unwrap_or("add");
                let journal_action = match action_str.trim().to_lowercase().as_str() {
                    "add" => {
                        let Some(text) = text.filter(|s| !s.trim().is_empty()) else {
                            record_journal_skill("error");
                            return Ok(BackendEngineDecision::Chat(
                                "What would you like me to journal?".to_string(),
                            ));
                        };
                        JournalAction::Add {
                            text,
                            sentiment: sentiment.as_deref().and_then(Sentiment::parse),
                            tags: tags.unwrap_or_default(),
                        }
                    }
                    "recall" => JournalAction::Recall {
                        from: None,
                        to: None,
                        contains: query.filter(|s| !s.trim().is_empty()),
                        limit: limit.unwrap_or(10),
                    },
                    "stats" => JournalAction::Stats {
                        from: None,
                        to: None,
                    },
                    other => {
                        record_journal_skill("error");
                        return Ok(BackendEngineDecision::Chat(format!(
                            "Unsupported journal action: {other}"
                        )));
                    }
                };
                let skill_started = Instant::now();
                let result = match journal_skill.execute(&journal_action).await {
                    Ok(value) => {
                        record_journal_skill("success");
                        record_backend_skill_execute("skill_journal", "success", None);
                        value
                    }
                    Err(error) => {
                        let kind = journal_error_kind(&error);
                        record_journal_skill("error");
                        record_backend_skill_execute("skill_journal", "error", Some(kind));
                        return Err(format!("journal skill failed: {error}").into());
                    }
                };
                record_backend_skill_execute_duration("skill_journal", skill_started.elapsed());
                record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                Ok(BackendEngineDecision::BackendSkill(compose_journal_answer(
                    &result,
                )))
            }
            IntentDecision::SkillScreenOcr { question, filename } => {
                record_screen_ocr_skill("dispatched");
                Ok(build_frontend_intent(
                    "skill_screen_ocr",
                    json!({
                        "ocr_question": question,
                        "ocr_filename": filename,
                    }),
                ))
            }
            IntentDecision::Chat => {
                let wake_up_started = Instant::now();
                let palace_for_wakeup = self.palace.clone();
                let memory_context =
                    tokio::task::spawn_blocking(move || -> Result<String, DynError> {
                        let mut palace = palace_for_wakeup
                            .lock()
                            .map_err(|e| -> DynError { format!("palace lock: {e}").into() })?;
                        Ok(palace.wake_up(None))
                    })
                    .await
                    .map_err(|e| -> DynError {
                        record_palace_wake_up("error", wake_up_started.elapsed());
                        record_palace_error("wake_up");
                        format!("palace wake_up task: {e}").into()
                    })??;
                record_palace_wake_up("success", wake_up_started.elapsed());
                record_backend_turn_stage_duration("palace_wake_up", wake_up_started.elapsed());

                let system_prompt_override = if memory_context.trim().is_empty() {
                    None
                } else {
                    Some(memory_context)
                };

                let chat_started = Instant::now();
                let text = self
                    .collect_llm(
                        "chat",
                        &request.transcript,
                        &[],
                        system_prompt_override.as_deref(),
                        None,
                    )
                    .await?;
                record_backend_turn_stage_duration("chat_generate", chat_started.elapsed());

                let palace_for_ingest = self.palace.clone();
                let ingest_user = request_text.clone();
                let ingest_assistant = text.clone();
                tokio::task::spawn_blocking(move || {
                    let ingest_started = Instant::now();
                    if let Ok(palace) = palace_for_ingest.lock() {
                        match palace.ingest_turn(&ingest_user, &ingest_assistant) {
                            Ok(()) => {
                                record_palace_ingest("success", ingest_started.elapsed());
                            }
                            Err(error) => {
                                record_palace_ingest("error", ingest_started.elapsed());
                                record_palace_error("ingest");
                                tracing::warn!(%error, "palace ingest_turn failed");
                            }
                        }
                    } else {
                        record_palace_error("ingest_lock");
                    }
                });

                Ok(BackendEngineDecision::Chat(text))
            }
        }
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        intent_id: &str,
        request: FrontendSkillResultRequest,
    ) -> Result<String, DynError> {
        if request.status.eq_ignore_ascii_case("error") {
            if intent_id == "skill_screen_ocr" {
                record_screen_ocr_skill("result_error");
            }
            return Ok(compose_frontend_skill_error_outcome(&request));
        }

        if intent_id == "skill_screen_ocr" {
            return self.finalize_screen_ocr(&request).await;
        }

        // Skill-agnostic: any non-empty structured context from the frontend is composed like backend skills.
        let context_opt = request
            .structured_result_context
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(context) = context_opt {
            if self.skip_secondary_llm_for_skill_answers {
                return Ok(compose_direct_skill_answer(context)
                    .unwrap_or_else(|| "The action completed successfully.".to_string()));
            }
            let compose_started = Instant::now();
            let composed = self
                .compose_skill_answer(&request.user_text, context)
                .await?;
            record_backend_turn_stage_duration(
                "frontend_skill_answer_compose",
                compose_started.elapsed(),
            );
            return Ok(composed);
        }

        Ok(compose_frontend_skill_success_echo(&request))
    }
}

impl AiceBackendEngine {
    /// Finalize a screen-OCR turn. The frontend captures pixels and sends OCR
    /// text via `structured_result_context` (JSON: `{"ocr_text": "...",
    /// "question": "..."?}`). The backend uses its vision-capable LLM adapter
    /// to compose the spoken answer.
    async fn finalize_screen_ocr(
        &self,
        request: &FrontendSkillResultRequest,
    ) -> Result<String, DynError> {
        let Some(raw) = request
            .structured_result_context
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            record_screen_ocr_skill("parse_error");
            return Ok("I did not receive any captured screen content.".to_string());
        };

        let payload: serde_json::Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => {
                record_screen_ocr_skill("parse_error");
                return Ok("I could not parse the screen capture payload.".to_string());
            }
        };

        let ocr_text = payload
            .get("ocr_text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(ocr_text) = ocr_text else {
            record_screen_ocr_skill("parse_error");
            return Ok("The screen capture did not contain readable text.".to_string());
        };

        let question = payload
            .get("question")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if !request.user_text.trim().is_empty() {
                    request.user_text.trim().to_string()
                } else {
                    "Summarize the visible content.".to_string()
                }
            });

        let compose_started = Instant::now();
        let answer = match self.screen_ocr_llm.answer(&question, ocr_text).await {
            Ok(text) => text,
            Err(error) => {
                record_screen_ocr_skill("result_error");
                return Err(format!("screen-ocr llm failed: {error}").into());
            }
        };
        record_backend_skill_execute_duration("skill_screen_ocr", compose_started.elapsed());
        record_backend_turn_stage_duration(
            "frontend_skill_answer_compose",
            compose_started.elapsed(),
        );
        record_screen_ocr_skill("result_ok");
        record_backend_skill_execute("skill_screen_ocr", "success", None);
        Ok(answer)
    }
}

fn compose_frontend_skill_error_outcome(request: &FrontendSkillResultRequest) -> String {
    let fallback = request
        .error
        .clone()
        .unwrap_or_else(|| "The action failed.".to_string());
    format!("I could not complete that action: {fallback}")
}

fn compose_frontend_skill_success_echo(request: &FrontendSkillResultRequest) -> String {
    request
        .structured_result_context
        .clone()
        .unwrap_or_else(|| "The action completed successfully.".to_string())
}

async fn resolve_startup_location(
    _config: &Config,
    weather_skill: &OpenMeteoWeatherSkill,
) -> Option<ResolvedLocation> {
    let _ = weather_skill;
    let resolved = try_ip_geolocation().await;
    if let Some(location) = resolved.as_ref() {
        info!(
            display_name = %location.display_name,
            "backend startup location resolved from IP geolocation"
        );
    } else {
        warn!("backend startup location unavailable");
    }
    resolved
}

async fn try_ip_geolocation() -> Option<ResolvedLocation> {
    #[derive(serde::Deserialize)]
    struct IpApiResponse {
        status: String,
        city: Option<String>,
        country: Option<String>,
        lat: Option<f64>,
        lon: Option<f64>,
    }

    const IP_API_URL: &str = "http://ip-api.com/json/?fields=status,city,country,lat,lon";
    let started_at = Instant::now();
    let response = match reqwest::Client::new().get(IP_API_URL).send().await {
        Ok(value) => {
            record_backend_dependency_request("ip_api", "geolocation", "success", None);
            value
        }
        Err(_) => {
            record_backend_dependency_request("ip_api", "geolocation", "error", Some("request"));
            record_backend_dependency_request_duration(
                "ip_api",
                "geolocation",
                started_at.elapsed(),
            );
            return None;
        }
    };
    record_backend_dependency_request_duration("ip_api", "geolocation", started_at.elapsed());
    if !response.status().is_success() {
        record_backend_dependency_request("ip_api", "geolocation", "error", Some("http_status"));
        return None;
    }
    let payload = match response.json::<IpApiResponse>().await {
        Ok(value) => value,
        Err(_) => {
            record_backend_dependency_request("ip_api", "geolocation", "error", Some("parse"));
            return None;
        }
    };
    if payload.status != "success" {
        record_backend_dependency_request("ip_api", "geolocation", "error", Some("status"));
        return None;
    }
    let lat = payload.lat?;
    let lon = payload.lon?;
    let display_name = match (payload.city, payload.country) {
        (Some(city), Some(country)) => format!("{city}, {country}"),
        (Some(city), None) => city,
        (None, Some(country)) => country,
        (None, None) => format!("{lat:.4}, {lon:.4}"),
    };
    Some(ResolvedLocation {
        display_name,
        lat,
        lon,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_available_classifier_skills, build_classifier_prompt_artifacts,
        build_intent_classification_prompt, classifier_cache_key, compose_distance_answer,
        compose_frontend_skill_error_outcome, compose_frontend_skill_success_echo,
        compose_time_answer, compose_weather_answer, parse_validated_intent,
    };
    use core_orchestrator::intent_classifier_few_shots;
    use core_runtime_protocol::FrontendSkillResultRequest;
    use core_skills::{
        DistanceResult, FuelPriceLookupResult, HolidayLookupResult, HoroscopeDailyResult,
        NewsHeadline, NewsHeadlinesResult, SportsEvent, SportsLiveResult, TimeResult,
        WeatherResult,
    };
    use serde_json::json;
    use std::time::SystemTime;

    #[test]
    fn compose_time_answer_is_deterministic() {
        let result = TimeResult {
            location_display: "Munich, Germany".to_string(),
            local_time: "13:12".to_string(),
            timezone: "GMT+1".to_string(),
        };
        let spoken = compose_time_answer(&result);
        assert_eq!(spoken, "In Munich, Germany, it is 13:12 (GMT+1).");
    }

    #[test]
    fn compose_weather_answer_is_deterministic() {
        let result = WeatherResult {
            location_display: "Munich, Germany".to_string(),
            temp_c: 9.0,
            humidity_pct: Some(55),
            weather_code: 3,
            description: "Partly cloudy".to_string(),
            daily_forecast: Vec::new(),
            alerts: Vec::new(),
        };
        let spoken = compose_weather_answer(&result);
        assert_eq!(
            spoken,
            "In Munich, Germany, it is 9°C, 55% humidity, with partly cloudy."
        );
    }

    #[test]
    fn compose_distance_answer_is_deterministic() {
        let result = DistanceResult {
            origin_display: "Munich, Germany".to_string(),
            destination_display: "Salzburg, Austria".to_string(),
            distance_km: 116.4,
        };
        let spoken = compose_distance_answer(&result);
        assert_eq!(
            spoken,
            "The straight-line distance from Munich, Germany to Salzburg, Austria is 116 km."
        );
    }

    #[test]
    fn classification_few_shots_include_message_send_contract_example() {
        let examples = intent_classifier_few_shots();
        assert!(
            examples.iter().any(|(u, a)| {
                u.contains("ask my wife how she is")
                    && a.contains("\"i\":\"msg\"")
                    && a.contains("\"c\":\"send\"")
                    && a.contains("\"t\":\"my wife\"")
                    && a.contains("\"v\":\"How are you?\"")
            }),
            "expected canonical message rewrite few-shot contract example"
        );
        assert!(
            examples.iter().any(|(u, a)| {
                u.contains("send a message to my wife.")
                    && a.contains("\"i\":\"msg\"")
                    && a.contains("\"c\":\"send\"")
                    && a.contains("\"t\":\"my wife\"")
                    && !a.contains("\"v\"")
            }),
            "expected no-invention message example for missing message text"
        );
    }

    #[test]
    fn frontend_skill_success_echo_returns_context_when_no_compose_path() {
        let request = FrontendSkillResultRequest {
            status: "ok".to_string(),
            user_text: "ask my wife how she is".to_string(),
            structured_result_context: Some(
                "Sent iMessage to Tetiana. Message: \"How are you?\".".to_string(),
            ),
            error: None,
        };

        let spoken = compose_frontend_skill_success_echo(&request);
        assert_eq!(
            spoken,
            "Sent iMessage to Tetiana. Message: \"How are you?\"."
        );
    }

    #[test]
    fn frontend_skill_success_echo_falls_back_when_context_missing() {
        let request = FrontendSkillResultRequest {
            status: "success".to_string(),
            user_text: "do something".to_string(),
            structured_result_context: None,
            error: None,
        };
        assert_eq!(
            compose_frontend_skill_success_echo(&request),
            "The action completed successfully."
        );
    }

    #[test]
    fn frontend_skill_error_returns_deterministic_failure() {
        let request = FrontendSkillResultRequest {
            status: "error".to_string(),
            user_text: "ask my wife how she is".to_string(),
            structured_result_context: None,
            error: Some("contact not found".to_string()),
        };

        let spoken = compose_frontend_skill_error_outcome(&request);
        assert_eq!(
            spoken,
            "I could not complete that action: contact not found"
        );
    }

    #[test]
    fn intent_prompt_uses_real_newline_before_user_request() {
        let prompt = build_intent_classification_prompt("what's the time?");
        assert!(prompt.contains("JSON object.\nUser request:"));
        assert!(!prompt.contains("JSON object.\\nUser request:"));
    }

    #[test]
    fn available_classifier_skills_include_backend_and_frontend_session_intents() {
        let context = json!({
            "frontend_supported_intents": [
                " skill_message ",
                "SKILL_TIMER",
                "skill_message",
                "unknown_intent"
            ]
        });
        let skills = build_available_classifier_skills(Some(&context));
        assert_eq!(
            skills,
            vec![
                "skill_distance".to_string(),
                "skill_fuel_price_lookup".to_string(),
                "skill_holiday_lookup".to_string(),
                "skill_horoscope_daily".to_string(),
                "skill_message".to_string(),
                "skill_news_headlines".to_string(),
                "skill_sports_live".to_string(),
                "skill_time".to_string(),
                "skill_timer".to_string(),
                "skill_weather".to_string(),
            ]
        );
    }

    #[test]
    fn available_classifier_skills_include_enabled_core_common_defaults() {
        let skills = build_available_classifier_skills(None);
        assert_eq!(
            skills,
            vec![
                "skill_distance".to_string(),
                "skill_fuel_price_lookup".to_string(),
                "skill_holiday_lookup".to_string(),
                "skill_horoscope_daily".to_string(),
                "skill_news_headlines".to_string(),
                "skill_sports_live".to_string(),
                "skill_time".to_string(),
                "skill_weather".to_string(),
            ]
        );
    }

    #[test]
    fn available_classifier_skills_do_not_include_macos_only_defaults() {
        let skills = build_available_classifier_skills(None);
        assert!(!skills.contains(&"skill_message".to_string()));
        assert!(!skills.contains(&"skill_timer".to_string()));
        assert!(!skills.contains(&"skill_smart_home".to_string()));
        assert!(!skills.contains(&"skill_memory".to_string()));
    }

    #[test]
    fn classifier_cache_key_is_order_and_case_insensitive() {
        let a = classifier_cache_key(&["skill_timer", "skill_time", "SKILL_TIMER"]);
        let b = classifier_cache_key(&["skill_time", "skill_timer"]);
        assert_eq!(a, b);
        assert_eq!(a, "skill_time|skill_timer");
    }

    #[test]
    fn classifier_prompt_artifacts_scope_compact_few_shots() {
        let artifacts = build_classifier_prompt_artifacts(&["skill_time"]);
        assert!(artifacts.system_prompt.contains("\"time\""));
        assert!(!artifacts.compact_few_shots.is_empty());
        assert!(artifacts
            .compact_few_shots
            .iter()
            .all(|(_, answer)| answer.contains("\"i\":\"time\"")));
    }

    #[test]
    fn classifier_prompt_artifacts_are_byte_identical_across_calls() {
        let skills = &["skill_weather", "skill_time", "skill_media"][..];
        let a = build_classifier_prompt_artifacts(skills);
        let b = build_classifier_prompt_artifacts(skills);
        assert_eq!(a.system_prompt, b.system_prompt);
        assert_eq!(a.compact_few_shots, b.compact_few_shots);
        assert_eq!(a.json_schema, b.json_schema);
    }

    #[test]
    fn parse_validated_intent_returns_none_for_invalid_json() {
        assert!(parse_validated_intent("not-json").is_none());
    }

    #[test]
    fn compose_sports_live_answer_is_deterministic() {
        let result = SportsLiveResult {
            events: vec![SportsEvent {
                league: Some("NBA".to_string()),
                event: "Lakers vs Celtics".to_string(),
                home_team: Some("Lakers".to_string()),
                away_team: Some("Celtics".to_string()),
                start_time: Some("20:00".to_string()),
                status: Some("scheduled".to_string()),
                home_score: None,
                away_score: None,
                scorers: vec![],
            }],
            as_of: SystemTime::UNIX_EPOCH,
        };
        let spoken = super::compose_sports_live_answer(&result);
        assert_eq!(spoken, "Sports events: Lakers vs Celtics.");
    }

    #[test]
    fn compose_holiday_lookup_answer_is_deterministic() {
        let result = HolidayLookupResult {
            country_code: "DE".to_string(),
            region_code: None,
            matches: vec![],
            as_of: SystemTime::UNIX_EPOCH,
        };
        let spoken = super::compose_holiday_lookup_answer(&result);
        assert_eq!(spoken, "No holiday matches found for DE.");
    }

    #[test]
    fn compose_fuel_price_lookup_answer_is_deterministic() {
        let result = FuelPriceLookupResult {
            country_code: "GB".to_string(),
            region: None,
            fuel_type: "diesel".to_string(),
            price: 1.589,
            currency: "GBP".to_string(),
            unit: "liter".to_string(),
            source_granularity: "national".to_string(),
            as_of: SystemTime::UNIX_EPOCH,
        };
        let spoken = super::compose_fuel_price_lookup_answer(&result);
        assert_eq!(spoken, "diesel fuel price in GB: 1.589 GBP per liter");
    }

    #[test]
    fn compose_horoscope_daily_answer_is_deterministic() {
        let result = HoroscopeDailyResult {
            sign: "Aries".to_string(),
            day: "today".to_string(),
            summary: "Good energy for focused work".to_string(),
            mood: None,
            color: None,
            lucky_number: None,
            as_of: SystemTime::UNIX_EPOCH,
        };
        let spoken = super::compose_horoscope_daily_answer(&result);
        assert_eq!(
            spoken,
            "Aries horoscope for today: Good energy for focused work"
        );
    }

    #[test]
    fn compose_news_headlines_answer_is_deterministic() {
        let result = NewsHeadlinesResult {
            headlines: vec![NewsHeadline {
                title: "Market rallies on AI demand".to_string(),
                source: Some("Reuters".to_string()),
                url: None,
                published_at: None,
            }],
            as_of: SystemTime::UNIX_EPOCH,
        };
        let spoken = super::compose_news_headlines_answer(&result);
        assert_eq!(spoken, "Top headlines: Market rallies on AI demand.");
    }
}
