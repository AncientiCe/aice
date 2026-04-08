use aice_backend::discovery_broadcast::{
    resolve_discovery_udp_port, spawn_udp_discovery_responder, DEFAULT_DISCOVERY_UDP_PORT,
};
use aice_backend::{spawn_server, AiceBackendEngine, BackendEngine};
use core_config::Config;
use core_observability::{
    init_json_logging, init_prometheus_exporter, record_backend_udp_discovery_listen_duration,
    record_backend_udp_discovery_listen_total, register_metrics, ExporterInitState,
};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

const DEFAULT_BACKEND_BIND: &str = "0.0.0.0:8781";

fn resolve_metrics_bind(
    metrics_enabled: bool,
    config_bind: &str,
    env_bind: Option<String>,
) -> Option<String> {
    if !metrics_enabled {
        return None;
    }
    Some(env_bind.unwrap_or_else(|| config_bind.to_string()))
}

fn resolve_backend_bind(env_bind: Option<String>) -> Result<String, String> {
    let bind = env_bind.unwrap_or_else(|| DEFAULT_BACKEND_BIND.to_string());
    bind.parse::<SocketAddr>()
        .map_err(|error| format!("invalid backend bind address '{bind}': {error}"))?;
    Ok(bind)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = init_json_logging();
    let config = Config::load(Path::new("config.json"))?;
    register_metrics();
    let metrics_bind = resolve_metrics_bind(
        config.service.metrics_enabled,
        &config.service.metrics_bind,
        std::env::var("AICE_BACKEND_METRICS_BIND").ok(),
    );
    if let Some(bind) = metrics_bind {
        match init_prometheus_exporter(&bind) {
            Ok(ExporterInitState::Started) | Ok(ExporterInitState::AlreadyRunning) => {}
            Err(error) => {
                warn!(%error, bind = %bind, "failed to initialize metrics exporter");
            }
        }
    } else {
        info!("metrics exporter disabled by config");
    }

    let bind = resolve_backend_bind(std::env::var("AICE_BACKEND_BIND").ok())
        .map_err(|error| format!("failed to resolve backend bind: {error}"))?;
    let discovery_port =
        resolve_discovery_udp_port(std::env::var("AICE_BACKEND_DISCOVERY_UDP_PORT").ok())
            .map_err(|error| format!("failed to resolve discovery UDP port: {error}"))?;

    let engine: Arc<dyn BackendEngine> = Arc::new(AiceBackendEngine::from_config(&config).await?);
    let handle = spawn_server(&bind, engine).await?;
    info!(bind = %handle.bind, "aice-backend started");

    let disc_started = Instant::now();
    let udp_handle = match spawn_udp_discovery_responder(&handle.bind, discovery_port).await {
        Ok(h) => {
            record_backend_udp_discovery_listen_duration(disc_started.elapsed());
            record_backend_udp_discovery_listen_total("success");
            info!(
                discovery_port,
                default_port = DEFAULT_DISCOVERY_UDP_PORT,
                "backend listening for UDP broadcast discovery (FIND -> HERE:<http_port>)"
            );
            h
        }
        Err(error) => {
            record_backend_udp_discovery_listen_duration(disc_started.elapsed());
            record_backend_udp_discovery_listen_total("error");
            return Err(format!("failed to start UDP discovery responder: {error}").into());
        }
    };

    tokio::signal::ctrl_c().await?;
    udp_handle.abort();
    let _ = udp_handle.await;
    handle.shutdown().await;
    Ok(())
}
