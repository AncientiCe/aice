use aice_backend::{spawn_server, AiceBackendEngine, BackendEngine};
use core_config::Config;
use core_observability::{
    init_json_logging, init_prometheus_exporter, record_backend_mdns_advertisement_duration,
    record_backend_mdns_advertisement_total, register_metrics, ExporterInitState,
};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

const DEFAULT_MDNS_SERVICE_TYPE: &str = "_aice-backend._tcp.local.";
const DEFAULT_MDNS_INSTANCE_NAME: &str = "aice-backend";
const DEFAULT_MDNS_HOSTNAME: &str = "aice-backend.local.";

struct MdnsRegistration {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration {
    fn register(bind: &str) -> Result<Self, String> {
        let service_type =
            resolve_mdns_service_type(std::env::var("AICE_BACKEND_MDNS_SERVICE_TYPE").ok());
        let instance_name =
            resolve_mdns_instance_name(std::env::var("AICE_BACKEND_MDNS_INSTANCE").ok());
        let hostname = build_mdns_hostname(
            &instance_name,
            std::env::var("AICE_BACKEND_MDNS_HOSTNAME").ok(),
        );
        let info = build_mdns_service_info(bind, &service_type, &instance_name, &hostname)?;
        let fullname = info.get_fullname().to_string();
        let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
        daemon.register(info).map_err(|error| error.to_string())?;
        Ok(Self { daemon, fullname })
    }

    fn shutdown(self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

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

fn resolve_mdns_service_type(env_value: Option<String>) -> String {
    env_value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MDNS_SERVICE_TYPE.to_string())
}

fn resolve_mdns_instance_name(env_value: Option<String>) -> String {
    env_value
        .map(|value| sanitize_dns_label(value.trim()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MDNS_INSTANCE_NAME.to_string())
}

fn sanitize_dns_label(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch == ' ' {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn build_mdns_hostname(instance_name: &str, env_value: Option<String>) -> String {
    if let Some(hostname) = env_value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        if hostname.ends_with(".local.") {
            return hostname;
        }
        return format!("{}.local.", hostname.trim_end_matches('.'));
    }
    let label = sanitize_dns_label(instance_name);
    if label.is_empty() {
        DEFAULT_MDNS_HOSTNAME.to_string()
    } else {
        format!("{label}.local.")
    }
}

fn parse_bind_for_mdns(bind: &str) -> Result<(u16, Option<IpAddr>), String> {
    let parsed = bind
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid bind address '{bind}': {error}"))?;
    let ip = parsed.ip();
    if ip.is_unspecified() {
        Ok((parsed.port(), None))
    } else {
        Ok((parsed.port(), Some(ip)))
    }
}

fn build_mdns_service_info(
    bind: &str,
    service_type: &str,
    instance_name: &str,
    hostname: &str,
) -> Result<ServiceInfo, String> {
    let (port, bind_ip) = parse_bind_for_mdns(bind)?;
    let info = if let Some(ip) = bind_ip {
        ServiceInfo::new(
            service_type,
            instance_name,
            hostname,
            ip,
            port,
            None::<std::collections::HashMap<String, String>>,
        )
    } else {
        ServiceInfo::new(
            service_type,
            instance_name,
            hostname,
            "",
            port,
            None::<std::collections::HashMap<String, String>>,
        )
        .map(|service| service.enable_addr_auto())
    };
    info.map_err(|error| error.to_string())
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

    let bind = std::env::var("AICE_BACKEND_BIND").unwrap_or_else(|_| "127.0.0.1:8781".to_string());
    let engine: Arc<dyn BackendEngine> = Arc::new(AiceBackendEngine::from_config(&config).await);
    let handle = spawn_server(&bind, engine).await?;
    info!(bind = %handle.bind, "aice-backend started");
    let mdns_started_at = Instant::now();
    let mdns_registration = match MdnsRegistration::register(&handle.bind) {
        Ok(registration) => {
            record_backend_mdns_advertisement_duration(mdns_started_at.elapsed());
            record_backend_mdns_advertisement_total("success");
            info!(bind = %handle.bind, "backend advertised via mDNS");
            registration
        }
        Err(error) => {
            record_backend_mdns_advertisement_duration(mdns_started_at.elapsed());
            record_backend_mdns_advertisement_total("error");
            return Err(format!("failed to advertise backend via mDNS: {error}").into());
        }
    };

    tokio::signal::ctrl_c().await?;
    mdns_registration.shutdown();
    handle.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_mdns_hostname, build_mdns_service_info, parse_bind_for_mdns,
        resolve_mdns_instance_name, resolve_mdns_service_type, resolve_metrics_bind,
        DEFAULT_MDNS_SERVICE_TYPE,
    };
    use std::net::IpAddr;

    #[test]
    fn resolve_metrics_bind_prefers_env_when_present() {
        let bind = resolve_metrics_bind(true, "127.0.0.1:9000", Some("127.0.0.1:9001".to_string()));
        assert_eq!(bind.as_deref(), Some("127.0.0.1:9001"));
    }

    #[test]
    fn resolve_metrics_bind_uses_config_when_env_absent() {
        let bind = resolve_metrics_bind(true, "127.0.0.1:9000", None);
        assert_eq!(bind.as_deref(), Some("127.0.0.1:9000"));
    }

    #[test]
    fn resolve_metrics_bind_none_when_disabled() {
        let bind =
            resolve_metrics_bind(false, "127.0.0.1:9000", Some("127.0.0.1:9001".to_string()));
        assert!(bind.is_none());
    }

    #[test]
    fn resolve_mdns_service_type_uses_default_when_env_missing() {
        let service_type = resolve_mdns_service_type(None);
        assert_eq!(service_type, DEFAULT_MDNS_SERVICE_TYPE);
    }

    #[test]
    fn resolve_mdns_service_type_trims_env_value() {
        let service_type = resolve_mdns_service_type(Some("  _custom._tcp.local.  ".to_string()));
        assert_eq!(service_type, "_custom._tcp.local.");
    }

    #[test]
    fn resolve_mdns_instance_name_defaults_when_env_missing() {
        let instance = resolve_mdns_instance_name(None);
        assert_eq!(instance, "aice-backend");
    }

    #[test]
    fn build_mdns_hostname_appends_local_suffix() {
        let host = build_mdns_hostname("aice-backend", None);
        assert_eq!(host, "aice-backend.local.");
    }

    #[test]
    fn parse_bind_for_mdns_returns_port_and_none_for_unspecified() {
        let parsed = match parse_bind_for_mdns("0.0.0.0:8781") {
            Ok(value) => value,
            Err(error) => panic!("bind should parse: {error}"),
        };
        assert_eq!(parsed.0, 8781);
        assert!(parsed.1.is_none());
    }

    #[test]
    fn parse_bind_for_mdns_returns_port_and_ip_for_specific_bind() {
        let parsed = match parse_bind_for_mdns("192.168.1.11:8781") {
            Ok(value) => value,
            Err(error) => panic!("bind should parse: {error}"),
        };
        assert_eq!(parsed.0, 8781);
        assert_eq!(
            parsed.1,
            Some(IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 11)))
        );
    }

    #[test]
    fn build_mdns_service_info_enables_auto_addr_for_unspecified_bind() {
        let info = match build_mdns_service_info(
            "0.0.0.0:8781",
            "_aice-backend._tcp.local.",
            "aice-backend",
            "aice-backend.local.",
        ) {
            Ok(value) => value,
            Err(error) => panic!("service info should build: {error}"),
        };
        assert_eq!(info.get_type(), "_aice-backend._tcp.local.");
        assert_eq!(info.get_port(), 8781);
        assert!(info.is_addr_auto());
    }
}
