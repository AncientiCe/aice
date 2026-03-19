//! Pod gateway server for M5Stack pod audio and control.

use pod_gateway::{run_gateway, PodEgressCommand, PodIngestEvent};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

async fn run_health_server(bind: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "health endpoint listening");
    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf).await;
            let response =
                b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\n\r\nok";
            let _ = stream.write_all(response).await;
            let _ = stream.shutdown().await;
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    core_observability::init_json_logging().ok();
    let config = core_config::Config::load(std::path::Path::new("config.json")).unwrap_or_default();
    if !config.service.health_bind.trim().is_empty() {
        let bind = config.service.health_bind.clone();
        tokio::spawn(async move {
            if let Err(e) = run_health_server(bind).await {
                tracing::warn!(error = %e, "health endpoint stopped");
            }
        });
    }
    let addr: SocketAddr = config.pod_bind.parse().unwrap_or_else(|error| {
        tracing::warn!(
            pod_bind = %config.pod_bind,
            %error,
            "invalid pod_bind, defaulting to 0.0.0.0:8765"
        );
        SocketAddr::from(([0, 0, 0, 0], 8765))
    });
    let listener = TcpListener::bind(addr).await?;
    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (egress_tx, egress_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            tracing::info!(device_id = %event.device_id, samples = event.pcm.len(), "pod ingest");
            let _ = egress_tx.send(PodEgressCommand::ToDevice {
                device_id: event.device_id,
                msg: pod_protocol::GatewayToPod::Led {
                    state: pod_protocol::LedState::Listening,
                },
            });
        }
    });
    run_gateway(listener, tx, egress_rx, None).await
}
