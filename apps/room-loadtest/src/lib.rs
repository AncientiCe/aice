//! Simulated room pods for measuring capacity end to end.
//!
//! Each simulated pod connects to the room bridge with its device token,
//! speaks an utterance at real-time pace (40 ms frames), goes quiet, and
//! measures the time from the end of speech to the first answer audio. The
//! same code runs as a CI gate against an in-process stack and, through the
//! `room-loadtest` binary, against a staging property with real Whisper and
//! Ollama.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pod_protocol::{AudioPayload, GatewayToPod, PodToGateway};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// 40 ms at 16 kHz mono PCM16.
const FRAME_BYTES: usize = 1280;
const FRAME_TIME: Duration = Duration::from_millis(40);

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Setup(String),
}

pub struct LoadSettings {
    /// `ws://` or `wss://` room bridge URL.
    pub bridge_url: String,
    /// TLS name when it differs from the URL host.
    pub server_name: Option<String>,
    /// Property CA for `wss://`.
    pub ca_file: Option<std::path::PathBuf>,
    /// One device token per simulated room.
    pub tokens: Vec<String>,
    /// PCM16 LE 16 kHz mono utterance, sent once per turn.
    pub utterance: Vec<u8>,
    pub turns_per_room: usize,
    pub answer_timeout: Duration,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub rooms: usize,
    pub answered: usize,
    pub failures: usize,
    /// End of speech to first answer audio, per answered turn.
    pub latencies: Vec<Duration>,
    pub errors: Vec<String>,
}

impl LoadReport {
    /// Latency at `percent` (0-100), nearest rank.
    pub fn percentile(&self, percent: f64) -> Option<Duration> {
        if self.latencies.is_empty() {
            return None;
        }
        let mut sorted = self.latencies.clone();
        sorted.sort();
        let rank = ((percent / 100.0) * sorted.len() as f64).ceil() as usize;
        sorted
            .get(rank.saturating_sub(1).min(sorted.len() - 1))
            .copied()
    }
}

impl std::fmt::Display for LoadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |value: Option<Duration>| {
            value
                .map(|d| format!("{} ms", d.as_millis()))
                .unwrap_or_else(|| "-".to_string())
        };
        write!(
            f,
            "rooms {} | answered {} | failed {} | p50 {} | p95 {} | max {}",
            self.rooms,
            self.answered,
            self.failures,
            show(self.percentile(50.0)),
            show(self.percentile(95.0)),
            show(self.latencies.iter().max().copied()),
        )?;
        for error in self.errors.iter().take(5) {
            write!(f, "\n  error: {error}")?;
        }
        Ok(())
    }
}

/// A synthetic utterance loud enough to trigger speech detection.
pub fn speech_like_pcm(length: Duration) -> Vec<u8> {
    let samples = (length.as_millis() as usize) * 16;
    (0..samples)
        .flat_map(|i| {
            let phase = (i % 32) as f32 / 32.0 * std::f32::consts::TAU;
            ((phase.sin() * 9_000.0) as i16).to_le_bytes()
        })
        .collect()
}

/// Enrol `count` simulated pods on a facilitator and assign each a room
/// directly in its database (the loadtest runs on the facilitator host).
pub async fn provision_pods(
    facilitator_url: &str,
    ca_file: Option<&Path>,
    database_path: &Path,
    prefix: &str,
    count: usize,
) -> Result<Vec<String>, LoadError> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if let Some(ca) = ca_file {
        let pem = std::fs::read(ca).map_err(|e| LoadError::Setup(e.to_string()))?;
        let cert =
            reqwest::Certificate::from_pem(&pem).map_err(|e| LoadError::Setup(e.to_string()))?;
        builder = builder
            .use_rustls_tls()
            .tls_built_in_root_certs(false)
            .add_root_certificate(cert);
    }
    let http = builder
        .build()
        .map_err(|e| LoadError::Setup(e.to_string()))?;
    let mut tokens = Vec::with_capacity(count);
    for index in 0..count {
        let device_id = format!("{prefix}-{index:04}");
        let body = json!({
            "device_id": device_id,
            "nonce": format!("{device_id}-loadtest-nonce"),
            "firmware": "loadtest",
            "lost_token": true,
        });
        let enroll = || async {
            http.post(format!("{facilitator_url}/api/devices/enroll"))
                .json(&body)
                .send()
                .await
                .map_err(|e| LoadError::Setup(e.to_string()))?
                .json::<Value>()
                .await
                .map_err(|e| LoadError::Setup(e.to_string()))
        };
        let _ = enroll().await?;
        property_facilitator::assign_device(database_path, &device_id, &format!("load-{index:04}"))
            .map_err(|e| LoadError::Setup(e.to_string()))?;
        let token = enroll().await?["token"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| LoadError::Setup(format!("{device_id} got no token")))?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Run every simulated room at once and collect latencies.
pub async fn run_load(settings: &LoadSettings) -> LoadReport {
    let tls = match &settings.ca_file {
        Some(ca) => match core_tls::client_config_trusting(ca) {
            Ok(config) => Some(config),
            Err(error) => {
                return LoadReport {
                    rooms: settings.tokens.len(),
                    failures: settings.tokens.len(),
                    errors: vec![error.to_string()],
                    ..LoadReport::default()
                }
            }
        },
        None => None,
    };
    let shared = Arc::new(Shared {
        url: settings.bridge_url.clone(),
        server_name: settings.server_name.clone(),
        tls,
        utterance: settings.utterance.clone(),
        turns: settings.turns_per_room,
        timeout: settings.answer_timeout,
    });
    let mut tasks = Vec::new();
    for (index, token) in settings.tokens.iter().enumerate() {
        let shared = Arc::clone(&shared);
        let token = token.clone();
        tasks.push(tokio::spawn(
            async move { room(index, &shared, &token).await },
        ));
    }
    let mut report = LoadReport {
        rooms: settings.tokens.len(),
        ..LoadReport::default()
    };
    for task in tasks {
        match task.await {
            Ok(outcome) => {
                report.answered += outcome.latencies.len();
                report.latencies.extend(outcome.latencies);
                report.failures += outcome.failures;
                report.errors.extend(outcome.errors);
            }
            Err(error) => {
                report.failures += settings.turns_per_room;
                report.errors.push(error.to_string());
            }
        }
    }
    report
}

#[derive(Default)]
struct RoomOutcome {
    latencies: Vec<Duration>,
    failures: usize,
    errors: Vec<String>,
}

trait Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Io for T {}

/// Settings every simulated room shares.
struct Shared {
    url: String,
    server_name: Option<String>,
    tls: Option<Arc<core_tls::rustls::ClientConfig>>,
    utterance: Vec<u8>,
    turns: usize,
    timeout: Duration,
}

async fn room(index: usize, shared: &Shared, token: &str) -> RoomOutcome {
    let Shared {
        url,
        server_name,
        tls,
        utterance,
        turns,
        timeout,
    } = shared;
    let (turns, timeout) = (*turns, *timeout);
    let mut outcome = RoomOutcome::default();
    let socket = match connect(url, server_name.as_deref(), tls.clone(), token).await {
        Ok(socket) => socket,
        Err(error) => {
            outcome.failures = turns;
            outcome.errors.push(format!("room {index}: {error}"));
            return outcome;
        }
    };
    let (mut write, mut read) = socket.split();
    let hello = PodToGateway::Hello {
        protocol_version: 1,
        device_id: format!("loadtest-{index}"),
        room: None,
    };
    if write
        .send(Message::Text(
            serde_json::to_string(&hello).unwrap_or_default(),
        ))
        .await
        .is_err()
    {
        outcome.failures = turns;
        return outcome;
    }
    let silence = vec![0u8; FRAME_BYTES];
    for turn in 0..turns {
        let mut paced = tokio::time::interval(FRAME_TIME);
        let frames = utterance
            .chunks(FRAME_BYTES)
            .map(<[u8]>::to_vec)
            .chain(std::iter::repeat_n(silence.clone(), 25));
        let mut speech_ended = None;
        let spoken_frames = utterance.len().div_ceil(FRAME_BYTES);
        let mut first_audio = None;
        for (position, frame) in frames.enumerate() {
            paced.tick().await;
            if position == spoken_frames {
                speech_ended = Some(Instant::now());
            }
            let audio = PodToGateway::Audio {
                payload: AudioPayload(frame),
            };
            if write
                .send(Message::Text(
                    serde_json::to_string(&audio).unwrap_or_default(),
                ))
                .await
                .is_err()
            {
                break;
            }
            // Drain anything the bridge already sent without blocking the pace.
            while let Ok(Some(Ok(message))) =
                tokio::time::timeout(Duration::from_millis(1), read.next()).await
            {
                if is_audio(&message) && first_audio.is_none() {
                    first_audio = Some(Instant::now());
                }
            }
            if first_audio.is_some() {
                break;
            }
        }
        let deadline = Instant::now() + timeout;
        while first_audio.is_none() && Instant::now() < deadline {
            match tokio::time::timeout(deadline - Instant::now(), read.next()).await {
                Ok(Some(Ok(message))) if is_audio(&message) => first_audio = Some(Instant::now()),
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
        match (speech_ended, first_audio) {
            (Some(ended), Some(heard)) => {
                outcome
                    .latencies
                    .push(heard.saturating_duration_since(ended));
                // Let the answer finish before speaking again, as a guest would.
                let quiet_until = Instant::now() + Duration::from_millis(600);
                while let Ok(Some(Ok(_))) = tokio::time::timeout(
                    quiet_until.saturating_duration_since(Instant::now()),
                    read.next(),
                )
                .await
                {}
            }
            _ => {
                outcome.failures += 1;
                outcome
                    .errors
                    .push(format!("room {index} turn {turn}: no answer audio"));
            }
        }
    }
    outcome
}

fn is_audio(message: &Message) -> bool {
    match message {
        Message::Text(text) => matches!(
            serde_json::from_str::<GatewayToPod>(text),
            Ok(GatewayToPod::Audio { .. })
        ),
        _ => false,
    }
}

async fn connect(
    url: &str,
    server_name: Option<&str>,
    tls: Option<Arc<core_tls::rustls::ClientConfig>>,
    token: &str,
) -> Result<tokio_tungstenite::WebSocketStream<Box<dyn Io>>, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let header = format!("Bearer {token}")
        .parse()
        .map_err(|_| "bad token".to_string())?;
    request.headers_mut().insert("authorization", header);
    let uri = request.uri().clone();
    let host = uri.host().ok_or("url has no host")?.to_string();
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| e.to_string())?;
    let stream: Box<dyn Io> = match (secure, tls) {
        (true, Some(config)) => {
            let name = core_tls::rustls::pki_types::ServerName::try_from(
                server_name.unwrap_or(&host).to_string(),
            )
            .map_err(|e| e.to_string())?;
            Box::new(
                core_tls::TlsConnector::from(config)
                    .connect(name, tcp)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        (true, None) => return Err("wss needs --ca".to_string()),
        (false, _) => Box::new(tcp),
    };
    tokio_tungstenite::client_async(request, stream)
        .await
        .map(|(socket, _)| socket)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{speech_like_pcm, LoadReport};
    use std::time::Duration;

    #[test]
    fn percentiles_use_nearest_rank() {
        let report = LoadReport {
            latencies: (1..=20).map(Duration::from_millis).collect(),
            ..LoadReport::default()
        };
        assert_eq!(report.percentile(50.0), Some(Duration::from_millis(10)));
        assert_eq!(report.percentile(95.0), Some(Duration::from_millis(19)));
        assert_eq!(report.percentile(100.0), Some(Duration::from_millis(20)));
        assert_eq!(LoadReport::default().percentile(95.0), None);
    }

    #[test]
    fn synthetic_speech_has_the_requested_length() {
        assert_eq!(speech_like_pcm(Duration::from_millis(40)).len(), 1280);
    }
}
