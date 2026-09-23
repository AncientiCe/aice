//! Room bridge: pods on `pod_bind`, turns on the backend, answers spoken back.

use std::path::Path;
use std::sync::Arc;

use core_config::Config;
use pod_gateway::{spawn_bridge, BridgeSettings, PiperSpeech, VadSettings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    core_observability::init_json_logging().ok();
    core_observability::register_metrics();
    let config = Config::load(Path::new("config.json"))?;
    let gateway = &config.pod_gateway;
    if !config.service.metrics_bind.trim().is_empty() && config.service.metrics_enabled {
        if let Err(error) = core_observability::init_prometheus_exporter(&gateway.metrics_bind) {
            tracing::warn!(%error, "pod bridge metrics exporter failed to start");
        }
    }
    let pod_tls = match &gateway.tls {
        Some(files) => Some(core_tls::acceptor(
            Path::new(&files.cert_file),
            Path::new(&files.key_file),
        )?),
        None => None,
    };
    if pod_tls.is_none()
        && !is_loopback(&config.pod_bind)
        && config.property.is_configured()
        && !config.service.allow_plaintext_lan
    {
        return Err(format!(
            "refusing plain WebSocket for pods on {}; set pod_gateway.tls or service.allow_plaintext_lan",
            config.pod_bind
        )
        .into());
    }
    let backend_tls = match gateway.backend_ca_file.as_deref() {
        Some(ca) if !ca.trim().is_empty() => Some(core_tls::client_config_trusting(Path::new(ca))?),
        _ => None,
    };
    let speech = PiperSpeech(core_tts::PiperSynth::new(Path::new(
        &config.tts.piper_model_path,
    ))?);
    let handle = spawn_bridge(
        &config.pod_bind,
        BridgeSettings {
            backend_url: gateway.backend_url.clone(),
            backend_tls,
            pod_tls,
            vad: VadSettings {
                start_level: gateway.vad_start_level,
                end_silence: std::time::Duration::from_millis(gateway.vad_end_silence_ms),
                ..VadSettings::default()
            },
        },
        Arc::new(speech),
    )
    .await?;
    tracing::info!(bind = %handle.bind, backend = %gateway.backend_url, "pod bridge started");
    handle.until_ctrl_c().await
}

fn is_loopback(bind: &str) -> bool {
    bind.parse::<std::net::SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false)
}
