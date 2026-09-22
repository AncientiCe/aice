use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::store::TicketStatus;
use crate::{Facilitator, FacilitatorError, FixedMcp, Running};

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
                    let io = TokioIo::new(stream);
                    let state = Arc::clone(&facilitator);
                    tokio::spawn(async move {
                        let service = service_fn(move |request| {
                            let state = Arc::clone(&state);
                            async move { Ok::<_, Infallible>(route(state, request).await) }
                        });
                        if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                            tracing::warn!(%error, "property desk connection closed");
                        }
                    });
                }
            }
        }
    });
    Ok(Running {
        url: format!("http://{addr}"),
        shutdown: Some(shutdown_tx),
        task: Some(task),
    })
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

async fn route(state: Arc<Facilitator>, request: Request<Incoming>) -> Response<BoxBody> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let content_type = request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return json_response(StatusCode::BAD_REQUEST, json!({"error": error.to_string()}))
        }
    };

    if method == Method::POST && path == "/mcp" {
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
        let response = state.handle_rpc(&value).await;
        return json_response(StatusCode::OK, response);
    }
    if method == Method::GET && path == "/desk" {
        return match state.desk_html() {
            Ok(html) => text_response(StatusCode::OK, "text/html; charset=utf-8", html),
            Err(error) => text_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                error.to_string(),
            ),
        };
    }
    if method == Method::GET && path == "/api/tickets" {
        return match state.list_tickets() {
            Ok(tickets) => json_response(StatusCode::OK, json!(tickets)),
            Err(error) => json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": error.to_string()}),
            ),
        };
    }
    if method == Method::POST {
        if let Some((id, action)) = ticket_action(&path) {
            let status = match action {
                "acknowledge" => TicketStatus::Acknowledged,
                "done" => TicketStatus::Done,
                "escalate" => TicketStatus::Escalated,
                _ => {
                    return json_response(StatusCode::NOT_FOUND, json!({"error": "not found"}));
                }
            };
            return match state.set_status(id, status) {
                Ok(ticket) if content_type.starts_with("application/x-www-form-urlencoded") => {
                    redirect("/desk")
                        .unwrap_or_else(|_| json_response(StatusCode::OK, json!(ticket)))
                }
                Ok(ticket) => json_response(StatusCode::OK, json!(ticket)),
                Err(FacilitatorError::TicketNotFound(missing)) => {
                    json_response(StatusCode::NOT_FOUND, json!({"error": missing}))
                }
                Err(error) => {
                    json_response(StatusCode::CONFLICT, json!({"error": error.to_string()}))
                }
            };
        }
    }
    text_response(StatusCode::NOT_FOUND, "text/plain", "not found".to_string())
}

fn ticket_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/tickets/")?;
    let (id, action) = rest.split_once('/')?;
    if id.is_empty() || action.is_empty() {
        return None;
    }
    Some((id, action))
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

fn redirect(location: &str) -> Result<Response<BoxBody>, ()> {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(hyper::header::LOCATION, location)
        .body(Full::new(Bytes::from_static(b"")).boxed())
        .map_err(|_| ())
}
