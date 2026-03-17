//! WebSocket server that receives pod messages and emits ingest events.

use core_observability::{
    record_pod_audio_frame, record_pod_connection, record_pod_disconnect,
    record_pod_egress_queue_drop, record_pod_egress_send_error,
};
use futures_util::{SinkExt, StreamExt};
use pod_protocol::{GatewayToPod, PodToGateway};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::info;

/// One audio chunk ingested from a pod (for pipeline input).
#[derive(Clone, Debug)]
pub struct PodIngestEvent {
    pub device_id: String,
    pub pcm: Vec<i16>,
}

/// Outbound command to pod(s).
#[derive(Clone, Debug)]
pub enum PodEgressCommand {
    ToDevice {
        device_id: String,
        msg: GatewayToPod,
    },
    Broadcast {
        msg: GatewayToPod,
    },
}

fn bytes_to_pcm(payload: &[u8]) -> Vec<i16> {
    payload
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

type SessionMap = Arc<Mutex<HashMap<String, mpsc::Sender<GatewayToPod>>>>;

const MAX_AUDIO_PAYLOAD_BYTES: usize = 64 * 1024;
const PROTOCOL_VERSION: u16 = 1;

/// Sender for pod button tap (stop/wake). Send () when a pod sends TapActivate.
pub type TapSender = mpsc::UnboundedSender<()>;

/// Run the pod gateway:
/// - accept websocket connections
/// - forward ingested audio to `tx`
/// - forward outbound messages from `egress_rx` to matching pod sessions
/// - when a pod sends TapActivate, send () on `tap_tx` if provided (so voice pipeline can stop playback)
pub async fn run_gateway(
    listener: TcpListener,
    tx: mpsc::UnboundedSender<PodIngestEvent>,
    mut egress_rx: mpsc::UnboundedReceiver<PodEgressCommand>,
    tap_tx: Option<TapSender>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = listener.local_addr()?;
    info!(%addr, "pod gateway listening");
    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    let sessions_for_dispatch = Arc::clone(&sessions);
    tokio::spawn(async move {
        while let Some(cmd) = egress_rx.recv().await {
            match cmd {
                PodEgressCommand::ToDevice { device_id, msg } => {
                    if let Some(sender) =
                        sessions_for_dispatch.lock().await.get(&device_id).cloned()
                    {
                        if sender.try_send(msg).is_err() {
                            record_pod_egress_queue_drop(&device_id);
                            tracing::warn!(%device_id, "dropping egress message: pod session queue full");
                        }
                    } else {
                        record_pod_egress_send_error(&device_id);
                        tracing::warn!(%device_id, "egress target not connected");
                    }
                }
                PodEgressCommand::Broadcast { msg } => {
                    let targets = sessions_for_dispatch
                        .lock()
                        .await
                        .values()
                        .cloned()
                        .collect::<Vec<_>>();
                    for tx in targets {
                        if tx.try_send(msg.clone()).is_err() {
                            record_pod_egress_queue_drop("broadcast");
                        }
                    }
                }
            }
        }
    });

    loop {
        let (stream, peer) = listener.accept().await?;
        let tx = tx.clone();
        let sessions = Arc::clone(&sessions);
        let tap_tx = tap_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, tx, sessions, tap_tx).await {
                tracing::warn!(%peer, error = %e, "pod connection error");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    tx: mpsc::UnboundedSender<PodIngestEvent>,
    sessions: SessionMap,
    tap_tx: Option<TapSender>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws = accept_async(stream).await?;
    let (mut write, mut read) = ws.split();
    let mut device_id = peer.to_string();
    let (egress_tx, mut egress_rx) = mpsc::channel::<GatewayToPod>(64);
    sessions
        .lock()
        .await
        .insert(device_id.clone(), egress_tx.clone());
    let mut first_audio_logged = false;

    let write_task = tokio::spawn(async move {
        while let Some(msg) = egress_rx.recv().await {
            let text = serde_json::to_string(&msg)?;
            write.send(Message::Text(text)).await?;
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    while let Some(msg) = read.next().await {
        let msg = msg?;
        match msg {
            Message::Text(s) => {
                let parsed: Result<PodToGateway, _> = serde_json::from_str(&s);
                match parsed {
                    Ok(PodToGateway::Hello {
                        protocol_version: _,
                        device_id: id,
                        room: _,
                    })
                    | Ok(PodToGateway::Identify {
                        device_id: id,
                        room: _,
                    }) => {
                        sessions.lock().await.remove(&device_id);
                        device_id = id;
                        sessions
                            .lock()
                            .await
                            .insert(device_id.clone(), egress_tx.clone());
                        tracing::info!(%device_id, "pod connected");
                        record_pod_connection(&device_id);
                        let _ = egress_tx
                            .send(GatewayToPod::HelloAck {
                                protocol_version: PROTOCOL_VERSION,
                            })
                            .await;
                    }
                    Ok(PodToGateway::Audio { payload }) => {
                        if !first_audio_logged {
                            first_audio_logged = true;
                            tracing::info!(%device_id, "pod audio stream started");
                        }
                        let bytes = &payload.0;
                        if bytes.len() > MAX_AUDIO_PAYLOAD_BYTES {
                            let _ = egress_tx
                                .send(GatewayToPod::Error {
                                    code: "payload_too_large".to_string(),
                                    message: "audio payload exceeds gateway limit".to_string(),
                                })
                                .await;
                            continue;
                        }
                        record_pod_audio_frame(&device_id, bytes.len());
                        let pcm = bytes_to_pcm(bytes);
                        let _ = tx.send(PodIngestEvent {
                            device_id: device_id.clone(),
                            pcm,
                        });
                    }
                    Ok(PodToGateway::Ping { seq }) => {
                        let _ = egress_tx.send(GatewayToPod::Pong { seq }).await;
                    }
                    Ok(PodToGateway::TapActivate) => {
                        if let Some(ref send) = tap_tx {
                            let _ = send.send(());
                            tracing::info!(%device_id, "pod button tap — stop/wake");
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let truncated = if msg.len() > 200 {
                            format!("{}... (truncated)", &msg[..200])
                        } else {
                            msg
                        };
                        tracing::debug!(%peer, error = %truncated, "invalid pod message");
                        let _ = egress_tx
                            .send(GatewayToPod::Error {
                                code: "invalid_message".to_string(),
                                message: truncated,
                            })
                            .await;
                    }
                }
            }
            Message::Binary(_) => {
                let _ = egress_tx
                    .send(GatewayToPod::Error {
                        code: "binary_not_supported".to_string(),
                        message: "send JSON text frames".to_string(),
                    })
                    .await;
            }
            Message::Close(frame) => {
                if let Some(frame) = frame {
                    tracing::info!(
                        %device_id,
                        code = ?frame.code,
                        reason = %frame.reason,
                        "pod websocket closed"
                    );
                } else {
                    tracing::info!(%device_id, "pod websocket closed");
                }
                break;
            }
            _ => {}
        }
    }
    sessions.lock().await.remove(&device_id);
    record_pod_disconnect(&device_id);
    write_task.abort();
    Ok(())
}
