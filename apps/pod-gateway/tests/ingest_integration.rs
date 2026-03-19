//! Integration tests: pod audio -> ingest event; device identity; invalid message resilience.

use futures_util::{SinkExt, StreamExt};
use pod_gateway::{run_gateway, PodEgressCommand, PodIngestEvent};
use pod_protocol::{AudioPayload, GatewayToPod, PodToGateway};
use std::error::Error;
use std::io;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

fn io_err(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

async fn recv_with_timeout<T>(
    fut: impl std::future::Future<Output = T>,
    timeout_message: &str,
) -> Result<T, Box<dyn Error + Send + Sync>> {
    tokio::time::timeout(Duration::from_secs(2), fut)
        .await
        .map_err(|_| io_err(timeout_message)) // timeout
        .map_err(Into::into)
}

#[tokio::test]
async fn pod_identify_then_audio_preserves_device_id() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, _read) = ws_stream.split();

    let identify = PodToGateway::Identify {
        device_id: "pod-abc-1".to_string(),
        room: Some("kitchen".to_string()),
    };
    write
        .send(Message::Text(serde_json::to_string(&identify)?))
        .await?;
    let audio = PodToGateway::Audio {
        payload: AudioPayload(vec![0, 0, 2, 0]),
    };
    write
        .send(Message::Text(serde_json::to_string(&audio)?))
        .await?;
    drop(write);

    let event = recv_with_timeout(rx.recv(), "timeout waiting for ingest event").await?;
    let event = event.ok_or_else(|| io_err("channel closed"))?;
    assert_eq!(event.device_id, "pod-abc-1");
    assert_eq!(event.pcm, vec![0_i16, 2_i16]);
    Ok(())
}

#[tokio::test]
async fn pod_invalid_json_skipped_connection_stays_up() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, _read) = ws_stream.split();

    write.send(Message::Text("not json".to_string())).await?;
    let valid = PodToGateway::Audio {
        payload: AudioPayload(vec![3, 0, 4, 0]),
    };
    write
        .send(Message::Text(serde_json::to_string(&valid)?))
        .await?;
    drop(write);

    let event = recv_with_timeout(rx.recv(), "timeout waiting for ingest event").await?;
    let event = event.ok_or_else(|| io_err("channel closed"))?;
    assert_eq!(event.pcm, vec![3_i16, 4_i16]);
    Ok(())
}

#[tokio::test]
async fn pod_audio_frame_produces_ingest_event() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, _read) = ws_stream.split();

    let msg = PodToGateway::Audio {
        payload: AudioPayload(vec![0, 0, 1, 0]),
    };
    let text = serde_json::to_string(&msg)?;
    write.send(Message::Text(text)).await?;
    drop(write);

    let event = recv_with_timeout(rx.recv(), "timeout waiting for ingest event").await?;
    let event = event.ok_or_else(|| io_err("channel closed"))?;

    assert_eq!(event.pcm.len(), 2);
    assert_eq!(event.pcm[0], 0_i16);
    assert_eq!(event.pcm[1], 1_i16);

    server.abort();
    Ok(())
}

#[tokio::test]
async fn pod_ping_receives_pong() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, _rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (_egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    let ping = PodToGateway::Ping { seq: 7 };
    write
        .send(Message::Text(serde_json::to_string(&ping)?))
        .await?;

    let response = recv_with_timeout(read.next(), "timeout waiting for pong").await?;
    let response = response.ok_or_else(|| io_err("connection closed"))??;
    match response {
        Message::Text(s) => {
            let msg: GatewayToPod = serde_json::from_str(&s)?;
            assert_eq!(msg, GatewayToPod::Pong { seq: 7 });
        }
        other => panic!("unexpected message: {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn egress_to_device_writes_audio_frame() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, _rx) = mpsc::unbounded_channel::<PodIngestEvent>();
    let (egress_tx, egress_rx) = mpsc::unbounded_channel();

    let _server = tokio::spawn(async move {
        let _ = run_gateway(listener, tx, egress_rx, None).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_url = format!("ws://{}/", addr);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut write, mut read) = ws_stream.split();
    let identify = PodToGateway::Identify {
        device_id: "pod-xyz".to_string(),
        room: None,
    };
    write
        .send(Message::Text(serde_json::to_string(&identify)?))
        .await?;
    let hello_ack = read
        .next()
        .await
        .ok_or_else(|| io_err("connection closed before identify ack"))??;
    match hello_ack {
        Message::Text(_) => {}
        other => return Err(io_err(format!("unexpected identify ack: {other:?}")).into()),
    }

    egress_tx
        .send(PodEgressCommand::ToDevice {
            device_id: "pod-xyz".to_string(),
            msg: GatewayToPod::Audio {
                payload: AudioPayload(vec![1, 2, 3, 4]),
            },
        })
        .map_err(|error| io_err(format!("egress send failed: {error}")))?;

    let response = recv_with_timeout(read.next(), "timeout waiting for egress frame").await?;
    let response = response.ok_or_else(|| io_err("connection closed before egress frame"))??;
    match response {
        Message::Text(s) => {
            let msg: GatewayToPod = serde_json::from_str(&s)?;
            assert_eq!(
                msg,
                GatewayToPod::Audio {
                    payload: AudioPayload(vec![1, 2, 3, 4])
                }
            );
        }
        other => panic!("unexpected message: {other:?}"),
    }
    Ok(())
}
