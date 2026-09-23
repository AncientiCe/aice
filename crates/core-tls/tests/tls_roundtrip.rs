use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use core_tls::{
    acceptor, client_config_trusting, ensure_ca, ensure_server_cert, fingerprint_sha256,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

fn temp_dir(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    std::env::temp_dir().join(format!("aice-tls-{label}-{}-{nanos}", std::process::id()))
}

/// Serve one TLS echo, then connect with a client that trusts `client_ca`.
async fn handshake(
    server_dir: &std::path::Path,
    client_ca: &std::path::Path,
) -> Result<String, String> {
    let (cert, key) = ensure_server_cert(
        server_dir,
        &["localhost".to_string(), "127.0.0.1".to_string()],
    )
    .map_err(|error| error.to_string())?;
    let acceptor = acceptor(&cert, &key).map_err(|error| error.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let server = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            if let Ok(mut tls) = acceptor.accept(stream).await {
                let _ = tls.write_all(b"hello over tls").await;
                let _ = tls.shutdown().await;
            }
        }
    });
    let config = client_config_trusting(client_ca).map_err(|error| error.to_string())?;
    let connector = TlsConnector::from(Arc::clone(&config));
    let stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
    let name = ServerName::try_from("localhost").map_err(|e| e.to_string())?;
    let mut tls = connector
        .connect(name, stream)
        .await
        .map_err(|error| error.to_string())?;
    let mut text = String::new();
    tls.read_to_string(&mut text)
        .await
        .map_err(|error| error.to_string())?;
    server.abort();
    Ok(text)
}

#[tokio::test]
async fn server_cert_signed_by_the_property_ca_is_trusted() {
    let dir = temp_dir("trusted");
    let ca = match ensure_ca(&dir) {
        Ok(ca) => ca,
        Err(error) => panic!("ca: {error}"),
    };
    assert_eq!(
        handshake(&dir, &ca.cert_path).await,
        Ok("hello over tls".to_string())
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_different_ca_is_not_trusted() {
    let server_dir = temp_dir("server");
    let other_dir = temp_dir("other");
    if let Err(error) = ensure_ca(&server_dir) {
        panic!("ca: {error}");
    }
    let other = match ensure_ca(&other_dir) {
        Ok(ca) => ca,
        Err(error) => panic!("ca: {error}"),
    };
    assert!(handshake(&server_dir, &other.cert_path).await.is_err());
    let _ = std::fs::remove_dir_all(server_dir);
    let _ = std::fs::remove_dir_all(other_dir);
}

#[test]
fn ca_is_created_once_and_its_fingerprint_is_stable() {
    let dir = temp_dir("stable");
    let first = match ensure_ca(&dir) {
        Ok(ca) => ca,
        Err(error) => panic!("ca: {error}"),
    };
    let second = match ensure_ca(&dir) {
        Ok(ca) => ca,
        Err(error) => panic!("ca: {error}"),
    };
    let a = fingerprint_sha256(&first.cert_path).unwrap_or_default();
    let b = fingerprint_sha256(&second.cert_path).unwrap_or_default();
    assert_eq!(a, b);
    assert_eq!(a.len(), 32 * 3 - 1, "colon-separated SHA-256: {a}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_key_files_are_reported() {
    let dir = temp_dir("missing");
    assert!(acceptor(&dir.join("nope.pem"), &dir.join("nope.key")).is_err());
}
