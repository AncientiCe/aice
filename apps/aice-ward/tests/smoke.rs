use std::fs::File;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

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

fn spawn_pack(bin: &str, pack: &str, port: u16) -> (StopChild, String, PathBuf) {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let mut dir = std::env::temp_dir();
    dir.push(format!("aice-smoke-{pack}-{}-{millis}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("failed to create smoke dir: {error}");
    }
    let config_path = dir.join("property.json");
    let database_path = dir.join("property.sqlite");
    let stderr_path = dir.join("stderr.log");
    let config = json!({
        "pack": pack,
        "bind": format!("127.0.0.1:{port}"),
        "database_path": database_path,
        "delegated_tools": [],
        "extensions": {}
    });
    let body = match serde_json::to_string(&config) {
        Ok(body) => body,
        Err(error) => panic!("failed to encode property config: {error}"),
    };
    if let Err(error) = std::fs::write(&config_path, body) {
        panic!("failed to write property config: {error}");
    }
    let stderr = match File::create(&stderr_path) {
        Ok(file) => file,
        Err(error) => panic!("failed to create stderr log: {error}"),
    };
    let child = match Command::new(bin)
        .arg(&config_path)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => panic!("failed to start {bin}: {error}"),
    };
    (
        StopChild(child),
        format!("http://127.0.0.1:{port}"),
        stderr_path,
    )
}

async fn wait_for_desk(base: &str, stderr_path: &PathBuf) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
    {
        Ok(client) => client,
        Err(error) => panic!("failed to build http client: {error}"),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if client.get(format!("{base}/desk")).send().await.is_ok() {
            return;
        }
        if Instant::now() >= deadline {
            let stderr = std::fs::read_to_string(stderr_path).unwrap_or_default();
            panic!("desk did not come up at {base}. stderr: {stderr}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn ward_binary_allows_wayfinding_and_denies_medication() {
    let bin = match std::env::var("CARGO_BIN_EXE_aice-ward") {
        Ok(path) => path,
        Err(error) => panic!("missing aice-ward binary: {error}"),
    };
    let port = free_port();
    let (child, base, stderr_path) = spawn_pack(&bin, "ward", port);
    wait_for_desk(&base, &stderr_path).await;
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(error) => panic!("failed to build http client: {error}"),
    };
    let allowed: serde_json::Value = match client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "wayfinding",
                "arguments": { "room": "4B" }
            }
        }))
        .send()
        .await
    {
        Ok(response) => match response.json().await {
            Ok(value) => value,
            Err(error) => panic!("tools/call json failed: {error}"),
        },
        Err(error) => panic!("tools/call failed: {error}"),
    };
    assert_eq!(allowed["result"]["isError"], false);
    assert_eq!(allowed["result"]["structuredContent"]["status"], "open");

    let denied: serde_json::Value = match client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "prescribe_medication",
                "arguments": { "room": "4B" }
            }
        }))
        .send()
        .await
    {
        Ok(response) => match response.json().await {
            Ok(value) => value,
            Err(error) => panic!("tools/call json failed: {error}"),
        },
        Err(error) => panic!("tools/call failed: {error}"),
    };
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(denied["result"]["structuredContent"]["status"], "escalated");
    drop(child);
}
