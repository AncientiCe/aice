use aice_macos::run_macos_frontend;
use core_config::Config;
use core_observability::{
    init_json_logging, init_prometheus_exporter, register_metrics, ExporterInitState,
};
use std::path::Path;
use tracing::{info, warn};

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = init_json_logging();
    let config = Config::load(Path::new("config.json"))?;
    register_metrics();
    let metrics_bind = resolve_metrics_bind(
        config.service.metrics_enabled,
        &config.service.metrics_bind,
        std::env::var("AICE_MACOS_METRICS_BIND").ok(),
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

    run_macos_frontend(config).await
}

#[cfg(test)]
mod tests {
    use super::resolve_metrics_bind;

    #[test]
    fn resolve_metrics_bind_prefers_env_when_present() {
        let bind = resolve_metrics_bind(true, "127.0.0.1:9000", Some("127.0.0.1:9002".to_string()));
        assert_eq!(bind.as_deref(), Some("127.0.0.1:9002"));
    }

    #[test]
    fn resolve_metrics_bind_uses_config_when_env_absent() {
        let bind = resolve_metrics_bind(true, "127.0.0.1:9000", None);
        assert_eq!(bind.as_deref(), Some("127.0.0.1:9000"));
    }

    #[test]
    fn resolve_metrics_bind_none_when_disabled() {
        let bind =
            resolve_metrics_bind(false, "127.0.0.1:9000", Some("127.0.0.1:9002".to_string()));
        assert!(bind.is_none());
    }
}
