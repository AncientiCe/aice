use core_observability::{
    init_prometheus_exporter, record_backend_http_request, register_metrics, ExporterInitState,
};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn shared_bind() -> Result<&'static str, std::io::Error> {
    static BIND: OnceLock<String> = OnceLock::new();
    if let Some(bind) = BIND.get() {
        return Ok(bind.as_str());
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener);
    let _ = BIND.set(addr.to_string());
    match BIND.get() {
        Some(bind) => Ok(bind.as_str()),
        None => Err(std::io::Error::other("failed to initialize test bind")),
    }
}

fn scrape(bind: &str) -> Result<String, std::io::Error> {
    let mut stream = TcpStream::connect(bind)?;
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn wait_for_metrics(bind: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_payload = String::new();
    loop {
        if let Ok(payload) = scrape(bind) {
            last_payload = payload.clone();
            if payload.contains(needle) {
                return payload;
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "metrics payload did not contain '{needle}' before timeout; last payload:\n{last_payload}"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn observability_json_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../ops/observability/grafana/provisioning/dashboards/json")
}

#[test]
fn backend_http_metrics_cover_core_routes_and_errors() {
    let metrics_bind = shared_bind().unwrap_or_else(|error| panic!("reserve bind failed: {error}"));
    let state = init_prometheus_exporter(metrics_bind)
        .unwrap_or_else(|error| panic!("failed to initialize exporter: {error}"));
    assert_eq!(
        state,
        ExporterInitState::Started,
        "exporter should start in this isolated test process"
    );
    register_metrics();

    record_backend_http_request("/healthz", "GET", 200, Duration::from_millis(3));
    record_backend_http_request("/v1/turns/chunks", "POST", 202, Duration::from_millis(5));
    record_backend_http_request(
        "/v1/frontends/activate",
        "POST",
        400,
        Duration::from_millis(4),
    );
    record_backend_http_request("/v1/turns", "POST", 200, Duration::from_millis(7));
    record_backend_http_request(
        "/v1/turns/:turn_id/frontend-skill-result",
        "POST",
        200,
        Duration::from_millis(6),
    );
    record_backend_http_request("not_found", "GET", 404, Duration::from_millis(2));

    let payload = wait_for_metrics(metrics_bind, "backend_http_request_duration_seconds");
    assert!(payload.contains("backend_http_request_duration_seconds"));
    assert!(payload.contains("route=\"/healthz\""));
    assert!(payload.contains("route=\"/v1/turns\""));
    assert!(payload.contains("route=\"/v1/turns/chunks\""));
    assert!(payload.contains("route=\"/v1/frontends/activate\""));
    assert!(payload.contains("route=\"/v1/turns/:turn_id/frontend-skill-result\""));
    assert!(payload.contains("route=\"not_found\""));
    assert!(payload.contains("status_class=\"4xx\""));
    assert!(payload.contains("status_class=\"2xx\""));
}

#[test]
fn dashboard_pack_is_backend_only() {
    let dir = observability_json_dir();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("failed to read dashboard dir {dir:?}: {error}"));

    let mut uids = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read entry: {error}"));
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read dashboard file {path:?}: {error}"));
        let value: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|error| panic!("invalid dashboard JSON in {path:?}: {error}"));
        if let Some(uid) = value.get("uid").and_then(|v| v.as_str()) {
            uids.push(uid.to_string());
        }
    }

    assert!(
        !dir.join("frontend-timings.json").exists(),
        "frontend dashboard file must be removed"
    );
    assert!(
        !uids.iter().any(|uid| uid == "aice-frontend-timings"),
        "frontend dashboard UID must be removed"
    );
}
