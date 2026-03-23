use aice_backend::discovery_broadcast::{spawn_udp_discovery_responder, FIND_PAYLOAD};
use std::net::UdpSocket;
use std::time::Duration;
use tokio::net::UdpSocket as TokioUdp;

#[tokio::test]
async fn udp_discovery_responder_replies_here_with_http_port() {
    let http_bind = "127.0.0.1:8781";
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind probe socket: {e}"));
    let discovery_port = probe
        .local_addr()
        .unwrap_or_else(|e| panic!("local addr: {e}"))
        .port();
    drop(probe);

    let handle = spawn_udp_discovery_responder(http_bind, discovery_port)
        .await
        .unwrap_or_else(|e| panic!("spawn discovery responder: {e}"));

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = TokioUdp::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind async client: {e}"));
    let target = format!("127.0.0.1:{discovery_port}");
    client
        .send_to(FIND_PAYLOAD, &target)
        .await
        .unwrap_or_else(|e| panic!("send FIND: {e}"));

    let mut buf = [0u8; 64];
    let recv = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf)).await;
    let io_result = match recv {
        Ok(v) => v,
        Err(_) => panic!("overall timeout"),
    };
    let (n, _) = match io_result {
        Ok(pair) => pair,
        Err(e) => panic!("recv HERE: {e}"),
    };
    let msg = match std::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(e) => panic!("utf8: {e}"),
    };
    assert!(
        msg.starts_with("HERE:8781"),
        "expected HERE:8781 prefix, got {msg:?}"
    );

    handle.abort();
    let _ = handle.await;
}
