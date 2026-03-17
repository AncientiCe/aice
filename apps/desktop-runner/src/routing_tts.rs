//! TTS sink that routes to local playback or to pod egress.

use async_trait::async_trait;
use core_observability::{record_pod_egress_send_error, record_pod_tts_chunk};
use core_orchestrator::TtsSink;
use core_tts::PiperTtsSink;
use pod_gateway::PodEgressCommand;
use pod_protocol::{AudioPayload, GatewayToPod};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;

/// Max bytes per WebSocket audio frame to the pod (ESP32 can't handle 300KB+ frames).
const POD_TTS_CHUNK_BYTES: usize = 2048;
/// Delay between chunk sends so the pod can receive and play without being flooded.
const POD_TTS_CHUNK_DELAY_MS: u64 = 35;
/// Pod-only high-cut smoothing coefficient (Q15).
const POD_PCM_LP_ALPHA_Q15: i64 = 9011; // ~0.275

/// TTS that plays locally or sends PCM to a pod when device_id is set.
pub struct RoutingTtsSink {
    piper: PiperTtsSink,
    egress_tx: Option<UnboundedSender<PodEgressCommand>>,
    egress_device_id: Mutex<Option<String>>,
    buffer: String,
}

impl RoutingTtsSink {
    pub fn new(piper: PiperTtsSink, egress_tx: Option<UnboundedSender<PodEgressCommand>>) -> Self {
        Self {
            piper,
            egress_tx,
            egress_device_id: Mutex::new(None),
            buffer: String::new(),
        }
    }

    fn smooth_for_pod(pcm: &[u8]) -> Vec<u8> {
        if pcm.len() < 2 {
            return pcm.to_vec();
        }
        let mut out = Vec::with_capacity(pcm.len());
        let mut initialized = false;
        let mut state_q15: i64 = 0;
        for frame in pcm.chunks_exact(2) {
            let s = i16::from_le_bytes([frame[0], frame[1]]) as i64;
            let x_q15 = s << 15;
            if !initialized {
                initialized = true;
                state_q15 = x_q15;
            } else {
                state_q15 += ((x_q15 - state_q15) * POD_PCM_LP_ALPHA_Q15) >> 15;
            }
            let y = (state_q15 >> 15).clamp(i16::MIN as i64, i16::MAX as i64) as i16;
            out.extend_from_slice(&y.to_le_bytes());
        }
        out
    }

    async fn send_pcm_to_pod(
        &self,
        tx: &UnboundedSender<PodEgressCommand>,
        device_id: &str,
        pcm: &[u8],
        log_label: &str,
    ) {
        let _ = tx.send(PodEgressCommand::ToDevice {
            device_id: device_id.to_string(),
            msg: GatewayToPod::StopAudio,
        });
        if pcm.is_empty() {
            return;
        }
        let smoothed = Self::smooth_for_pod(pcm);
        let total = smoothed.len();
        let chunks: Vec<_> = smoothed
            .chunks(POD_TTS_CHUNK_BYTES)
            .map(|c| c.to_vec())
            .collect();
        let n_chunks = chunks.len();
        tracing::info!(
            device_id = %device_id,
            bytes = total,
            chunks = n_chunks,
            mode = log_label,
            "sending audio to pod"
        );
        for (i, chunk) in chunks.into_iter().enumerate() {
            let bytes = chunk.len();
            if tx
                .send(PodEgressCommand::ToDevice {
                    device_id: device_id.to_string(),
                    msg: GatewayToPod::Audio {
                        payload: AudioPayload(chunk),
                    },
                })
                .is_err()
            {
                record_pod_egress_send_error(device_id);
                tracing::warn!(device_id = %device_id, "failed to send audio chunk to gateway egress");
                break;
            }
            record_pod_tts_chunk(device_id, bytes);
            if i + 1 < n_chunks {
                sleep(Duration::from_millis(POD_TTS_CHUNK_DELAY_MS)).await;
            }
        }
    }
}

#[async_trait]
impl TtsSink for RoutingTtsSink {
    fn set_egress_device(&mut self, device_id: Option<String>) {
        *self.egress_device_id.lock().expect("lock") = device_id;
    }

    fn request_stop_playback(&mut self) {
        if let (Some(tx), Some(id)) = (
            &self.egress_tx,
            self.egress_device_id.lock().expect("lock").clone(),
        ) {
            let _ = tx.send(PodEgressCommand::ToDevice {
                device_id: id,
                msg: GatewayToPod::StopAudio,
            });
        }
    }

    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.buffer.push_str(text);
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let device_id = self.egress_device_id.lock().expect("lock").clone();
        let text = std::mem::take(&mut self.buffer);
        if text.trim().is_empty() {
            return Ok(());
        }
        if let (Some(tx), Some(id)) = (&self.egress_tx, device_id) {
            let pcm = self.piper.synthesize_to_pcm(&text)?;
            self.send_pcm_to_pod(tx, &id, &pcm, "tts").await;
            Ok(())
        } else {
            self.piper.push_text(&text).await?;
            self.piper.flush().await
        }
    }

    async fn play_pcm_bytes(
        &mut self,
        pcm: &[u8],
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let device_id = self.egress_device_id.lock().expect("lock").clone();
        if let (Some(tx), Some(id)) = (&self.egress_tx, device_id) {
            self.send_pcm_to_pod(tx, &id, pcm, "raw").await;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RoutingTtsSink;

    fn decode_i16(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    fn mean_abs_diff(samples: &[i16]) -> i64 {
        if samples.len() < 2 {
            return 0;
        }
        let mut sum = 0i64;
        for w in samples.windows(2) {
            sum += (w[1] as i32 - w[0] as i32).abs() as i64;
        }
        sum / (samples.len() as i64 - 1)
    }

    #[test]
    fn smooth_for_pod_reduces_harsh_edges() {
        let mut raw = Vec::new();
        for i in 0..800 {
            let s: i16 = if i % 2 == 0 { 12000 } else { -12000 };
            raw.extend_from_slice(&s.to_le_bytes());
        }
        let smoothed = RoutingTtsSink::smooth_for_pod(&raw);
        let in_s = decode_i16(&raw);
        let out_s = decode_i16(&smoothed);
        assert_eq!(in_s.len(), out_s.len());
        assert!(mean_abs_diff(&out_s) < mean_abs_diff(&in_s));
    }
}
