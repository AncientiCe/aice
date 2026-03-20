use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExporterInitState {
    Started,
    AlreadyRunning,
}

#[derive(Debug)]
pub enum ExporterInitError {
    InvalidBind { bind: String, message: String },
    LockPoisoned,
    InstallFailed(String),
}

impl std::fmt::Display for ExporterInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBind { bind, message } => {
                write!(f, "invalid metrics bind '{bind}': {message}")
            }
            Self::LockPoisoned => write!(f, "metrics exporter lock poisoned"),
            Self::InstallFailed(message) => {
                write!(f, "failed to install prometheus exporter: {message}")
            }
        }
    }
}

impl std::error::Error for ExporterInitError {}

#[derive(Debug, Default)]
struct ExporterState {
    started_bind: Option<String>,
}

fn state() -> &'static Mutex<ExporterState> {
    static EXPORTER_STATE: OnceLock<Mutex<ExporterState>> = OnceLock::new();
    EXPORTER_STATE.get_or_init(|| Mutex::new(ExporterState::default()))
}

/// Initialize the Prometheus HTTP exporter once for the process.
///
/// Subsequent calls are no-ops and return `AlreadyRunning`.
pub fn init_prometheus_exporter(bind: &str) -> Result<ExporterInitState, ExporterInitError> {
    let state = state();
    let mut guard = match state.lock() {
        Ok(value) => value,
        Err(_) => return Err(ExporterInitError::LockPoisoned),
    };
    if guard.started_bind.is_some() {
        return Ok(ExporterInitState::AlreadyRunning);
    }

    let addr: SocketAddr = match bind.parse() {
        Ok(value) => value,
        Err(error) => {
            return Err(ExporterInitError::InvalidBind {
                bind: bind.to_string(),
                message: error.to_string(),
            });
        }
    };
    match PrometheusBuilder::new().with_http_listener(addr).install() {
        Ok(()) => {
            guard.started_bind = Some(bind.to_string());
            Ok(ExporterInitState::Started)
        }
        Err(error) => Err(ExporterInitError::InstallFailed(error.to_string())),
    }
}
