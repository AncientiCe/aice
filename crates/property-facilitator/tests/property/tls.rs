use property_facilitator::{add_user, serve, Pack, PropertyClient, Role, Settings, TlsFiles};
use reqwest::StatusCode;

use crate::common::{hotels, temp_db, PASSWORD, TOKEN};

fn temp_dir(label: &str) -> std::path::PathBuf {
    temp_db(label).with_extension("d")
}

fn trusting_client(ca: &std::path::Path) -> reqwest::Client {
    let pem = std::fs::read(ca).unwrap_or_default();
    let cert = match reqwest::Certificate::from_pem(&pem) {
        Ok(cert) => cert,
        Err(error) => panic!("ca pem: {error}"),
    };
    match reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(cert)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => panic!("client: {error}"),
    }
}

fn tls_files(dir: &std::path::Path) -> TlsFiles {
    let (cert, key) = match core_tls::ensure_server_cert(
        dir,
        &["localhost".to_string(), "127.0.0.1".to_string()],
    ) {
        Ok(paths) => paths,
        Err(error) => panic!("server cert: {error}"),
    };
    TlsFiles {
        cert,
        key,
        ca: Some(dir.join("ca.pem")),
    }
}

#[tokio::test]
async fn desk_and_mcp_are_served_over_tls_with_the_property_ca() {
    let db = temp_db("tls");
    let dir = temp_dir("tls");
    let mut settings = hotels(db.clone(), None, Vec::new());
    let files = tls_files(&dir);
    let ca = files.ca.clone().unwrap_or_default();
    settings.tls = Some(files);
    let running = match serve(settings).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    assert!(
        running.url.starts_with("https://127.0.0.1:"),
        "{}",
        running.url
    );

    let client = match PropertyClient::with_ca_file(format!("{}/mcp", running.url), TOKEN, &ca) {
        Ok(client) => client,
        Err(error) => panic!("client: {error}"),
    };
    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(error) => panic!("tools over tls: {error}"),
    };
    assert!(tools.iter().any(|tool| tool == "request_extra_towels"));

    let untrusting = match reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .build()
    {
        Ok(client) => client,
        Err(error) => panic!("client: {error}"),
    };
    assert!(untrusting
        .get(format!("{}/healthz", running.url))
        .send()
        .await
        .is_err());

    let trusted = trusting_client(&ca);
    let served_ca = match trusted
        .get(format!("{}/api/tls/ca", running.url))
        .send()
        .await
    {
        Ok(response) => response.text().await.unwrap_or_default(),
        Err(error) => panic!("ca download: {error}"),
    };
    assert_eq!(served_ca, std::fs::read_to_string(&ca).unwrap_or_default());

    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let login = match trusted
        .post(format!("{}/login", running.url))
        .form(&[("username", "maria"), ("password", PASSWORD)])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("login: {error}"),
    };
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let cookie = login
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        cookie.contains("Secure"),
        "session cookie must be Secure over TLS: {cookie}"
    );
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_network_bind_without_tls_is_refused() {
    let db = temp_db("plain-lan");
    let mut settings = hotels(db.clone(), None, Vec::new());
    settings.bind = "0.0.0.0:0".to_string();
    settings.tls = None;
    assert!(serve(settings).await.is_err());
    let _ = std::fs::remove_file(db);
}

#[test]
fn auto_tls_creates_a_property_ca_beside_the_config() {
    let dir = temp_dir("auto");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("dir: {error}");
    }
    let config = dir.join("property.json");
    let body = serde_json::json!({
        "pack": "hotels",
        "bind": "0.0.0.0:8791",
        "tls": { "mode": "auto", "hostnames": ["desk.hotel.local"] }
    });
    if let Err(error) = std::fs::write(&config, body.to_string()) {
        panic!("write: {error}");
    }
    let settings = match Settings::load_or_default(Pack::Hotels, &config) {
        Ok(settings) => settings,
        Err(error) => panic!("load: {error}"),
    };
    let files = match settings.tls {
        Some(files) => files,
        None => panic!("auto mode must enable tls"),
    };
    assert!(files.cert.exists() && files.key.exists());
    let ca = files.ca.unwrap_or_default();
    assert!(ca.starts_with(dir.join("tls")));
    assert!(core_tls::fingerprint_sha256(&ca).is_ok());

    let plain =
        serde_json::json!({ "pack": "hotels", "bind": "0.0.0.0:8791", "tls": { "mode": "off" } });
    if let Err(error) = std::fs::write(&config, plain.to_string()) {
        panic!("write: {error}");
    }
    assert!(
        Settings::load_or_default(Pack::Hotels, &config).is_err(),
        "tls off on a network bind is refused"
    );
    let _ = std::fs::remove_dir_all(dir);
}
