use aice_backend::discovery_broadcast::{spawn_udp_discovery_responder, FIND_PAYLOAD};
use std::net::UdpSocket;
use std::time::Duration;
use tokio::net::UdpSocket as TokioUdp;

#[tokio::test]
async fn udp_discovery_responder_replies_here_with_http_port() {
    let http_bind = "127.0.0.1:8781";
    let probe = UdpSocket::bind("127.0.0.1:0").expect("bind probe socket");
    let discovery_port = probe.local_addr().expect("local addr").port();
    drop(probe);

    let handle = spawn_udp_discovery_responder(http_bind, discovery_port)
        .await
        .expect("spawn discovery responder");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = TokioUdp::bind("127.0.0.1:0")
        .await
        .expect("bind async client");
    let target = format!("127.0.0.1:{discovery_port}");
    client
        .send_to(FIND_PAYLOAD, &target)
        .await
        .expect("send FIND");

    let mut buf = [0u8; 64];
    let recv = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf)).await;
    let (n, _) = recv.expect("overall timeout").expect("recv HERE");
    let msg = std::str::from_utf8(&buf[..n]).expect("utf8");
    assert!(
        msg.starts_with("HERE:8781"),
        "expected HERE:8781 prefix, got {msg:?}"
    );

    handle.abort();
    let _ = handle.await;
}
