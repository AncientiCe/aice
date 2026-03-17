//! Integration tests: pod audio -> ingest event; device identity; invalid message resilience.

use futures_util::{SinkExt, StreamExt};
use pod_gateway::{run_gateway, PodEgressCommand, PodIngestEvent};
use pod_protocol::{AudioPayload, GatewayToPod, PodToGateway};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn pod_identify_then_audio_preserves_device_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut write, _read) = ws_stream.split();

    let identify = PodToGateway::Identify {
        device_id: "pod-abc-1".to_string(),
        room: Some("kitchen".to_string()),
    };
    write
        .send(Message::Text(serde_json::to_string(&identify).unwrap()))
        .await
        .unwrap();
    let audio = PodToGateway::Audio {
        payload: AudioPayload(vec![0, 0, 2, 0]),
    };
    write
        .send(Message::Text(serde_json::to_string(&audio).unwrap()))
        .await
        .unwrap();
    drop(write);

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(event.device_id, "pod-abc-1");
    assert_eq!(event.pcm, vec![0_i16, 2_i16]);
}

#[tokio::test]
async fn pod_invalid_json_skipped_connection_stays_up() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut write, _read) = ws_stream.split();

    write
        .send(Message::Text("not json".to_string()))
        .await
        .unwrap();
    let valid = PodToGateway::Audio {
        payload: AudioPayload(vec![3, 0, 4, 0]),
    };
    write
        .send(Message::Text(serde_json::to_string(&valid).unwrap()))
        .await
        .unwrap();
    drop(write);

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(event.pcm, vec![3_i16, 4_i16]);
}

#[tokio::test]
async fn pod_audio_frame_produces_ingest_event() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut write, _read) = ws_stream.split();

    let msg = PodToGateway::Audio {
        payload: AudioPayload(vec![0, 0, 1, 0]),
    };
    let text = serde_json::to_string(&msg).unwrap();
    write.send(Message::Text(text)).await.unwrap();
    drop(write);

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for ingest event")
        .expect("channel closed");

    assert_eq!(event.pcm.len(), 2);
    assert_eq!(event.pcm[0], 0_i16);
    assert_eq!(event.pcm[1], 1_i16);

    server.abort();
}

#[tokio::test]
async fn pod_ping_receives_pong() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, _rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut write, mut read) = ws_stream.split();

    let ping = PodToGateway::Ping { seq: 7 };
    write
        .send(Message::Text(serde_json::to_string(&ping).unwrap()))
        .await
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(2), read.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match response {
        Message::Text(s) => {
            let msg: GatewayToPod = serde_json::from_str(&s).unwrap();
            assert_eq!(msg, GatewayToPod::Pong { seq: 7 });
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[tokio::test]
async fn egress_to_device_writes_audio_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, _rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut write, mut read) = ws_stream.split();
    let identify = PodToGateway::Identify {
        device_id: "pod-xyz".to_string(),
        room: None,
    };
    write
        .send(Message::Text(serde_json::to_string(&identify).unwrap()))
        .await
        .unwrap();
    let _hello_ack = read.next().await.unwrap().unwrap();

    egress_tx
        .send(PodEgressCommand::ToDevice {
            device_id: "pod-xyz".to_string(),
            msg: GatewayToPod::Audio {
                payload: AudioPayload(vec![1, 2, 3, 4]),
            },
        })
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(2), read.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match response {
        Message::Text(s) => {
            let msg: GatewayToPod = serde_json::from_str(&s).unwrap();
            assert_eq!(
                msg,
                GatewayToPod::Audio {
                    payload: AudioPayload(vec![1, 2, 3, 4])
                }
            );
        }
        other => panic!("unexpected message: {other:?}"),
    }
}
