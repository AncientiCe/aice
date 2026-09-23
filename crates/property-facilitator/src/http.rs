use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use core_observability::{
    record_fleet_firmware_download, record_property_auth_attempt, record_property_mcp_error,
};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::auth::{self, Role};
use crate::desk;
use crate::store::{EnrollOutcome, StaffSession, TicketStatus};
use crate::{Facilitator, FacilitatorError, FixedMcp, LoginOutcome, Running};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

pub async fn listen(facilitator: Arc<Facilitator>) -> Result<Running, FacilitatorError> {
    let listener = TcpListener::bind(&facilitator.settings.bind)
        .await
        .map_err(|error| FacilitatorError::Bind {
            bind: facilitator.settings.bind.clone(),
            message: error.to_string(),
        })?;
    let addr = listener
        .local_addr()
        .map_err(|error| FacilitatorError::Bind {
            bind: facilitator.settings.bind.clone(),
            message: error.to_string(),
        })?;
    let acceptor = match &facilitator.settings.tls {
        Some(files) => Some(core_tls::acceptor(&files.cert, &files.key)?),
        None => None,
    };
    let scheme = if acceptor.is_some() { "https" } else { "http" };
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let (stream, _) = match accepted {
                        Ok(value) => value,
                        Err(error) => {
                            tracing::warn!(%error, "property desk accept failed");
                            continue;
                        }
                    };
                    let state = Arc::clone(&facilitator);
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        match acceptor {
                            Some(acceptor) => match acceptor.accept(stream).await {
                                Ok(tls) => serve_connection(TokioIo::new(tls), state).await,
                                Err(error) => {
                                    record_property_mcp_error("tls_handshake");
                                    tracing::debug!(%error, "property desk tls handshake failed");
                                }
                            },
                            None => serve_connection(TokioIo::new(stream), state).await,
                        }
                    });
                }
            }
        }
    });
    Ok(Running {
        url: format!("{scheme}://{addr}"),
        shutdown: Some(shutdown_tx),
        task: Some(task),
        background: Vec::new(),
    })
}

async fn serve_connection<IO>(io: IO, state: Arc<Facilitator>)
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |request| {
        let state = Arc::clone(&state);
        async move { Ok::<_, Infallible>(route(state, request).await) }
    });
    if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
        tracing::warn!(%error, "property desk connection closed");
    }
}

struct FixedState {
    tools: Vec<String>,
    text: String,
    calls: Arc<AtomicU64>,
}

pub async fn listen_fixed(tools: Vec<String>, text: String) -> Result<FixedMcp, FacilitatorError> {
    let calls = Arc::new(AtomicU64::new(0));
    let state = Arc::new(FixedState {
        tools,
        text,
        calls: Arc::clone(&calls),
    });
    let listener =
        TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| FacilitatorError::Bind {
                bind: "127.0.0.1:0".to_string(),
                message: error.to_string(),
            })?;
    let addr = listener
        .local_addr()
        .map_err(|error| FacilitatorError::Bind {
            bind: "127.0.0.1:0".to_string(),
            message: error.to_string(),
        })?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let (stream, _) = match accepted {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let io = TokioIo::new(stream);
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        let service = service_fn(move |request| {
                            let state = Arc::clone(&state);
                            async move { Ok::<_, Infallible>(route_fixed(state, request).await) }
                        });
                        let _ = http1::Builder::new().serve_connection(io, service).await;
                    });
                }
            }
        }
    });
    Ok(FixedMcp {
        url: format!("http://{addr}/mcp"),
        calls,
        shutdown: Some(shutdown_tx),
        task: Some(task),
    })
}

/// Largest request body the facilitator reads.
const MAX_BODY_BYTES: usize = 256 * 1024;

struct DeskRequest {
    method: Method,
    path: String,
    query: String,
    content_type: String,
    authorization: Option<String>,
    cookie: Option<String>,
    csrf_header: Option<String>,
    body: Bytes,
}

async fn read_request(request: Request<Incoming>) -> Result<DeskRequest, Response<BoxBody>> {
    let header = |name: hyper::header::HeaderName| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();
    let content_type = header(hyper::header::CONTENT_TYPE).unwrap_or_default();
    let authorization = header(hyper::header::AUTHORIZATION);
    let cookie = header(hyper::header::COOKIE);
    let csrf_header = header(hyper::header::HeaderName::from_static("x-csrf-token"));
    let body = match Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
    {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            let status = if error.downcast_ref::<LengthLimitError>().is_some() {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            };
            return Err(json_response(status, json!({"error": error.to_string()})));
        }
    };
    Ok(DeskRequest {
        method,
        path,
        query,
        content_type,
        authorization,
        cookie,
        csrf_header,
        body,
    })
}

impl DeskRequest {
    fn is_form(&self) -> bool {
        self.content_type
            .starts_with("application/x-www-form-urlencoded")
    }

    fn form_field(&self, name: &str) -> Option<String> {
        form_urlencoded::parse(&self.body)
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }

    fn session_token(&self) -> Option<&str> {
        auth::cookie_value(self.cookie.as_deref(), auth::SESSION_COOKIE)
    }

    fn bearer(&self) -> Option<&str> {
        auth::bearer_token(self.authorization.as_deref())
    }

    /// Form posts carry the token as a field; API calls carry it as a header.
    fn csrf_matches(&self, expected: &str) -> bool {
        let presented = if self.is_form() {
            self.form_field("csrf")
        } else {
            self.csrf_header.clone()
        };
        presented.is_some_and(|value| auth::constant_time_eq(value.as_bytes(), expected.as_bytes()))
    }
}

async fn route(state: Arc<Facilitator>, request: Request<Incoming>) -> Response<BoxBody> {
    let incoming = match read_request(request).await {
        Ok(incoming) => incoming,
        Err(response) => return with_security_headers(response),
    };
    with_security_headers(dispatch(&state, incoming).await)
}

async fn dispatch(state: &Arc<Facilitator>, incoming: DeskRequest) -> Response<BoxBody> {
    let method = incoming.method.clone();
    let path = incoming.path.clone();
    if method == Method::GET && path == "/healthz" {
        return text_response(StatusCode::OK, "text/plain", "ok".to_string());
    }
    if method == Method::POST && path == "/mcp" {
        return mcp(state, &incoming).await;
    }
    if path == "/login" {
        return match method {
            Method::GET => login_page(state, None, StatusCode::OK),
            Method::POST => login(state, &incoming).await,
            _ => not_found(),
        };
    }
    if method == Method::GET && path == "/api/tls/ca" {
        return match state
            .settings
            .tls
            .as_ref()
            .and_then(|files| files.ca.as_ref())
        {
            Some(ca) => match std::fs::read_to_string(ca) {
                Ok(pem) => text_response(StatusCode::OK, "application/x-pem-file", pem),
                Err(error) => internal_error(&FacilitatorError::Io(error)),
            },
            None => not_found(),
        };
    }
    if method == Method::POST && path == "/api/devices/enroll" {
        return enroll_device(state, &incoming);
    }
    if method == Method::POST && path == "/api/devices/verify" {
        return verify_device(state, &incoming);
    }
    if method == Method::GET && path.starts_with("/api/firmware/") {
        return firmware_api(state, &incoming);
    }
    if method == Method::POST && path == "/api/devices/help" {
        return device_help(state, &incoming);
    }
    if method == Method::POST && path == "/api/devices/heartbeat" {
        return device_heartbeat(state, &incoming);
    }
    if path == "/api/stays/purge-due"
        || (path.starts_with("/api/stays/") && path.ends_with("/purged"))
    {
        return stay_purge_api(state, &incoming);
    }
    if method == Method::GET
        && path == "/api/tickets"
        && incoming
            .bearer()
            .is_some_and(|token| state.service_token_matches(token))
    {
        record_property_auth_attempt("service_token", "accepted");
        return tickets_json(state);
    }

    let session = match incoming.session_token() {
        Some(token) => match state.session(token) {
            Ok(session) => session.map(|session| (token.to_string(), session)),
            Err(error) => return internal_error(&error),
        },
        None => None,
    };
    let Some((token, session)) = session else {
        record_property_auth_attempt("session", "missing");
        return if method == Method::GET && path == "/desk" {
            redirect("/login")
        } else {
            json_response(StatusCode::UNAUTHORIZED, json!({"error": "login required"}))
        };
    };

    match (&method, path.as_str()) {
        (&Method::GET, "/desk") => match desk::desk_html(state, &session) {
            Ok(html) => html_response(StatusCode::OK, html),
            Err(error) => internal_error(&error),
        },
        (&Method::GET, "/api/tickets") => tickets_json(state),
        (&Method::GET, "/api/audit") => {
            if session.role != Role::Supervisor {
                return forbidden("supervisor role required");
            }
            match state.list_audit(200) {
                Ok(events) => json_response(StatusCode::OK, json!(events)),
                Err(error) => internal_error(&error),
            }
        }
        (&Method::POST, "/logout") => {
            if !incoming.csrf_matches(&session.csrf) {
                return forbidden("csrf token mismatch");
            }
            match state.logout(&token, &session.username) {
                Ok(()) => with_cookie(redirect("/login"), &clear_cookie()),
                Err(error) => internal_error(&error),
            }
        }
        (&Method::GET, "/api/stays") => match state.list_stays() {
            Ok(stays) => json_response(StatusCode::OK, json!(stays)),
            Err(error) => internal_error(&error),
        },
        (&Method::POST, "/api/stays") => {
            if !incoming.csrf_matches(&session.csrf) {
                return forbidden("csrf token mismatch");
            }
            open_stay(state, &incoming, &session)
        }
        (&Method::GET, "/api/devices") => match state.list_devices() {
            Ok(devices) => json_response(StatusCode::OK, json!(devices)),
            Err(error) => internal_error(&error),
        },
        (&Method::POST, _) => {
            if let Some((id, action)) = ticket_action(&path) {
                if !incoming.csrf_matches(&session.csrf) {
                    return forbidden("csrf token mismatch");
                }
                return change_ticket(state, &incoming, &session, id, action);
            }
            if let Some(id) = path
                .strip_prefix("/api/stays/")
                .and_then(|rest| rest.strip_suffix("/consent"))
            {
                if !incoming.csrf_matches(&session.csrf) {
                    return forbidden("csrf token mismatch");
                }
                let consent = if incoming.is_form() {
                    incoming.form_field("memory_consent").unwrap_or_default()
                } else {
                    serde_json::from_slice::<Value>(&incoming.body)
                        .ok()
                        .and_then(|body| {
                            body.get("memory_consent").map(|value| match value {
                                Value::Bool(true) => "yes".to_string(),
                                Value::String(text) => text.clone(),
                                _ => String::new(),
                            })
                        })
                        .unwrap_or_default()
                };
                return match state.set_stay_consent(id, parse_yes(&consent), &session.username) {
                    Ok(_) if incoming.is_form() => redirect("/desk"),
                    Ok(stay) => json_response(StatusCode::OK, json!(stay)),
                    Err(FacilitatorError::UnknownStay(missing)) => {
                        json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
                    }
                    Err(error) => internal_error(&error),
                };
            }
            if let Some(id) = path
                .strip_prefix("/api/stays/")
                .and_then(|rest| rest.strip_suffix("/close"))
            {
                if !incoming.csrf_matches(&session.csrf) {
                    return forbidden("csrf token mismatch");
                }
                return match state.close_stay(id, &session.username) {
                    Ok(_) if incoming.is_form() => redirect("/desk"),
                    Ok(stay) => json_response(StatusCode::OK, json!(stay)),
                    Err(FacilitatorError::UnknownStay(missing)) => {
                        json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
                    }
                    Err(error) => internal_error(&error),
                };
            }
            if let Some((id, action)) = device_action(&path) {
                if session.role != Role::Supervisor {
                    return forbidden("supervisor role required");
                }
                if !incoming.csrf_matches(&session.csrf) {
                    return forbidden("csrf token mismatch");
                }
                return change_device(state, &incoming, &session, id, action);
            }
            not_found()
        }
        _ => not_found(),
    }
}

async fn mcp(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let authorized = incoming
        .bearer()
        .is_some_and(|token| state.service_token_matches(token));
    if !authorized {
        record_property_auth_attempt("service_token", "rejected");
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "service token required"}),
        );
    }
    record_property_auth_attempt("service_token", "accepted");
    let value: Value = match serde_json::from_slice(&incoming.body) {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode::OK,
                json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": error.to_string() }
                }),
            )
        }
    };
    json_response(StatusCode::OK, state.handle_rpc(&value).await)
}

async fn login(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    if !incoming.is_form() {
        return login_page(
            state,
            Some("Unsupported login request."),
            StatusCode::BAD_REQUEST,
        );
    }
    let username = incoming.form_field("username").unwrap_or_default();
    let password = incoming.form_field("password").unwrap_or_default();
    // Argon2 is deliberately slow; keep it off the async workers.
    let checker = Arc::clone(state);
    let outcome =
        match tokio::task::spawn_blocking(move || checker.login(&username, &password)).await {
            Ok(outcome) => outcome,
            Err(error) => Err(FacilitatorError::Auth(error.to_string())),
        };
    match outcome {
        Ok(LoginOutcome::Accepted { token }) => with_cookie(
            redirect("/desk"),
            &session_cookie(&token, state.settings.tls.is_some()),
        ),
        Ok(LoginOutcome::Rejected) => login_page(
            state,
            Some("Wrong username or password."),
            StatusCode::UNAUTHORIZED,
        ),
        Ok(LoginOutcome::Locked) => login_page(
            state,
            Some("Too many failed logins. Try again in five minutes."),
            StatusCode::TOO_MANY_REQUESTS,
        ),
        Err(error) => internal_error(&error),
    }
}

fn login_page(
    state: &Arc<Facilitator>,
    message: Option<&str>,
    status: StatusCode,
) -> Response<BoxBody> {
    match desk::login_html(state, message) {
        Ok(html) => html_response(status, html),
        Err(error) => internal_error(&error),
    }
}

fn tickets_json(state: &Arc<Facilitator>) -> Response<BoxBody> {
    match state.list_tickets() {
        Ok(tickets) => json_response(StatusCode::OK, json!(tickets)),
        Err(error) => internal_error(&error),
    }
}

fn change_ticket(
    state: &Arc<Facilitator>,
    incoming: &DeskRequest,
    session: &StaffSession,
    id: &str,
    action: &str,
) -> Response<BoxBody> {
    let status = match action {
        "acknowledge" => TicketStatus::Acknowledged,
        "done" => TicketStatus::Done,
        "escalate" => TicketStatus::Escalated,
        "reopen" => {
            if session.role != Role::Supervisor {
                return forbidden("supervisor role required");
            }
            TicketStatus::Open
        }
        _ => return not_found(),
    };
    match state.set_status(id, status, &session.username) {
        Ok(_) if incoming.is_form() => redirect("/desk"),
        Ok(ticket) => json_response(StatusCode::OK, json!(ticket)),
        Err(FacilitatorError::TicketNotFound(missing)) => {
            json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
        }
        Err(error @ FacilitatorError::InvalidTransition { .. }) => {
            json_response(StatusCode::CONFLICT, json!({"error": error.to_string()}))
        }
        Err(error) => internal_error(&error),
    }
}

fn open_stay(
    state: &Arc<Facilitator>,
    incoming: &DeskRequest,
    session: &StaffSession,
) -> Response<BoxBody> {
    let field = |name: &str| -> String {
        if incoming.is_form() {
            incoming.form_field(name).unwrap_or_default()
        } else {
            serde_json::from_slice::<Value>(&incoming.body)
                .ok()
                .and_then(|body| body.get(name).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default()
        }
    };
    let room = field("room");
    let room = room.trim();
    if room.is_empty() || room.len() > 32 || room.chars().any(char::is_control) {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "room must be 1-32 printable characters"}),
        );
    }
    let continue_from = field("continue_from");
    let continue_from = Some(continue_from.trim()).filter(|value| !value.is_empty());
    let consent = field("memory_consent");
    let consent = (!consent.trim().is_empty()).then(|| parse_yes(&consent));
    match state.open_stay(room, continue_from, consent, &session.username) {
        Ok(_) if incoming.is_form() => redirect("/desk"),
        Ok(stay) => json_response(StatusCode::CREATED, json!(stay)),
        Err(error @ FacilitatorError::StayConflict(_)) => {
            json_response(StatusCode::CONFLICT, json!({"error": error.to_string()}))
        }
        Err(FacilitatorError::UnknownStay(missing)) => {
            json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
        }
        Err(error) => internal_error(&error),
    }
}

/// Backend-only: which stay memories to delete, and confirmation that one was deleted.
fn stay_purge_api(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let authorized = incoming
        .bearer()
        .is_some_and(|token| state.service_token_matches(token));
    if !authorized {
        record_property_auth_attempt("service_token", "rejected");
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "service token required"}),
        );
    }
    if incoming.method == Method::GET && incoming.path == "/api/stays/purge-due" {
        return match state.purge_due() {
            Ok(stays) => json_response(StatusCode::OK, json!(stays)),
            Err(error) => internal_error(&error),
        };
    }
    let id = incoming
        .path
        .strip_prefix("/api/stays/")
        .and_then(|rest| rest.strip_suffix("/purged"));
    match (incoming.method == Method::POST, id) {
        (true, Some(id)) => match state.mark_stay_purged(id) {
            Ok(()) => json_response(StatusCode::OK, json!({"id": id, "purged": true})),
            Err(FacilitatorError::UnknownStay(missing)) => {
                json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
            }
            Err(error) => internal_error(&error),
        },
        _ => not_found(),
    }
}

fn enroll_device(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let body: Value = serde_json::from_slice(&incoming.body).unwrap_or(Value::Null);
    let field = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or("").trim();
    let device_id = field("device_id");
    let nonce = field("nonce");
    let firmware = field("firmware");
    if !valid_device_id(device_id) || nonce.len() < 16 || nonce.len() > 256 || firmware.len() > 64 {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "device_id, nonce (16+ chars) and firmware are required"}),
        );
    }
    let lost_token = body
        .get("lost_token")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match state.enroll_device(device_id, nonce, firmware, lost_token) {
        Ok(EnrollOutcome::Pending) => {
            json_response(StatusCode::ACCEPTED, json!({"status": "pending"}))
        }
        Ok(EnrollOutcome::Active { room, token }) => json_response(
            StatusCode::OK,
            json!({"status": "active", "room": room, "token": token}),
        ),
        Ok(EnrollOutcome::Revoked) => {
            json_response(StatusCode::FORBIDDEN, json!({"status": "revoked"}))
        }
        Ok(EnrollOutcome::NonceMismatch) => json_response(
            StatusCode::CONFLICT,
            json!({"error": "device is enrolled with a different nonce"}),
        ),
        Ok(EnrollOutcome::TooManyPending) => json_response(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "too many pods are waiting for a room"}),
        ),
        Err(error) => internal_error(&error),
    }
}

fn verify_device(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let authorized = incoming
        .bearer()
        .is_some_and(|token| state.service_token_matches(token));
    if !authorized {
        record_property_auth_attempt("service_token", "rejected");
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "service token required"}),
        );
    }
    let body: Value = serde_json::from_slice(&incoming.body).unwrap_or(Value::Null);
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    match state.verify_device(token) {
        Ok(Some(identity)) => {
            record_property_auth_attempt("device_token", "accepted");
            json_response(StatusCode::OK, json!(identity))
        }
        Ok(None) => {
            record_property_auth_attempt("device_token", "rejected");
            json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "unknown device token"}),
            )
        }
        Err(error) => internal_error(&error),
    }
}

/// Pods ask for updates and download images with their device token.
fn firmware_api(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let identity = match incoming.bearer().map(|token| state.verify_device(token)) {
        Some(Ok(Some(identity))) => identity,
        Some(Err(error)) => return internal_error(&error),
        Some(Ok(None)) | None => {
            record_property_auth_attempt("device_token", "rejected");
            return json_response(
                StatusCode::UNAUTHORIZED,
                json!({"error": "device token required"}),
            );
        }
    };
    if incoming.path == "/api/firmware/manifest" {
        let current = form_urlencoded::parse(incoming.query.as_bytes())
            .find(|(key, _)| key == "current")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        return match state.firmware_offer(&identity.device_id, current.trim()) {
            Ok(Some(release)) => json_response(
                StatusCode::OK,
                json!({
                    "version": release.version,
                    "size": release.size,
                    "sha256_hex": release.sha256_hex,
                    "signature_hex": release.signature_hex,
                    "url": format!("/api/firmware/{}.bin", release.version),
                }),
            ),
            Ok(None) => empty_response(StatusCode::NO_CONTENT),
            Err(error) => internal_error(&error),
        };
    }
    let Some(version) = incoming
        .path
        .strip_prefix("/api/firmware/")
        .and_then(|rest| rest.strip_suffix(".bin"))
    else {
        return not_found();
    };
    match state.firmware_image(version) {
        Ok(Some(bytes)) => {
            record_fleet_firmware_download(version);
            bytes_response(bytes)
        }
        Ok(None) => not_found(),
        Err(error) => internal_error(&error),
    }
}

fn empty_response(status: StatusCode) -> Response<BoxBody> {
    let mut response = Response::new(Full::new(Bytes::new()).boxed());
    *response.status_mut() = status;
    response
}

fn bytes_response(bytes: Vec<u8>) -> Response<BoxBody> {
    let length = bytes.len();
    let mut response = Response::new(Full::new(Bytes::from(bytes)).boxed());
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        response
            .headers_mut()
            .insert(hyper::header::CONTENT_LENGTH, value);
    }
    response
}

fn device_help(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let authorized = incoming
        .bearer()
        .is_some_and(|token| state.service_token_matches(token));
    if !authorized {
        record_property_auth_attempt("service_token", "rejected");
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "service token required"}),
        );
    }
    let device_id = serde_json::from_slice::<Value>(&incoming.body)
        .ok()
        .and_then(|body| {
            body.get("device_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    match state.raise_help(&device_id) {
        Ok(Some(ticket)) => json_response(StatusCode::OK, json!({"ticket_id": ticket.id})),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            json!({"error": "no active pod with that id"}),
        ),
        Err(error) => internal_error(&error),
    }
}

fn device_heartbeat(state: &Arc<Facilitator>, incoming: &DeskRequest) -> Response<BoxBody> {
    let authorized = incoming
        .bearer()
        .is_some_and(|token| state.service_token_matches(token));
    if !authorized {
        record_property_auth_attempt("service_token", "rejected");
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "service token required"}),
        );
    }
    let device_ids: Vec<String> = serde_json::from_slice::<Value>(&incoming.body)
        .ok()
        .and_then(|body| body.get("device_ids").cloned())
        .and_then(|ids| serde_json::from_value(ids).ok())
        .unwrap_or_default();
    match state.heartbeat(&device_ids) {
        Ok(seen) => json_response(StatusCode::OK, json!({"seen": seen})),
        Err(error) => internal_error(&error),
    }
}

fn change_device(
    state: &Arc<Facilitator>,
    incoming: &DeskRequest,
    session: &StaffSession,
    id: &str,
    action: &str,
) -> Response<BoxBody> {
    let result = match action {
        "assign" => {
            let room = if incoming.is_form() {
                incoming.form_field("room").unwrap_or_default()
            } else {
                serde_json::from_slice::<Value>(&incoming.body)
                    .ok()
                    .and_then(|body| body.get("room").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_default()
            };
            let room = room.trim();
            if room.is_empty() || room.len() > 32 || room.chars().any(char::is_control) {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "room must be 1-32 printable characters"}),
                );
            }
            state.assign_device(id, room, &session.username)
        }
        "revoke" => state.revoke_device(id, &session.username),
        _ => return not_found(),
    };
    match result {
        Ok(()) if incoming.is_form() => redirect("/desk"),
        Ok(()) => json_response(StatusCode::OK, json!({"device_id": id})),
        Err(FacilitatorError::UnknownDevice(missing)) => {
            json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
        }
        Err(error) => internal_error(&error),
    }
}

/// Form and JSON consent values: yes/on/true/1 mean agreed.
fn parse_yes(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "on" | "true" | "1"
    )
}

fn valid_device_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
}

fn device_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/devices/")?;
    let (id, action) = rest.split_once('/')?;
    if !valid_device_id(id) || action.is_empty() || action.contains('/') {
        return None;
    }
    Some((id, action))
}

fn ticket_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/tickets/")?;
    let (id, action) = rest.split_once('/')?;
    if id.is_empty() || action.is_empty() || action.contains('/') {
        return None;
    }
    Some((id, action))
}

fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        auth::SESSION_COOKIE,
        auth::SESSION_MILLIS / 1000,
        if secure { "; Secure" } else { "" }
    )
}

fn clear_cookie() -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        auth::SESSION_COOKIE
    )
}

fn with_cookie(mut response: Response<BoxBody>, cookie: &str) -> Response<BoxBody> {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response
            .headers_mut()
            .insert(hyper::header::SET_COOKIE, value);
    }
    response
}

fn with_security_headers(mut response: Response<BoxBody>) -> Response<BoxBody> {
    let headers = response.headers_mut();
    headers.insert(
        hyper::header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        hyper::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        hyper::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        hyper::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        hyper::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

fn forbidden(message: &str) -> Response<BoxBody> {
    json_response(StatusCode::FORBIDDEN, json!({"error": message}))
}

fn not_found() -> Response<BoxBody> {
    text_response(StatusCode::NOT_FOUND, "text/plain", "not found".to_string())
}

fn internal_error(error: &FacilitatorError) -> Response<BoxBody> {
    tracing::warn!(%error, "property desk request failed");
    json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": "internal error"}),
    )
}

fn html_response(status: StatusCode, html: String) -> Response<BoxBody> {
    text_response(status, "text/html; charset=utf-8", html)
}

fn redirect(location: &'static str) -> Response<BoxBody> {
    let mut response = Response::new(Full::new(Bytes::from_static(b"")).boxed());
    *response.status_mut() = StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert(hyper::header::LOCATION, HeaderValue::from_static(location));
    response
}

async fn route_fixed(state: Arc<FixedState>, request: Request<Incoming>) -> Response<BoxBody> {
    if request.method() != Method::POST || request.uri().path() != "/mcp" {
        return text_response(StatusCode::NOT_FOUND, "text/plain", "not found".to_string());
    }
    let body = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, json!({"error": error.to_string()}))
        }
    };
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode::OK,
                json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": error.to_string() }
                }),
            )
        }
    };
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let response = match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "property-mcp", "version": "0.1.0" }
            }
        }),
        "tools/list" => {
            let tools: Vec<Value> = state
                .tools
                .iter()
                .map(|name| json!({"name": name, "inputSchema": {"type": "object"}}))
                .collect();
            json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
        }
        "tools/call" => {
            state.calls.fetch_add(1, Ordering::SeqCst);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": state.text }],
                    "isError": false
                }
            })
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "method not found" }
        }),
    };
    json_response(StatusCode::OK, response)
}

fn json_response(status: StatusCode, body: Value) -> Response<BoxBody> {
    text_response(status, "application/json", body.to_string())
}

fn text_response(status: StatusCode, content_type: &str, body: String) -> Response<BoxBody> {
    match Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(body)).boxed())
    {
        Ok(response) => response,
        Err(_) => Response::new(Full::new(Bytes::from_static(b"")).boxed()),
    }
}
