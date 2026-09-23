use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

const PASSWORD: &str = "smoke test password";

struct StopChild(Child);

impl Drop for StopChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) => panic!("failed to reserve a port: {error}"),
    };
    match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(error) => panic!("failed to read reserved port: {error}"),
    }
}

struct Pack {
    _child: StopChild,
    base: String,
    dir: PathBuf,
    config: PathBuf,
    stderr: PathBuf,
}

fn spawn_pack(bin: &str, pack: &str, extensions: serde_json::Value, delegated: &[&str]) -> Pack {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let mut dir = std::env::temp_dir();
    dir.push(format!("aice-smoke-{pack}-{}-{millis}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("failed to create smoke dir: {error}");
    }
    let port = free_port();
    let config = dir.join("property.json");
    let body = json!({
        "pack": pack,
        "bind": format!("127.0.0.1:{port}"),
        "database_path": dir.join("property.sqlite"),
        "service_token_file": "service.token",
        "delegated_tools": delegated,
        "extensions": extensions
    });
    if let Err(error) = std::fs::write(&config, body.to_string()) {
        panic!("failed to write property config: {error}");
    }
    let stderr = dir.join("stderr.log");
    let log = match File::create(&stderr) {
        Ok(file) => file,
        Err(error) => panic!("failed to create stderr log: {error}"),
    };
    let child = match Command::new(bin)
        .arg(&config)
        .env_remove("AICE_PROPERTY_SERVICE_TOKEN")
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => panic!("failed to start {bin}: {error}"),
    };
    Pack {
        _child: StopChild(child),
        base: format!("http://127.0.0.1:{port}"),
        dir,
        config,
        stderr,
    }
}

fn client() -> reqwest::Client {
    match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => panic!("failed to build http client: {error}"),
    }
}

/// Wait for the desk, then read the service token the binary generated.
async fn wait_for_token(pack: &Pack) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if client()
            .get(format!("{}/healthz", pack.base))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        if Instant::now() >= deadline {
            let stderr = std::fs::read_to_string(&pack.stderr).unwrap_or_default();
            panic!("desk did not come up at {}. stderr: {stderr}", pack.base);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    match std::fs::read_to_string(pack.dir.join("service.token")) {
        Ok(token) => token.trim().to_string(),
        Err(error) => panic!("binary did not write service.token: {error}"),
    }
}

fn add_supervisor(bin: &str, config: &Path) {
    let status = match Command::new(bin)
        .arg(config)
        .args(["user", "add", "lee", "supervisor"])
        .env("AICE_PROPERTY_PASSWORD", PASSWORD)
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) => status,
        Err(error) => panic!("user add failed to run: {error}"),
    };
    assert!(status.success(), "user add exited with {status}");
}

async fn call_tool(
    base: &str,
    token: Option<&str>,
    tool: &str,
    room: &str,
) -> (reqwest::StatusCode, serde_json::Value) {
    let mut request = client().post(format!("{base}/mcp")).json(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": { "room": room } }
    }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    match request.send().await {
        Ok(response) => {
            let status = response.status();
            (status, response.json().await.unwrap_or_default())
        }
        Err(error) => panic!("tools/call failed: {error}"),
    }
}

async fn desk_html(base: &str) -> String {
    let login = match client()
        .post(format!("{base}/login"))
        .form(&[("username", "lee"), ("password", PASSWORD)])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("login failed: {error}"),
    };
    assert_eq!(login.status(), reqwest::StatusCode::SEE_OTHER);
    let cookie = login
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .to_string();
    match client()
        .get(format!("{base}/desk"))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
    {
        Ok(response) => response.text().await.unwrap_or_default(),
        Err(error) => panic!("desk get failed: {error}"),
    }
}

#[tokio::test]
async fn hotels_binary_opens_a_desk_ticket() {
    let bin = match std::env::var("CARGO_BIN_EXE_aice-hotels") {
        Ok(path) => path,
        Err(error) => panic!("missing aice-hotels binary: {error}"),
    };
    let pack = spawn_pack(&bin, "hotels", json!({ "101": "204" }), &[]);
    let token = wait_for_token(&pack).await;
    let (status, _) = call_tool(&pack.base, None, "request_extra_towels", "204").await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    let (status, created) =
        call_tool(&pack.base, Some(&token), "request_extra_towels", "204").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(created["result"]["structuredContent"]["status"], "open");
    add_supervisor(&bin, &pack.config);
    let desk = desk_html(&pack.base).await;
    assert!(desk.contains("aice-hotels desk"));
    assert!(desk.contains("204"));
    assert!(desk.contains("request_extra_towels"));
}
