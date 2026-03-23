//! LAN discovery via UDP broadcast: frontend sends `FIND`, backend replies with `HERE:<http_port>`.
//! No mDNS/multicast; works on macOS, Linux, and Windows on the same broadcast domain.

use core_observability::{
    record_backend_udp_discovery_request_total, record_backend_udp_discovery_response_total,
};
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tracing::warn;

/// Request payload sent by the frontend (4 bytes).
pub const FIND_PAYLOAD: &[u8] = b"FIND";

/// Default UDP port for discovery (env: `AICE_BACKEND_DISCOVERY_UDP_PORT`).
pub const DEFAULT_DISCOVERY_UDP_PORT: u16 = 9999;

/// Max datagram size we accept for a discovery request.
const MAX_DISCOVERY_DATAGRAM: usize = 64;

#[must_use]
pub fn discovery_response_bytes(http_port: u16) -> Vec<u8> {
    format!("HERE:{http_port}").into_bytes()
}

#[must_use]
pub fn is_discovery_find_request(packet: &[u8]) -> bool {
    packet.len() >= FIND_PAYLOAD.len() && &packet[..FIND_PAYLOAD.len()] == FIND_PAYLOAD
}

pub fn parse_http_port_from_bind(bind: &str) -> Result<u16, String> {
    bind.parse::<SocketAddr>()
        .map(|addr| addr.port())
        .map_err(|error| format!("invalid bind address '{bind}': {error}"))
}

pub fn resolve_discovery_udp_port(env_value: Option<String>) -> Result<u16, String> {
    match env_value {
        None => Ok(DEFAULT_DISCOVERY_UDP_PORT),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(DEFAULT_DISCOVERY_UDP_PORT);
            }
            trimmed
                .parse::<u16>()
                .map_err(|error| format!("invalid AICE_BACKEND_DISCOVERY_UDP_PORT: {error}"))
        }
    }
}

/// Binds `0.0.0.0:discovery_port` and responds to `FIND` with `HERE:<http_port>` (from `http_bind`).
/// Run the returned join handle until shutdown, then [`abort`](tokio::task::JoinHandle::abort) it.
pub async fn spawn_udp_discovery_responder(
    http_bind: &str,
    discovery_port: u16,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let http_port = parse_http_port_from_bind(http_bind)?;
    let response = discovery_response_bytes(http_port);
    let socket = UdpSocket::bind(("0.0.0.0", discovery_port))
        .await
        .map_err(|error| format!("UDP discovery bind on port {discovery_port} failed: {error}"))?;

    Ok(tokio::spawn(async move {
        let mut buf = [0u8; MAX_DISCOVERY_DATAGRAM];
        loop {
            let (len, addr) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(error) => {
                    warn!(%error, "udp discovery recv_from failed");
                    continue;
                }
            };
            let packet = &buf[..len];
            if !is_discovery_find_request(packet) {
                continue;
            }
            record_backend_udp_discovery_request_total();
            match socket.send_to(&response, addr).await {
                Ok(_) => record_backend_udp_discovery_response_total("success"),
                Err(error) => {
                    warn!(%error, %addr, "udp discovery send_to failed");
                    record_backend_udp_discovery_response_total("send_error");
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_discovery_find_request_accepts_find_prefix() {
        assert!(is_discovery_find_request(b"FIND"));
        assert!(is_discovery_find_request(b"FIND\x00"));
    }

    #[test]
    fn is_discovery_find_request_rejects_other_payloads() {
        assert!(!is_discovery_find_request(b"FOO"));
        assert!(!is_discovery_find_request(b"FI"));
    }

    #[test]
    fn discovery_response_bytes_format() {
        assert_eq!(discovery_response_bytes(8781), b"HERE:8781");
    }

    #[test]
    fn resolve_discovery_udp_port_defaults() {
        assert_eq!(resolve_discovery_udp_port(None).unwrap(), 9999);
        assert_eq!(
            resolve_discovery_udp_port(Some("".to_string())).unwrap(),
            9999
        );
        assert_eq!(
            resolve_discovery_udp_port(Some("  ".to_string())).unwrap(),
            9999
        );
    }

    #[test]
    fn resolve_discovery_udp_port_parses_number() {
        assert_eq!(
            resolve_discovery_udp_port(Some("12345".to_string())).unwrap(),
            12345
        );
    }

    #[test]
    fn parse_http_port_from_bind_works() {
        assert_eq!(parse_http_port_from_bind("0.0.0.0:8781").unwrap(), 8781);
    }
}
