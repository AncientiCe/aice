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

fn temp_paths(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "aice-smoke-{label}-{}-{millis}",
        std::process::id()
    ));
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("failed to create smoke dir: {error}");
    }
    (
        dir.join("property.json"),
        dir.join("property.sqlite"),
        dir.join("stderr.log"),
    )
}

fn spawn_pack(bin: &str, pack: &str, port: u16) -> (StopChild, String, PathBuf) {
    let (config_path, database_path, stderr_path) = temp_paths(pack);
    let config = json!({
        "pack": pack,
        "bind": format!("127.0.0.1:{port}"),
        "database_path": database_path,
        "delegated_tools": [],
        "extensions": { "101": "204" }
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

async fn call_tool(base: &str, tool: &str, room: &str) -> serde_json::Value {
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(error) => panic!("failed to build http client: {error}"),
    };
    let response = match client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": { "room": room }
            }
        }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("tools/call failed: {error}"),
    };
    match response.json().await {
        Ok(value) => value,
        Err(error) => panic!("tools/call json failed: {error}"),
    }
}

#[tokio::test]
async fn hotels_binary_opens_a_desk_ticket() {
    let bin = match std::env::var("CARGO_BIN_EXE_aice-hotels") {
        Ok(path) => path,
        Err(error) => panic!("missing aice-hotels binary: {error}"),
    };
    let port = free_port();
    let (child, base, stderr_path) = spawn_pack(&bin, "hotels", port);
    wait_for_desk(&base, &stderr_path).await;
    let created = call_tool(&base, "request_extra_towels", "204").await;
    assert_eq!(created["result"]["structuredContent"]["status"], "open");
    let desk = match reqwest::get(format!("{base}/desk")).await {
        Ok(response) => match response.text().await {
            Ok(text) => text,
            Err(error) => panic!("desk body failed: {error}"),
        },
        Err(error) => panic!("desk get failed: {error}"),
    };
    assert!(desk.contains("aice-hotels desk"));
    assert!(desk.contains("204"));
    assert!(desk.contains("request_extra_towels"));
    drop(child);
}
