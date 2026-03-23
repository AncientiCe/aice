pub mod discovery_broadcast;

use async_trait::async_trait;
use bytes::Bytes;
use core_config::Config;
use core_llm::OllamaLlmStream;
use core_observability::{
    record_backend_dependency_request, record_backend_dependency_request_duration,
    record_backend_http_request, record_backend_turn_duration, record_backend_turn_stage_duration,
    record_backend_turn_total,
};
use core_orchestrator::{
    intent_classifier_few_shots, intent_classifier_system_prompt_for_skills, parse_intent,
    validate_intent_decision, IntentClassifier, IntentDecision, LlmCallOptions, LlmStream,
};
use core_runtime_protocol::{
    sse_data_line, FrontendActivateRequest, FrontendDeactivateRequest, FrontendHeartbeatRequest,
    FrontendSkillIntent, FrontendSkillResultRequest, RuntimeEvent, TurnChunkRequest, TurnRequest,
    CURRENT_PROTOCOL_VERSION,
};
use core_skills::{
    DistanceResult, DistanceSkill, HueSmartHomeSkill, MemorySkill, OpenMeteoDistanceSkill,
    OpenMeteoTimeSkill, OpenMeteoWeatherSkill, ResolvedLocation, SmartHomeSkill, SqliteMemorySkill,
    TimeResult, TimeSkill, WeatherResult, WeatherSkill,
};
use futures::StreamExt;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};

pub type DynError = Box<dyn Error + Send + Sync>;
type RespBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;
type SessionHistory = Arc<Mutex<HashMap<String, Vec<(String, String)>>>>;
type TurnChunks = Arc<Mutex<HashMap<String, String>>>;
type FrontendSessions = Arc<Mutex<HashMap<FrontendSessionKey, FrontendSessionState>>>;

const DEFAULT_FRONTEND_SESSION_TTL_SECONDS: u64 = 120;
const CLASSIFIER_ALL_SKILLS: [&str; 15] = [
    "skill_weather",
    "skill_time",
    "skill_distance",
    "skill_smart_home",
    "skill_assistant",
    "skill_media",
    "skill_memory",
    "skill_computer",
    "skill_screenshot",
    "skill_app_switcher",
    "skill_reminder",
    "skill_timer",
    "skill_shopping_list",
    "skill_message",
    "skill_volume",
];
const FRONTEND_CLASSIFIER_SKILLS: [&str; 10] = [
    "skill_assistant",
    "skill_media",
    "skill_computer",
    "skill_screenshot",
    "skill_app_switcher",
    "skill_reminder",
    "skill_timer",
    "skill_shopping_list",
    "skill_message",
    "skill_volume",
];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FrontendSessionKey {
    device_id: String,
    session_id: String,
}

#[derive(Clone, Debug)]
struct FrontendSessionState {
    supported_frontend_intents: Vec<String>,
    expires_at: Instant,
}

#[derive(Clone)]
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
        request: FrontendSkillResultRequest,
    ) -> Result<String, DynError>;
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
    let listener = TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let pending_chunks: TurnChunks = Arc::new(Mutex::new(HashMap::new()));
    let frontend_sessions: FrontendSessions = Arc::new(Mutex::new(HashMap::new()));
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
                    let pending_chunks = pending_chunks.clone();
                    let frontend_sessions = frontend_sessions.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let engine = engine.clone();
                            let pending_chunks = pending_chunks.clone();
                            let frontend_sessions = frontend_sessions.clone();
                            async move { handle_request(req, engine, pending_chunks, frontend_sessions).await }
                        });
                        if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
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

fn sse_response(events: &[RuntimeEvent]) -> Response<RespBody> {
    let body = events
        .iter()
        .filter_map(|event| sse_data_line(event).ok())
        .collect::<Vec<_>>()
        .join("");
    let mut response = Response::new(full_body(body));
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("text/event-stream"),
    );
    response.headers_mut().insert(
        hyper::header::CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-cache"),
    );
    response
}

fn sse_response_timed(events: &[RuntimeEvent]) -> Response<RespBody> {
    let started_at = Instant::now();
    let response = sse_response(events);
    record_backend_turn_stage_duration("sse_write", started_at.elapsed());
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

async fn decode_json<T: for<'de> serde::Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, DynError> {
    let bytes = req.collect().await?.to_bytes();
    Ok(serde_json::from_slice::<T>(&bytes)?)
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

async fn register_frontend_session(
    sessions: &FrontendSessions,
    request: &FrontendActivateRequest,
) -> Result<(), String> {
    let key = build_frontend_session_key(&request.device_id, &request.session_id)
        .ok_or_else(|| "device_id and session_id are required".to_string())?;
    let supported_frontend_intents =
        normalize_supported_frontend_intents(&request.supported_frontend_intents);
    let ttl_secs = request
        .expires_in_seconds
        .unwrap_or(DEFAULT_FRONTEND_SESSION_TTL_SECONDS);
    let expires_at = Instant::now() + Duration::from_secs(ttl_secs);
    let mut guard = sessions.lock().await;
    guard.insert(
        key,
        FrontendSessionState {
            supported_frontend_intents,
            expires_at,
        },
    );
    Ok(())
}

async fn refresh_frontend_session(
    sessions: &FrontendSessions,
    request: &FrontendHeartbeatRequest,
) -> Result<bool, String> {
    let key = build_frontend_session_key(&request.device_id, &request.session_id)
        .ok_or_else(|| "device_id and session_id are required".to_string())?;
    let mut guard = sessions.lock().await;
    let Some(state) = guard.get_mut(&key) else {
        return Ok(false);
    };
    state.expires_at = Instant::now() + Duration::from_secs(DEFAULT_FRONTEND_SESSION_TTL_SECONDS);
    Ok(true)
}

async fn remove_frontend_session(
    sessions: &FrontendSessions,
    request: &FrontendDeactivateRequest,
) -> Result<bool, String> {
    let key = build_frontend_session_key(&request.device_id, &request.session_id)
        .ok_or_else(|| "device_id and session_id are required".to_string())?;
    let mut guard = sessions.lock().await;
    Ok(guard.remove(&key).is_some())
}

async fn lookup_frontend_capabilities(
    sessions: &FrontendSessions,
    device_id: Option<&str>,
    session_id: &str,
) -> Option<Vec<String>> {
    let key = build_frontend_session_key(device_id?, session_id)?;
    let now = Instant::now();
    let mut guard = sessions.lock().await;
    guard.retain(|_, state| state.expires_at > now);
    let state = guard.get(&key)?;
    Some(state.supported_frontend_intents.clone())
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

fn build_available_classifier_skills(
    smart_home_enabled: bool,
    memory_enabled: bool,
    context: Option<&Value>,
) -> Vec<String> {
    let mut skills = vec![
        "skill_weather".to_string(),
        "skill_time".to_string(),
        "skill_distance".to_string(),
    ];
    if smart_home_enabled {
        skills.push("skill_smart_home".to_string());
    }
    if memory_enabled {
        skills.push("skill_memory".to_string());
    }
    let frontend_intents = frontend_classifier_skills_from_context(context);
    for skill in FRONTEND_CLASSIFIER_SKILLS {
        if frontend_intents
            .iter()
            .any(|intent| intent.as_str() == skill)
        {
            skills.push(skill.to_string());
        }
    }
    skills
}

async fn handle_request(
    req: Request<Incoming>,
    engine: Arc<dyn BackendEngine>,
    pending_chunks: TurnChunks,
    frontend_sessions: FrontendSessions,
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

    if method == Method::POST && path == "/v1/frontends/activate" {
        let started_at = Instant::now();
        let request = match decode_json::<FrontendActivateRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                record_backend_turn_total("frontend_activate", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/activate",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        info!(
            device_id = %request.device_id,
            session_id = %request.session_id,
            platform = %request.platform,
            frontend_version = %request.frontend_version,
            supported_frontend_intents = ?request.supported_frontend_intents,
            expires_in_seconds = ?request.expires_in_seconds,
            protocol_version = ?request.protocol_version,
            "frontend activate request received"
        );
        if let Some(version) = request.protocol_version {
            if version != CURRENT_PROTOCOL_VERSION {
                record_backend_turn_total("frontend_activate", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/activate",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("unsupported protocol_version {version}; expected {CURRENT_PROTOCOL_VERSION}")}),
                    ),
                ));
            }
        }
        let status = match register_frontend_session(&frontend_sessions, &request).await {
            Ok(()) => {
                record_backend_turn_total("frontend_activate", "success");
                StatusCode::ACCEPTED
            }
            Err(error) => {
                record_backend_turn_total("frontend_activate", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/activate",
                    request_started_at,
                    json_response(StatusCode::BAD_REQUEST, json!({ "error": error })),
                ));
            }
        };
        record_backend_turn_duration("frontend_activate", started_at.elapsed());
        return Ok(with_backend_http_metrics(
            &method,
            "/v1/frontends/activate",
            request_started_at,
            plain_response(status, "accepted"),
        ));
    }

    if method == Method::POST && path == "/v1/frontends/heartbeat" {
        let started_at = Instant::now();
        let request = match decode_json::<FrontendHeartbeatRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                record_backend_turn_total("frontend_heartbeat", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/heartbeat",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        let refreshed = match refresh_frontend_session(&frontend_sessions, &request).await {
            Ok(found) => found,
            Err(error) => {
                record_backend_turn_total("frontend_heartbeat", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/heartbeat",
                    request_started_at,
                    json_response(StatusCode::BAD_REQUEST, json!({ "error": error })),
                ));
            }
        };
        record_backend_turn_total(
            "frontend_heartbeat",
            if refreshed { "success" } else { "missing" },
        );
        record_backend_turn_duration("frontend_heartbeat", started_at.elapsed());
        return Ok(with_backend_http_metrics(
            &method,
            "/v1/frontends/heartbeat",
            request_started_at,
            plain_response(StatusCode::ACCEPTED, "accepted"),
        ));
    }

    if method == Method::POST && path == "/v1/frontends/deactivate" {
        let started_at = Instant::now();
        let request = match decode_json::<FrontendDeactivateRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                record_backend_turn_total("frontend_deactivate", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/deactivate",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        let removed = match remove_frontend_session(&frontend_sessions, &request).await {
            Ok(found) => found,
            Err(error) => {
                record_backend_turn_total("frontend_deactivate", "error");
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/frontends/deactivate",
                    request_started_at,
                    json_response(StatusCode::BAD_REQUEST, json!({ "error": error })),
                ));
            }
        };
        record_backend_turn_total(
            "frontend_deactivate",
            if removed { "success" } else { "missing" },
        );
        record_backend_turn_duration("frontend_deactivate", started_at.elapsed());
        return Ok(with_backend_http_metrics(
            &method,
            "/v1/frontends/deactivate",
            request_started_at,
            plain_response(StatusCode::ACCEPTED, "accepted"),
        ));
    }

    if method == Method::POST && path == "/v1/turns/chunks" {
        let request = match decode_json::<TurnChunkRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/turns/chunks",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        let chunk = request.chunk.trim();
        if !chunk.is_empty() {
            let mut chunks = pending_chunks.lock().await;
            let entry = chunks.entry(request.session_id).or_default();
            if !entry.is_empty() {
                entry.push(' ');
            }
            entry.push_str(chunk);
        }
        return Ok(with_backend_http_metrics(
            &method,
            "/v1/turns/chunks",
            request_started_at,
            plain_response(StatusCode::ACCEPTED, "accepted"),
        ));
    }

    if method == Method::POST && path == "/v1/turns" {
        let mut request = match decode_json::<TurnRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/turns",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        if !request.finalize {
            let transcript = request.transcript.trim();
            if !transcript.is_empty() {
                let mut chunks = pending_chunks.lock().await;
                let entry = chunks.entry(request.session_id).or_default();
                if !entry.is_empty() {
                    entry.push(' ');
                }
                entry.push_str(transcript);
            }
            return Ok(with_backend_http_metrics(
                &method,
                "/v1/turns",
                request_started_at,
                plain_response(StatusCode::ACCEPTED, "buffered"),
            ));
        }
        let buffered = {
            let mut chunks = pending_chunks.lock().await;
            chunks.remove(&request.session_id).unwrap_or_default()
        };
        if request.transcript.trim().is_empty() {
            request.transcript = buffered;
        } else if !buffered.trim().is_empty() {
            request.transcript = format!("{} {}", buffered.trim(), request.transcript.trim());
        }
        let frontend_capabilities = lookup_frontend_capabilities(
            &frontend_sessions,
            request.device_id.as_deref(),
            &request.session_id,
        )
        .await;
        if let Some(capabilities) = frontend_capabilities.as_ref() {
            let context_value = request.context.take().unwrap_or_else(|| json!({}));
            let mut context_object = match context_value {
                Value::Object(obj) => obj,
                other => {
                    let mut obj = serde_json::Map::new();
                    obj.insert("upstream_context".to_string(), other);
                    obj
                }
            };
            context_object.insert(
                "frontend_supported_intents".to_string(),
                json!(capabilities),
            );
            request.context = Some(Value::Object(context_object));
        }
        info!(
            session_id = %request.session_id,
            device_id = request.device_id.as_deref().unwrap_or(""),
            turn_id = request.turn_id.as_deref().unwrap_or(""),
            transcript = %request.transcript,
            "backend turn request received"
        );

        let started_at = Instant::now();
        let result = engine.process_turn(request).await;
        record_backend_turn_duration("turn", started_at.elapsed());

        return Ok(with_backend_http_metrics(
            &method,
            "/v1/turns",
            request_started_at,
            match result {
                Ok(BackendEngineDecision::Chat(text)) => {
                    info!("backend routed turn to chat");
                    record_backend_turn_total("chat", "success");
                    sse_response_timed(&[RuntimeEvent::Token { text }, RuntimeEvent::Done])
                }
                Ok(BackendEngineDecision::BackendSkill(text)) => {
                    info!("backend routed turn to backend skill");
                    record_backend_turn_total("backend_skill", "success");
                    sse_response_timed(&[RuntimeEvent::Token { text }, RuntimeEvent::Done])
                }
                Ok(BackendEngineDecision::FrontendSkillIntent(intent)) => {
                    if !frontend_intent_allowed(&intent.intent, frontend_capabilities.as_deref()) {
                        record_backend_turn_total("frontend_skill_capability_gate", "fallback");
                        sse_response_timed(&[
                            RuntimeEvent::Token {
                                text: "That action is not available on this active frontend."
                                    .to_string(),
                            },
                            RuntimeEvent::Done,
                        ])
                    } else {
                        info!(
                            intent = %intent.intent,
                            turn_id = %intent.turn_id,
                            "backend routed turn to frontend skill intent"
                        );
                        record_backend_turn_total("frontend_skill", "success");
                        sse_response_timed(&[
                            RuntimeEvent::FrontendSkillIntent(intent),
                            RuntimeEvent::Done,
                        ])
                    }
                }
                Err(error) => {
                    warn!(%error, "backend turn request failed");
                    record_backend_turn_total("turn", "error");
                    sse_response_timed(&[
                        RuntimeEvent::Error {
                            message: format!("backend error: {error}"),
                        },
                        RuntimeEvent::Done,
                    ])
                }
            },
        ));
    }

    if method == Method::POST
        && path.starts_with("/v1/turns/")
        && path.ends_with("/frontend-skill-result")
    {
        let turn_id = path
            .trim_start_matches("/v1/turns/")
            .trim_end_matches("/frontend-skill-result")
            .trim_end_matches('/');
        let request = match decode_json::<FrontendSkillResultRequest>(req).await {
            Ok(value) => value,
            Err(error) => {
                return Ok(with_backend_http_metrics(
                    &method,
                    "/v1/turns/:turn_id/frontend-skill-result",
                    request_started_at,
                    json_response(
                        StatusCode::BAD_REQUEST,
                        json!({"error": format!("invalid request body: {error}")}),
                    ),
                ));
            }
        };
        info!(
            turn_id = %turn_id,
            status = %request.status,
            "backend frontend-skill-result received"
        );

        let started_at = Instant::now();
        let result = engine.finalize_frontend_skill(turn_id, request).await;
        record_backend_turn_duration("frontend_skill_finalize", started_at.elapsed());

        return Ok(with_backend_http_metrics(
            &method,
            "/v1/turns/:turn_id/frontend-skill-result",
            request_started_at,
            match result {
                Ok(text) => {
                    record_backend_turn_total("frontend_skill_finalize", "success");
                    sse_response_timed(&[RuntimeEvent::Token { text }, RuntimeEvent::Done])
                }
                Err(error) => {
                    record_backend_turn_total("frontend_skill_finalize", "error");
                    sse_response_timed(&[
                        RuntimeEvent::Error {
                            message: format!("finalize error: {error}"),
                        },
                        RuntimeEvent::Done,
                    ])
                }
            },
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
    llm: Arc<OllamaLlmStream>,
    weather_skill: OpenMeteoWeatherSkill,
    time_skill: OpenMeteoTimeSkill,
    distance_skill: OpenMeteoDistanceSkill,
    smart_home_skill: Option<HueSmartHomeSkill>,
    memory_skill: Option<SqliteMemorySkill>,
    session_history: SessionHistory,
    turn_counter: Arc<std::sync::atomic::AtomicU64>,
    resolved_location: Option<ResolvedLocation>,
    skip_secondary_llm_for_skill_answers: bool,
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
            system_prompt: intent_classifier_system_prompt_for_skills(&CLASSIFIER_ALL_SKILLS),
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
    pub async fn from_config(config: &Config) -> Self {
        let llm = OllamaLlmStream::new(
            config.ollama_url.clone(),
            config.model.clone(),
            config.llm.short_replies,
            config.llm.max_output_tokens,
            config.llm.system_prompt.clone(),
        );
        let weather_skill = OpenMeteoWeatherSkill::new();
        let resolved_location = resolve_startup_location(config, &weather_skill).await;
        let smart_home_skill = if config.smart_home.hue.enabled {
            match (
                config.smart_home.hue.bridge_host.as_deref(),
                config.smart_home.hue.app_key.as_deref(),
            ) {
                (Some(host), Some(key)) => Some(HueSmartHomeSkill::new(
                    host,
                    key,
                    &config.smart_home.hue.default_light_name,
                )),
                _ => None,
            }
        } else {
            None
        };

        let memory_skill = if config.memory.enabled {
            SqliteMemorySkill::new(std::path::Path::new(&config.memory.sqlite_path)).ok()
        } else {
            None
        };

        Self {
            llm: Arc::new(llm),
            weather_skill,
            time_skill: OpenMeteoTimeSkill::new(),
            distance_skill: OpenMeteoDistanceSkill::new(),
            smart_home_skill,
            memory_skill,
            session_history: Arc::new(Mutex::new(HashMap::new())),
            turn_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            resolved_location,
            skip_secondary_llm_for_skill_answers: config.llm.skip_secondary_llm_for_skill_answers,
        }
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
        info!(
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
        let mut output = String::new();
        while let Some(token) = stream.next().await {
            output.push_str(&token);
        }
        info!(
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
        let classifier_prompt = intent_classifier_system_prompt_for_skills(available_skills);
        let few_shot_history = intent_classifier_few_shots();
        let classification_options = LlmCallOptions::for_classification();
        let raw = self
            .collect_llm(
                "intent_classification",
                &prompt,
                few_shot_history.as_slice(),
                Some(classifier_prompt.as_str()),
                Some(&classification_options),
            )
            .await?;
        let decision = parse_intent(raw.trim())?;
        Ok(validate_intent_decision(decision))
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
        let request_session_id = request.session_id.clone();
        let available_skills = build_available_classifier_skills(
            self.smart_home_skill.is_some(),
            self.memory_skill.is_some(),
            request.context.as_ref(),
        );
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
        record_backend_turn_stage_duration("classify_intent", classify_started.elapsed());

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
            IntentDecision::SkillSmartHome { target, action } => {
                if let Some(skill) = self.smart_home_skill.as_ref() {
                    let skill_started = Instant::now();
                    let result = skill
                        .execute(target.as_deref(), action.as_deref())
                        .await
                        .map_err(|error| format!("smart-home skill failed: {error}"))?;
                    record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                    let context = result.to_prompt_context();
                    let answer = if self.skip_secondary_llm_for_skill_answers {
                        compose_direct_skill_answer(&context)
                            .unwrap_or_else(|| "Smart home action completed.".to_string())
                    } else {
                        let compose_started = Instant::now();
                        let composed = self
                            .compose_skill_answer(&request.transcript, &context)
                            .await?;
                        record_backend_turn_stage_duration(
                            "answer_compose",
                            compose_started.elapsed(),
                        );
                        composed
                    };
                    Ok(BackendEngineDecision::BackendSkill(answer))
                } else {
                    Ok(BackendEngineDecision::BackendSkill(
                        "Smart home is not configured.".to_string(),
                    ))
                }
            }
            IntentDecision::SkillMemory { query, store } => {
                if let Some(skill) = self.memory_skill.as_ref() {
                    let skill_started = Instant::now();
                    let result = skill
                        .execute(query.as_deref(), store)
                        .await
                        .map_err(|error| format!("memory skill failed: {error}"))?;
                    record_backend_turn_stage_duration("skill_execute", skill_started.elapsed());
                    let context = result.to_prompt_context();
                    let answer = if self.skip_secondary_llm_for_skill_answers {
                        compose_direct_skill_answer(&context)
                            .unwrap_or_else(|| "Memory updated.".to_string())
                    } else {
                        let compose_started = Instant::now();
                        let composed = self
                            .compose_skill_answer(&request.transcript, &context)
                            .await?;
                        record_backend_turn_stage_duration(
                            "answer_compose",
                            compose_started.elapsed(),
                        );
                        composed
                    };
                    Ok(BackendEngineDecision::BackendSkill(answer))
                } else {
                    Ok(BackendEngineDecision::BackendSkill(
                        "Memory is not configured.".to_string(),
                    ))
                }
            }
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
            IntentDecision::SkillAssistant { kind } => Ok(build_frontend_intent(
                "skill_assistant",
                json!({"assistant_kind": kind}),
            )),
            IntentDecision::Chat => {
                let history = {
                    let sessions = self.session_history.lock().await;
                    sessions
                        .get(&request_session_id)
                        .cloned()
                        .unwrap_or_default()
                };
                let chat_started = Instant::now();
                let text = self
                    .collect_llm("chat", &request.transcript, &history, None, None)
                    .await?;
                record_backend_turn_stage_duration("chat_generate", chat_started.elapsed());
                {
                    let mut sessions = self.session_history.lock().await;
                    let entry = sessions.entry(request_session_id).or_default();
                    entry.push((request_text, text.clone()));
                    if entry.len() > 12 {
                        let keep_from = entry.len().saturating_sub(12);
                        entry.drain(0..keep_from);
                    }
                }
                Ok(BackendEngineDecision::Chat(text))
            }
        }
    }

    async fn finalize_frontend_skill(
        &self,
        _turn_id: &str,
        request: FrontendSkillResultRequest,
    ) -> Result<String, DynError> {
        Ok(compose_frontend_skill_outcome(&request))
    }
}

fn compose_frontend_skill_outcome(request: &FrontendSkillResultRequest) -> String {
    if request.status.eq_ignore_ascii_case("error") {
        let fallback = request
            .error
            .clone()
            .unwrap_or_else(|| "The action failed.".to_string());
        return format!("I could not complete that action: {fallback}");
    }

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
        build_available_classifier_skills, build_intent_classification_prompt,
        compose_distance_answer, compose_frontend_skill_outcome, compose_time_answer,
        compose_weather_answer,
    };
    use core_orchestrator::intent_classifier_few_shots;
    use core_runtime_protocol::FrontendSkillResultRequest;
    use core_skills::{DistanceResult, TimeResult, WeatherResult};
    use serde_json::json;

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
                u.contains("send a message to John saying running late")
                    && a.contains("\"intent\":\"skill_message\"")
                    && a.contains("\"command\":\"send\"")
                    && a.contains("\"message_contact\":\"John\"")
                    && a.contains("\"message_text\":\"running late\"")
            }),
            "expected canonical message send few-shot contract example"
        );
        assert!(
            examples.iter().any(|(u, a)| {
                u.contains("send a message to my wife.")
                    && a.contains("\"intent\":\"skill_message\"")
                    && a.contains("\"command\":\"send\"")
                    && a.contains("\"message_contact\":\"my wife\"")
                    && !a.contains("\"message_text\"")
            }),
            "expected no-invention message example for missing message text"
        );
    }

    #[test]
    fn frontend_skill_success_returns_outcome_without_llm_rewrite() {
        let request = FrontendSkillResultRequest {
            status: "ok".to_string(),
            user_text: "ask my wife how she is".to_string(),
            structured_result_context: Some(
                "Sent iMessage to Tetiana. Message: \"How are you?\".".to_string(),
            ),
            error: None,
        };

        let spoken = compose_frontend_skill_outcome(&request);
        assert_eq!(
            spoken,
            "Sent iMessage to Tetiana. Message: \"How are you?\"."
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

        let spoken = compose_frontend_skill_outcome(&request);
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
        let skills = build_available_classifier_skills(true, false, Some(&context));
        assert_eq!(
            skills,
            vec![
                "skill_weather".to_string(),
                "skill_time".to_string(),
                "skill_distance".to_string(),
                "skill_smart_home".to_string(),
                "skill_timer".to_string(),
                "skill_message".to_string(),
            ]
        );
    }
}
