use aice_backend::discovery_broadcast::{
    resolve_discovery_udp_port, spawn_udp_discovery_responder, DEFAULT_DISCOVERY_UDP_PORT,
};
use aice_backend::{
    property_client_from_config, server_options_from_config, spawn_device_heartbeats,
    spawn_memory_retention, spawn_server_with_options, AiceBackendEngine, AudioIngressConfig,
    BackendEngine, WhisperAudioTranscriber, HEARTBEAT_INTERVAL,
};
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
/// How often the backend asks the facilitator for stay memories to delete.
const MEMORY_RETENTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

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

    let concrete_engine = AiceBackendEngine::from_config(&config).await?;
    let retention = match property_client_from_config(&config.property) {
        Some(Ok(client)) => Some(spawn_memory_retention(
            concrete_engine.palace_handle(),
            client,
            MEMORY_RETENTION_INTERVAL,
        )),
        Some(Err(error)) => {
            warn!(%error, "memory retention job not started");
            None
        }
        None => None,
    };
    let engine: Arc<dyn BackendEngine> = Arc::new(concrete_engine);
    let transcriber = Arc::new(WhisperAudioTranscriber::new(
        config.stt.whisper_model_path.clone(),
        config.stt.preload_model_on_startup,
    )?);
    let options = server_options_from_config(&config, &bind)?;
    let heartbeats = options.device_auth.as_ref().map(|auth| {
        info!("turn stream requires facilitator device tokens");
        spawn_device_heartbeats(Arc::clone(auth), HEARTBEAT_INTERVAL)
    });
    if options.tls.is_some() {
        info!("backend serves TLS only");
    }
    let handle = spawn_server_with_options(
        &bind,
        engine,
        transcriber,
        AudioIngressConfig::from_config(&config),
        options,
    )
    .await?;
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
    if let Some(retention) = retention {
        retention.abort();
    }
    if let Some(heartbeats) = heartbeats {
        heartbeats.abort();
    }
    udp_handle.abort();
    let _ = udp_handle.await;
    handle.shutdown().await;
    Ok(())
}
