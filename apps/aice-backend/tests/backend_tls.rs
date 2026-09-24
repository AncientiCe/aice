//! The backend's own listener over TLS, trusted through the property CA.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aice_backend::{
    server_options_from_config, spawn_server_with_options, AudioIngressConfig, AudioTranscriber,
    BackendEngine, BackendEngineDecision, ServerOptions,
};
use async_trait::async_trait;
use core_runtime_protocol::{FrontendSkillResultRequest, TurnRequest};
use tokio::net::TcpStream;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct Echo;

#[async_trait]
impl BackendEngine for Echo {
    async fn process_turn(
        &self,
        request: TurnRequest,
    ) -> Result<BackendEngineDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(BackendEngineDecision::Chat(request.transcript))
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

struct Silent;

#[async_trait]
impl AudioTranscriber for Silent {
    async fn transcribe(
        &self,
        _samples: Vec<i16>,
        _sample_rate_hz: u32,
        _channels: u16,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(String::new())
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!(
        "aice-backend-tls-{label}-{}-{nanos}",
        std::process::id()
    ))
}

#[tokio::test]
async fn turn_stream_is_served_over_tls() {
    let dir = temp_dir("wss");
    let (cert, key) =
        core_tls::ensure_server_cert(&dir, &["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap_or_else(|error| panic!("cert failed: {error}"));
    let acceptor =
        core_tls::acceptor(&cert, &key).unwrap_or_else(|error| panic!("acceptor: {error}"));
    let engine: Arc<dyn BackendEngine> = Arc::new(Echo);
    let transcriber: Arc<dyn AudioTranscriber> = Arc::new(Silent);
    let handle = spawn_server_with_options(
        "127.0.0.1:0",
        engine,
        transcriber,
        AudioIngressConfig::default(),
        ServerOptions {
            tls: Some(acceptor),
            ..ServerOptions::default()
        },
    )
    .await
    .unwrap_or_else(|error| panic!("backend failed: {error}"));

    let config = core_tls::client_config_trusting(&dir.join("ca.pem"))
        .unwrap_or_else(|error| panic!("client config: {error}"));
    let stream = TcpStream::connect(&handle.bind)
        .await
        .unwrap_or_else(|error| panic!("tcp: {error}"));
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
        .unwrap_or_else(|error| panic!("name: {error}"));
    let tls = core_tls::TlsConnector::from(config)
        .connect(name, stream)
        .await
        .unwrap_or_else(|error| panic!("tls handshake: {error}"));
    let request = format!("wss://localhost:{}/turns/stream", port(&handle.bind))
        .into_client_request()
        .unwrap_or_else(|error| panic!("request: {error}"));
    let (_ws, response) = client_async(request, tls)
        .await
        .unwrap_or_else(|error| panic!("wss upgrade: {error}"));
    assert_eq!(response.status().as_u16(), 101);

    let plain =
        tokio_tungstenite::connect_async(format!("ws://{}/turns/stream", handle.bind)).await;
    assert!(
        plain.is_err(),
        "plain websocket must not be accepted on a TLS listener"
    );

    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}

fn port(bind: &str) -> &str {
    bind.rsplit_once(':').map(|(_, port)| port).unwrap_or("0")
}

#[test]
fn property_backend_refuses_plain_http_on_the_network() {
    let mut config = core_config::Config::default();
    config.property.facilitator_url = Some("http://127.0.0.1:8791/mcp".to_string());
    config.property.require_device_token = Some(false);
    assert!(server_options_from_config(&config, "0.0.0.0:8781").is_err());
    assert!(server_options_from_config(&config, "127.0.0.1:8781").is_ok());
    config.service.allow_plaintext_lan = true;
    assert!(server_options_from_config(&config, "0.0.0.0:8781").is_ok());

    let home = core_config::Config::default();
    assert!(
        server_options_from_config(&home, "0.0.0.0:8781").is_ok(),
        "home installs keep plain HTTP"
    );
}
