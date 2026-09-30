//! Room bridge: room pods on one side, the aice backend on the other.
//!
//! A pod connects with its facilitator-issued device token and streams
//! microphone audio. The bridge finds where speech starts and ends, runs that
//! stretch as one turn on the backend's `/turns/stream` (with the pod's token,
//! so the backend decides the room and the stay), turns the answer into speech,
//! and streams it back to the pod speaker. A full-duplex pod also sends what
//! its speaker played; the bridge cancels that echo and lets the guest's
//! speech interrupt an answer. The bridge never interprets what was said; the
//! backend's LLM does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use core_observability::{
    record_pod_audio_frame, record_pod_bridge_barge_in, record_pod_bridge_barge_in_latency,
    record_pod_bridge_turn, record_pod_bridge_turn_duration, record_pod_connection,
    record_pod_disconnect, record_pod_egress_queue_drop, record_pod_egress_send_error,
    record_pod_tts_chunk,
};
use core_runtime_protocol::{
    FrontendSkillResultRequest, TurnStreamClientMessage, TurnStreamServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use pod_protocol::{AudioPayload, GatewayToPod, LedState, PodToGateway};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

use crate::echo::{EchoCanceller, DEFAULT_ECHO_TAPS};

type DynError = Box<dyn std::error::Error + Send + Sync>;

const PROTOCOL_VERSION: u16 = 1;
/// Largest audio payload a pod may send in one frame.
pub const MAX_AUDIO_PAYLOAD_BYTES: usize = 64 * 1024;
/// Largest audio chunk sent to a pod (its playback slots are 2 KiB).
const POD_AUDIO_CHUNK_BYTES: usize = 2048;
/// Audio time in one full chunk: 1024 samples at 16 kHz.
const POD_AUDIO_CHUNK_TIME: Duration = Duration::from_millis(64);
/// Chunks sent ahead of real time. The pod queues six; three keeps it fed
/// without overflowing.
const POD_AUDIO_LEAD_CHUNKS: u32 = 3;

/// Text in, 16 kHz mono PCM16 little-endian bytes out.
pub trait Speech: Send + Sync + 'static {
    fn synthesize(&self, text: &str) -> Result<Vec<u8>, String>;
}

/// Piper voice for room pods.
pub struct PiperSpeech(pub core_tts::PiperSynth);

impl Speech for PiperSpeech {
    fn synthesize(&self, text: &str) -> Result<Vec<u8>, String> {
        self.0
            .synthesize_to_pcm(text)
            .map_err(|error| error.to_string())
    }
}

/// Energy-based speech detection on the pod's 40 ms frames.
#[derive(Clone, Debug)]
pub struct VadSettings {
    /// Mean absolute sample value that counts as speech.
    pub start_level: i16,
    /// Consecutive loud frames that start a turn.
    pub start_frames: usize,
    /// Quiet time that ends a turn.
    pub end_silence: Duration,
    /// Longest single turn.
    pub max_turn: Duration,
    /// Frames kept from before speech started, so the first word is not cut.
    pub preroll_frames: usize,
    /// Speech that interrupts an answer on a full-duplex pod.
    pub barge_in: BargeInSettings,
}

/// Onset during an answer, measured on echo-cancelled audio. A small pod
/// speaker distorts, so some of its voice is always left after cancelling;
/// speech must be loud on its own and clearly louder than the echo predicted
/// for the same frame.
#[derive(Clone, Debug)]
pub struct BargeInSettings {
    /// Mean absolute sample value that counts as the guest talking over.
    pub start_level: i16,
    /// Consecutive frames that must pass both tests.
    pub start_frames: usize,
    /// How many times the predicted echo level a frame must reach.
    pub echo_ratio: f32,
}

impl Default for BargeInSettings {
    fn default() -> Self {
        // Tuned on a Waveshare pod recording: no false interruption over 27 s
        // of answers, speech caught within a second about 60% of the time.
        Self {
            start_level: 600,
            start_frames: 3,
            echo_ratio: 1.5,
        }
    }
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            start_level: 400,
            start_frames: 3,
            end_silence: Duration::from_millis(700),
            max_turn: Duration::from_secs(15),
            preroll_frames: 8,
            barge_in: BargeInSettings::default(),
        }
    }
}

#[derive(Clone)]
pub struct BridgeSettings {
    /// `ws://` or `wss://` URL of the backend's `/turns/stream`.
    pub backend_url: String,
    /// Trust for a `wss://` backend (the property CA).
    pub backend_tls: Option<Arc<core_tls::rustls::ClientConfig>>,
    /// Serve pods over TLS.
    pub pod_tls: Option<core_tls::TlsAcceptor>,
    pub vad: VadSettings,
}

pub struct BridgeHandle {
    pub bind: String,
    task: tokio::task::JoinHandle<()>,
}

impl BridgeHandle {
    pub async fn until_ctrl_c(self) -> Result<(), DynError> {
        tokio::signal::ctrl_c().await?;
        self.task.abort();
        Ok(())
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn spawn_bridge(
    bind: &str,
    settings: BridgeSettings,
    speech: Arc<dyn Speech>,
) -> Result<BridgeHandle, DynError> {
    let listener = TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    info!(%local, "pod bridge listening");
    let settings = Arc::new(settings);
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            let settings = Arc::clone(&settings);
            let speech = Arc::clone(&speech);
            tokio::spawn(async move {
                let result = match settings.pod_tls.clone() {
                    Some(acceptor) => match acceptor.accept(stream).await {
                        Ok(tls) => serve_pod(tls, settings, speech).await,
                        Err(error) => Err(error.into()),
                    },
                    None => serve_pod(stream, settings, speech).await,
                };
                if let Err(error) = result {
                    warn!(%peer, %error, "pod session ended with an error");
                }
            });
        }
    });
    Ok(BridgeHandle {
        bind: local.to_string(),
        task,
    })
}

async fn serve_pod<S>(
    stream: S,
    settings: Arc<BridgeSettings>,
    speech: Arc<dyn Speech>,
) -> Result<(), DynError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let bearer = Arc::new(std::sync::Mutex::new(None));
    let pod = tokio_tungstenite::accept_hdr_async(
        stream,
        BearerCapture {
            bearer: Arc::clone(&bearer),
        },
    )
    .await?;
    let bearer: Option<String> = bearer.lock().ok().and_then(|mut slot| slot.take());
    let (mut pod_tx, mut pod_rx) = pod.split();

    // Wait for the pod to say hello before opening a backend turn stream.
    let (device_label, full_duplex) = loop {
        match pod_rx.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<PodToGateway>(&text) {
                Ok(PodToGateway::Hello {
                    device_id,
                    full_duplex,
                    ..
                }) => break (device_id, full_duplex),
                Ok(PodToGateway::Identify { device_id, .. }) => break (device_id, false),
                Ok(_) => continue,
                Err(error) => {
                    send_pod(
                        &mut pod_tx,
                        &error_message("invalid_message", &error.to_string()),
                    )
                    .await?;
                }
            },
            Some(Ok(Message::Close(_))) | None => return Ok(()),
            Some(Ok(_)) => continue,
            Some(Err(error)) => return Err(error.into()),
        }
    };

    let backend = match connect_backend(&settings, bearer.as_deref()).await {
        Ok(backend) => backend,
        Err(BackendRefusal::Unauthorized) => {
            record_pod_bridge_turn("unauthorized");
            send_pod(
                &mut pod_tx,
                &error_message("unauthorized", "device token was not accepted"),
            )
            .await?;
            let _ = pod_tx.close().await;
            return Ok(());
        }
        Err(BackendRefusal::Unavailable(reason)) => {
            record_pod_bridge_turn("backend_unavailable");
            send_pod(&mut pod_tx, &error_message("backend_unavailable", &reason)).await?;
            let _ = pod_tx.close().await;
            return Ok(());
        }
    };
    record_pod_connection(&device_label);
    info!(device = %device_label, full_duplex, "pod bridged to backend");
    let (mut backend_tx, mut backend_rx) = backend.split();

    // Pod writes go through one queue so speech and control never interleave badly.
    let (out_tx, mut out_rx) = mpsc::channel::<GatewayToPod>(256);
    let writer_label = device_label.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if send_pod(&mut pod_tx, &message).await.is_err() {
                record_pod_egress_send_error(&writer_label);
                break;
            }
        }
        let _ = pod_tx.close().await;
    });
    let push = |message: GatewayToPod| {
        if out_tx.try_send(message).is_err() {
            record_pod_egress_queue_drop(&device_label);
        }
    };
    push(GatewayToPod::HelloAck {
        protocol_version: PROTOCOL_VERSION,
    });
    push(GatewayToPod::Led {
        state: LedState::Listening,
    });

    let mut turn = TurnDetector::new(settings.vad.clone());
    turn.allow_barge_in(full_duplex);
    let mut echo = full_duplex.then(|| EchoCanceller::new(DEFAULT_ECHO_TAPS));
    let mut echo_levels = LevelMeter::default();
    let mut speaker_tail = SpeakerTail::default();
    // When the current answer started playing and how long it is.
    let mut answer_playback = Playback::started(&[]);
    // True from a barge-in until its turn is answered (or comes back empty).
    let mut barge_in_turn = false;
    // Audio of the latest turn, so speech that goes on after a pause (before
    // any answer) is sent again as one request.
    let mut last_turn_audio: Vec<Vec<u8>> = Vec::new();
    let mut answer = String::new();
    let mut turn_counter: u64 = 0;
    let mut turn_started_at = Instant::now();
    // Speech is tagged with the turn it answers; an interrupted turn's speech is dropped.
    let (spoken_tx, mut spoken_rx) = mpsc::channel::<(u64, Result<Vec<u8>, String>)>(1);
    // True between turn_done and the answer; a cancelled turn's answer is dropped.
    let mut awaiting_answer = false;
    let mut speaking: Option<tokio::task::JoinHandle<()>> = None;
    // Signals that the pod has been sent the whole answer, so the backend's
    // wake-word awake window can run from the end of playback.
    let (played_tx, mut played_rx) = mpsc::channel::<u64>(1);
    let mut playback: u64 = 0;

    let result: Result<(), DynError> = async {
        loop {
            tokio::select! {
                incoming = pod_rx.next() => {
                    let Some(incoming) = incoming else { break };
                    let text = match incoming? {
                        Message::Text(text) => text,
                        Message::Close(_) => break,
                        Message::Binary(_) => {
                            push(error_message("binary_not_supported", "send JSON text frames"));
                            continue;
                        }
                        _ => continue,
                    };
                    let message = match serde_json::from_str::<PodToGateway>(&text) {
                        Ok(message) => message,
                        Err(error) => {
                            push(error_message("invalid_message", &truncate(&error.to_string())));
                            continue;
                        }
                    };
                    match message {
                        PodToGateway::Audio { payload, reference } => {
                            let too_large = payload.0.len() > MAX_AUDIO_PAYLOAD_BYTES
                                || reference.as_ref().is_some_and(|r| r.0.len() > MAX_AUDIO_PAYLOAD_BYTES);
                            if too_large {
                                push(error_message("payload_too_large", "audio payload exceeds gateway limit"));
                                continue;
                            }
                            record_pod_audio_frame(&device_label, payload.0.len());
                            let received_at = Instant::now();
                            let (frame, echo_level) = match (echo.as_mut(), reference) {
                                (Some(canceller), Some(reference)) => {
                                    let (cleaned, echo_level) = cancel_echo(canceller, &payload.0, &reference.0);
                                    if let Some([mic, speaker, left]) =
                                        echo_levels.add([&payload.0, &reference.0, &cleaned])
                                    {
                                        debug!(device = %device_label, mic, speaker, left, playing = speaking.is_some(), "echo levels (mean abs per second)");
                                    }
                                    if speaker_tail.speaker_frame(mean_abs_level(&reference.0)) {
                                        turn.listen();
                                    }
                                    (cleaned, echo_level)
                                }
                                _ => (payload.0, 0),
                            };
                            for step in turn.push_with_echo(&frame, echo_level) {
                                match step {
                                    TurnStep::Start(preroll) => {
                                        barge_in_turn = false;
                                        turn_counter += 1;
                                        turn_started_at = Instant::now();
                                        answer.clear();
                                        last_turn_audio = preroll.clone();
                                        backend_tx.send(Message::Text(turn_start(&device_label, turn_counter)?)).await?;
                                        for frame in preroll {
                                            backend_tx.send(Message::Binary(frame)).await?;
                                        }
                                    }
                                    TurnStep::BargeIn(preroll) if awaiting_answer && speaking.is_none() && !speaker_tail.waiting() => {
                                        // Nothing has been said yet: the guest paused and went on
                                        // ("Computer, ... how do I make tahini"). Ask again with
                                        // both parts as one request instead of losing the first.
                                        awaiting_answer = false;
                                        backend_tx
                                            .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::TurnCancel)?))
                                            .await?;
                                        turn_counter += 1;
                                        answer.clear();
                                        let mut audio = std::mem::take(&mut last_turn_audio);
                                        audio.extend(preroll);
                                        backend_tx.send(Message::Text(turn_start(&device_label, turn_counter)?)).await?;
                                        for frame in &audio {
                                            backend_tx.send(Message::Binary(frame.clone())).await?;
                                        }
                                        last_turn_audio = audio;
                                        push(GatewayToPod::Led { state: LedState::Listening });
                                    }
                                    TurnStep::BargeIn(preroll) => {
                                        // The guest spoke over the answer: the tap_activate
                                        // stop path, then a turn that keeps their first words.
                                        let still_playing = speaker_tail.cancel();
                                        if let Some(task) = speaking.take() {
                                            task.abort();
                                            backend_tx
                                                .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::PlaybackFinished)?))
                                                .await?;
                                            backend_tx.send(Message::Text(interrupted(&answer_playback)?)).await?;
                                        } else if still_playing {
                                            backend_tx.send(Message::Text(interrupted(&answer_playback)?)).await?;
                                        }
                                        push(GatewayToPod::StopAudio);
                                        // Only a turn still being answered is open on the backend.
                                        if std::mem::take(&mut awaiting_answer) {
                                            backend_tx
                                                .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::TurnCancel)?))
                                                .await?;
                                        }
                                        let onset = frame_time(&frame) * u32::try_from(settings.vad.barge_in.start_frames).unwrap_or(1);
                                        record_pod_bridge_barge_in_latency(onset + received_at.elapsed());
                                        barge_in_turn = true;
                                        turn_counter += 1;
                                        turn_started_at = Instant::now();
                                        answer.clear();
                                        last_turn_audio = preroll.clone();
                                        backend_tx.send(Message::Text(turn_start(&device_label, turn_counter)?)).await?;
                                        for frame in preroll {
                                            backend_tx.send(Message::Binary(frame)).await?;
                                        }
                                    }
                                    TurnStep::Audio(frame) => {
                                        last_turn_audio.push(frame.clone());
                                        backend_tx.send(Message::Binary(frame)).await?;
                                    }
                                    TurnStep::Done => {
                                        backend_tx
                                            .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::TurnDone)?))
                                            .await?;
                                        awaiting_answer = true;
                                        push(GatewayToPod::Led { state: LedState::Thinking });
                                    }
                                }
                            }
                        }
                        PodToGateway::TapActivate => {
                            let still_playing = speaker_tail.cancel();
                            if let Some(task) = speaking.take() {
                                task.abort();
                                backend_tx
                                    .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::PlaybackFinished)?))
                                    .await?;
                                backend_tx.send(Message::Text(interrupted(&answer_playback)?)).await?;
                            } else if still_playing {
                                backend_tx.send(Message::Text(interrupted(&answer_playback)?)).await?;
                            }
                            awaiting_answer = false;
                            push(GatewayToPod::StopAudio);
                            if turn.cancel() {
                                backend_tx
                                    .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::TurnCancel)?))
                                    .await?;
                                record_pod_bridge_turn("cancelled");
                            }
                            push(GatewayToPod::Led { state: LedState::Listening });
                        }
                        PodToGateway::HelpButton => {
                            // No speech needed: the backend raises the ticket directly.
                            backend_tx
                                .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::HelpRequest)?))
                                .await?;
                            record_pod_bridge_turn("help_button");
                            push(GatewayToPod::Led { state: LedState::Thinking });
                        }
                        PodToGateway::Ping { seq } => push(GatewayToPod::Pong { seq }),
                        PodToGateway::Hello { .. } | PodToGateway::Identify { .. } => {
                            push(GatewayToPod::HelloAck { protocol_version: PROTOCOL_VERSION });
                        }
                    }
                }
                event = backend_rx.next() => {
                    let Some(event) = event else { break };
                    let Message::Text(text) = event? else { continue };
                    let Ok(event) = serde_json::from_str::<TurnStreamServerEvent>(&text) else { continue };
                    match event {
                        // Events for a turn the bridge has since replaced (cancelled,
                        // interrupted, or merged into a longer request) are ignored.
                        TurnStreamServerEvent::Token { turn_id, .. } | TurnStreamServerEvent::Done { turn_id }
                            if turn_id != current_turn_id(&device_label, turn_counter) => {}
                        TurnStreamServerEvent::Token { text, .. } => answer.push_str(&text),
                        TurnStreamServerEvent::Done { .. } => {
                            let text = std::mem::take(&mut answer);
                            if !awaiting_answer {
                                continue;
                            }
                            awaiting_answer = false;
                            if text.trim().is_empty() {
                                record_pod_bridge_turn("no_answer");
                                if std::mem::take(&mut barge_in_turn) {
                                    record_pod_bridge_barge_in("empty");
                                }
                                turn.listen();
                                push(GatewayToPod::Led { state: LedState::Listening });
                                continue;
                            }
                            let speech = Arc::clone(&speech);
                            let spoken_tx = spoken_tx.clone();
                            let for_turn = turn_counter;
                            tokio::task::spawn_blocking(move || {
                                let _ = spoken_tx.blocking_send((for_turn, speech.synthesize(&text)));
                            });
                        }
                        TurnStreamServerEvent::Error { message, .. } => {
                            warn!(device = %device_label, %message, "backend turn error");
                            record_pod_bridge_turn("backend_error");
                            barge_in_turn = false;
                            answer.clear();
                            turn.listen();
                            push(GatewayToPod::Led { state: LedState::Listening });
                        }
                        TurnStreamServerEvent::FrontendSkillIntent(intent) => {
                            // Pods have no local skills; tell the backend so it can answer.
                            let result = TurnStreamClientMessage::FrontendSkillResult {
                                turn_id: intent.turn_id,
                                intent_id: intent.intent,
                                result: FrontendSkillResultRequest {
                                    status: "error".to_string(),
                                    user_text: intent.user_text,
                                    structured_result_context: None,
                                    error: Some("not available on room pods".to_string()),
                                },
                            };
                            backend_tx.send(Message::Text(serde_json::to_string(&result)?)).await?;
                        }
                        TurnStreamServerEvent::HelpRaised { spoken, .. } => {
                            let speech = Arc::clone(&speech);
                            let spoken_tx = spoken_tx.clone();
                            let for_turn = turn_counter;
                            tokio::task::spawn_blocking(move || {
                                let _ = spoken_tx.blocking_send((for_turn, speech.synthesize(&spoken)));
                            });
                        }
                        TurnStreamServerEvent::PartialTranscript { .. }
                        | TurnStreamServerEvent::IntentUpdate { .. } => {}
                    }
                }
                spoken = spoken_rx.recv() => {
                    let Some((for_turn, spoken)) = spoken else { continue };
                    if for_turn != turn_counter {
                        // Speech for a turn the guest has since interrupted.
                        continue;
                    }
                    match spoken {
                        Ok(pcm) => {
                            record_pod_bridge_turn("answered");
                            if std::mem::take(&mut barge_in_turn) {
                                record_pod_bridge_barge_in("turn");
                            }
                            if let Some(task) = speaking.take() {
                                task.abort();
                            }
                            backend_tx
                                .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::PlaybackStarted)?))
                                .await?;
                            answer_playback = Playback::started(&pcm);
                            // The pod returns to listening on its own when playback ends.
                            let out = out_tx.clone();
                            let label = device_label.clone();
                            let played = played_tx.clone();
                            playback += 1;
                            let this_playback = playback;
                            speaking = Some(tokio::spawn(async move {
                                speak(out, label, pcm).await;
                                let _ = played.send(this_playback).await;
                            }));
                        }
                        Err(error) => {
                            warn!(device = %device_label, %error, "speech synthesis failed");
                            record_pod_bridge_turn("speech_failed");
                            push(GatewayToPod::Led { state: LedState::Listening });
                        }
                    }
                    record_pod_bridge_turn_duration(turn_started_at.elapsed());
                    // A half-duplex pod's mic is off while it plays, so listen now.
                    // A full-duplex pod keeps answering (interruptible) until playback ends.
                    if !full_duplex || speaking.is_none() {
                        turn.listen();
                    }
                }
                played = played_rx.recv() => {
                    // A stale signal from a replaced answer is ignored.
                    if played == Some(playback) && speaking.take().is_some() {
                        backend_tx
                            .send(Message::Text(serde_json::to_string(&TurnStreamClientMessage::PlaybackFinished)?))
                            .await?;
                        if full_duplex {
                            // The pod is still playing what it queued; listen once it is quiet.
                            speaker_tail.answer_sent();
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    record_pod_disconnect(&device_label);
    if let Some(task) = speaking.take() {
        task.abort();
    }
    drop(out_tx);
    let _ = writer.await;
    let _ = backend_tx.close().await;
    result
}

fn current_turn_id(device_label: &str, turn_counter: u64) -> String {
    format!("{device_label}-{turn_counter}")
}

fn turn_start(device_label: &str, turn_counter: u64) -> Result<String, serde_json::Error> {
    serde_json::to_string(&TurnStreamClientMessage::TurnStart {
        session_id: device_label.to_string(),
        device_id: None,
        turn_id: current_turn_id(device_label, turn_counter),
        supported_frontend_intents: Vec::new(),
        schema_version: None,
    })
}

/// Echo-cancelled PCM16 LE microphone bytes for one frame, and the mean
/// absolute level of the echo that was removed.
fn cancel_echo(canceller: &mut EchoCanceller, mic: &[u8], reference: &[u8]) -> (Vec<u8>, i32) {
    let samples = |bytes: &[u8]| -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect()
    };
    let heard = samples(mic);
    let cleaned = canceller.process(&heard, &samples(reference));
    let removed: i64 = heard
        .iter()
        .zip(&cleaned)
        .map(|(&h, &c)| (i64::from(h) - i64::from(c)).abs())
        .sum();
    let echo_level =
        i32::try_from(removed / i64::try_from(heard.len().max(1)).unwrap_or(1)).unwrap_or(i32::MAX);
    (
        cleaned.into_iter().flat_map(i16::to_le_bytes).collect(),
        echo_level,
    )
}

/// Frames a full-duplex pod's speaker must stay quiet before its answer counts
/// as finished (400 ms): the pod keeps playing what it had queued after the
/// bridge has sent the last chunk, and pauses between sentences are shorter.
const SPEAKER_QUIET_FRAMES: usize = 10;
/// Reference level (mean absolute) below which the speaker counts as quiet.
const SPEAKER_QUIET_LEVEL: i32 = 20;

/// Waits for a full-duplex pod to finish playing an answer that has been sent.
#[derive(Default)]
struct SpeakerTail {
    waiting: bool,
    quiet_frames: usize,
}

impl SpeakerTail {
    fn answer_sent(&mut self) {
        self.waiting = true;
        self.quiet_frames = 0;
    }

    /// True while a sent answer is still playing on the pod.
    fn waiting(&self) -> bool {
        self.waiting
    }

    /// Stops waiting; true when the pod was still playing a sent answer.
    fn cancel(&mut self) -> bool {
        std::mem::take(&mut self.waiting)
    }

    /// Takes the speaker level of one frame; true once, when the speaker has
    /// gone quiet after an answer.
    fn speaker_frame(&mut self, level: i32) -> bool {
        if !self.waiting {
            return false;
        }
        self.quiet_frames = if level < SPEAKER_QUIET_LEVEL {
            self.quiet_frames + 1
        } else {
            0
        };
        if self.quiet_frames < SPEAKER_QUIET_FRAMES {
            return false;
        }
        self.waiting = false;
        true
    }
}

/// Mean absolute levels of several signals over one second of frames, for
/// tuning echo cancellation from the debug log.
#[derive(Default)]
struct LevelMeter {
    sums: [i64; 3],
    frames: u32,
}

impl LevelMeter {
    /// Adds one frame of each signal; returns the averages once a second is full.
    fn add(&mut self, frames: [&[u8]; 3]) -> Option<[i32; 3]> {
        for (sum, frame) in self.sums.iter_mut().zip(frames) {
            *sum += i64::from(mean_abs_level(frame));
        }
        self.frames += 1;
        if self.frames < 25 {
            return None;
        }
        let frames = i64::from(self.frames);
        let averages = self
            .sums
            .map(|sum| i32::try_from(sum / frames).unwrap_or(i32::MAX));
        *self = Self::default();
        Some(averages)
    }
}

/// An answer's playback: when it started and how long it lasts.
struct Playback {
    started_at: Instant,
    length: Duration,
}

impl Playback {
    fn started(pcm: &[u8]) -> Self {
        Self {
            started_at: Instant::now(),
            length: frame_time(pcm),
        }
    }

    /// The share of the answer the pod had played by now (0 to 1).
    fn spoken_ratio(&self) -> f32 {
        if self.length.is_zero() {
            return 1.0;
        }
        (self.started_at.elapsed().as_secs_f32() / self.length.as_secs_f32()).clamp(0.0, 1.0)
    }
}

/// Tells the backend how much of the answer was heard before it was cut off.
fn interrupted(playback: &Playback) -> Result<String, serde_json::Error> {
    serde_json::to_string(&TurnStreamClientMessage::PlaybackInterrupted {
        spoken_ratio: playback.spoken_ratio(),
    })
}

/// Audio time of one PCM16 frame at 16 kHz.
fn frame_time(frame: &[u8]) -> Duration {
    let samples = u64::try_from(frame.len() / 2).unwrap_or(0);
    Duration::from_micros(samples * 1_000_000 / SAMPLE_RATE_HZ)
}

/// Send an answer at the pace the pod plays it, a few chunks ahead.
async fn speak(out: mpsc::Sender<GatewayToPod>, device_label: String, pcm: Vec<u8>) {
    if out
        .send(GatewayToPod::Led {
            state: LedState::Speaking,
        })
        .await
        .is_err()
    {
        return;
    }
    let started = tokio::time::Instant::now();
    for (index, chunk) in pcm.chunks(POD_AUDIO_CHUNK_BYTES).enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        if index >= POD_AUDIO_LEAD_CHUNKS {
            tokio::time::sleep_until(
                started + POD_AUDIO_CHUNK_TIME * (index - POD_AUDIO_LEAD_CHUNKS),
            )
            .await;
        }
        record_pod_tts_chunk(&device_label, chunk.len());
        let audio = GatewayToPod::Audio {
            payload: AudioPayload(chunk.to_vec()),
        };
        if out.send(audio).await.is_err() {
            return;
        }
    }
}

/// Handshake callback that keeps the pod's bearer token.
struct BearerCapture {
    bearer: Arc<std::sync::Mutex<Option<String>>>,
}

impl Callback for BearerCapture {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let token = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::to_string);
        if let Ok(mut slot) = self.bearer.lock() {
            *slot = token;
        }
        Ok(response)
    }
}

enum BackendRefusal {
    Unauthorized,
    Unavailable(String),
}

type BackendSocket = WebSocketStream<Box<dyn Stream>>;

/// Object-safe alias for a plain or TLS TCP stream.
trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

async fn connect_backend(
    settings: &BridgeSettings,
    bearer: Option<&str>,
) -> Result<BackendSocket, BackendRefusal> {
    let mut request = settings
        .backend_url
        .as_str()
        .into_client_request()
        .map_err(|error| BackendRefusal::Unavailable(error.to_string()))?;
    if let Some(token) = bearer {
        let value = format!("Bearer {token}")
            .parse()
            .map_err(|_| BackendRefusal::Unauthorized)?;
        request.headers_mut().insert("authorization", value);
    }
    let uri = request.uri().clone();
    let host = uri
        .host()
        .ok_or_else(|| BackendRefusal::Unavailable("backend url has no host".to_string()))?
        .to_string();
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|error| BackendRefusal::Unavailable(error.to_string()))?;
    let stream: Box<dyn Stream> = if secure {
        let config = settings.backend_tls.clone().ok_or_else(|| {
            BackendRefusal::Unavailable("wss backend needs the property CA".to_string())
        })?;
        let name = core_tls::rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|error| BackendRefusal::Unavailable(error.to_string()))?;
        let tls = core_tls::TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .map_err(|error| BackendRefusal::Unavailable(error.to_string()))?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };
    match tokio_tungstenite::client_async(request, stream).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response))
            if response.status().as_u16() == 401 =>
        {
            Err(BackendRefusal::Unauthorized)
        }
        Err(error) => Err(BackendRefusal::Unavailable(error.to_string())),
    }
}

async fn send_pod<S>(sink: &mut S, message: &GatewayToPod) -> Result<(), DynError>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(Message::Text(serde_json::to_string(message)?))
        .await
        .map_err(Into::into)
}

fn error_message(code: &str, message: &str) -> GatewayToPod {
    GatewayToPod::Error {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn truncate(message: &str) -> String {
    if message.chars().count() > 200 {
        format!(
            "{}... (truncated)",
            message.chars().take(200).collect::<String>()
        )
    } else {
        message.to_string()
    }
}

/// What the bridge should do with a pod frame.
#[derive(Debug, PartialEq, Eq)]
pub enum TurnStep {
    /// Speech began: open a turn and send these earlier frames first.
    Start(Vec<Vec<u8>>),
    Audio(Vec<u8>),
    /// Speech ended: close the turn.
    Done,
    /// Speech began while an answer was pending or playing: stop the answer,
    /// then open a turn with these earlier frames first.
    BargeIn(Vec<Vec<u8>>),
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Speaking,
    /// Waiting for the answer and while it plays. Only a full-duplex pod
    /// (echo-cancelled audio) can interrupt it by speaking.
    Answering,
}

/// Pods send 16 kHz mono PCM16.
const SAMPLE_RATE_HZ: u64 = 16_000;

/// Finds where speech starts and stops in a stream of PCM frames.
///
/// Time is measured in audio (samples / 16 kHz), not wall-clock time, so a
/// burst of buffered frames is judged exactly like frames arriving live.
pub struct TurnDetector {
    settings: VadSettings,
    phase: Phase,
    barge_in: bool,
    loud_frames: usize,
    preroll: std::collections::VecDeque<Vec<u8>>,
    clock: Duration,
    started_at: Duration,
    last_voice_at: Duration,
}

impl TurnDetector {
    pub fn new(settings: VadSettings) -> Self {
        Self {
            settings,
            phase: Phase::Idle,
            barge_in: false,
            loud_frames: 0,
            preroll: std::collections::VecDeque::new(),
            clock: Duration::ZERO,
            started_at: Duration::ZERO,
            last_voice_at: Duration::ZERO,
        }
    }

    /// Let speech interrupt answers (full-duplex pods, echo-cancelled audio).
    pub fn allow_barge_in(&mut self, allowed: bool) {
        self.barge_in = allowed;
    }

    /// Feed one frame of PCM16 LE audio.
    pub fn push(&mut self, frame: &[u8]) -> Vec<TurnStep> {
        self.push_with_echo(frame, 0)
    }

    /// Feed one frame of echo-cancelled audio with the level of the echo that
    /// was predicted (and removed) for it.
    pub fn push_with_echo(&mut self, frame: &[u8], echo_level: i32) -> Vec<TurnStep> {
        let samples = u64::try_from(frame.len() / 2).unwrap_or(0);
        self.clock += Duration::from_micros(samples * 1_000_000 / SAMPLE_RATE_HZ);
        let now = self.clock;
        let level = mean_abs_level(frame);
        let loud = level >= i32::from(self.settings.start_level);
        match self.phase {
            Phase::Answering if !self.barge_in => Vec::new(),
            Phase::Answering => {
                let barge_in = &self.settings.barge_in;
                let interrupting = level >= i32::from(barge_in.start_level)
                    && level as f32 >= barge_in.echo_ratio * echo_level as f32;
                if !self.onset(frame, interrupting, self.settings.barge_in.start_frames) {
                    return Vec::new();
                }
                self.begin(now);
                vec![TurnStep::BargeIn(self.preroll.drain(..).collect())]
            }
            Phase::Idle => {
                if !self.onset(frame, loud, self.settings.start_frames) {
                    return Vec::new();
                }
                self.begin(now);
                vec![TurnStep::Start(self.preroll.drain(..).collect())]
            }
            Phase::Speaking => {
                if loud {
                    self.last_voice_at = now;
                }
                let mut steps = vec![TurnStep::Audio(frame.to_vec())];
                let quiet_for = now.saturating_sub(self.last_voice_at);
                let spoken_for = now.saturating_sub(self.started_at);
                if quiet_for >= self.settings.end_silence || spoken_for >= self.settings.max_turn {
                    self.phase = Phase::Answering;
                    self.loud_frames = 0;
                    self.preroll.clear();
                    steps.push(TurnStep::Done);
                }
                steps
            }
        }
    }

    /// Keeps the frame for the preroll; true once `needed` loud frames ran in a row.
    fn onset(&mut self, frame: &[u8], loud: bool, needed: usize) -> bool {
        self.preroll.push_back(frame.to_vec());
        while self.preroll.len() > self.settings.preroll_frames.max(1) {
            self.preroll.pop_front();
        }
        self.loud_frames = if loud { self.loud_frames + 1 } else { 0 };
        self.loud_frames >= needed.max(1)
    }

    fn begin(&mut self, now: Duration) {
        self.phase = Phase::Speaking;
        self.started_at = now;
        self.last_voice_at = now;
        self.loud_frames = 0;
    }

    /// The answer finished (or failed); listen for the next request.
    pub fn listen(&mut self) {
        self.phase = Phase::Idle;
        self.loud_frames = 0;
        self.preroll.clear();
    }

    /// Button press: drop the current turn. True when a turn was open.
    pub fn cancel(&mut self) -> bool {
        let open = self.phase != Phase::Idle;
        self.listen();
        open
    }
}

fn mean_abs_level(frame: &[u8]) -> i32 {
    let samples = frame.len() / 2;
    if samples == 0 {
        return 0;
    }
    let total: i64 = frame
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i64::from(i16::from_le_bytes(*pair)).abs())
        .sum();
    i32::try_from(total / i64::try_from(samples).unwrap_or(1)).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{BargeInSettings, SpeakerTail, TurnDetector, TurnStep, VadSettings};
    use std::time::Duration;

    /// 40 ms at 16 kHz.
    fn frame(level: i16) -> Vec<u8> {
        (0..640).flat_map(|_| level.to_le_bytes()).collect()
    }

    fn settings() -> VadSettings {
        VadSettings {
            start_level: 1_000,
            start_frames: 2,
            end_silence: Duration::from_millis(200),
            max_turn: Duration::from_secs(2),
            preroll_frames: 2,
            barge_in: BargeInSettings {
                start_level: 3_000,
                start_frames: 2,
                echo_ratio: 1.5,
            },
        }
    }

    /// A detector whose turn has ended and whose answer is pending or playing.
    fn answering(barge_in: bool) -> TurnDetector {
        let mut detector = TurnDetector::new(settings());
        detector.allow_barge_in(barge_in);
        let _ = detector.push(&frame(5_000));
        let _ = detector.push(&frame(5_000));
        for _ in 0..5 {
            let _ = detector.push(&frame(0));
        }
        detector
    }

    #[test]
    fn speech_during_an_answer_interrupts_it_and_keeps_the_first_words() {
        let mut detector = answering(true);
        assert!(detector.push(&frame(0)).is_empty());
        assert!(detector.push(&frame(6_000)).is_empty());
        let steps = detector.push(&frame(6_000));
        assert!(
            matches!(&steps[..], [TurnStep::BargeIn(preroll)] if preroll.len() == 2),
            "{steps:?}"
        );
        assert_eq!(
            detector.push(&frame(6_000)),
            vec![TurnStep::Audio(frame(6_000))]
        );
    }

    #[test]
    fn leftover_echo_during_an_answer_does_not_interrupt_it() {
        // Above the turn-start level, below the barge-in level.
        let mut detector = answering(true);
        for _ in 0..20 {
            assert!(detector.push(&frame(2_000)).is_empty());
        }
    }

    #[test]
    fn speech_that_does_not_stand_out_from_the_echo_does_not_interrupt() {
        // Loud enough, but the pod's own voice is predicted almost as loud.
        let mut detector = answering(true);
        for _ in 0..20 {
            assert!(detector.push_with_echo(&frame(6_000), 5_000).is_empty());
        }
    }

    #[test]
    fn speech_well_above_the_echo_interrupts() {
        let mut detector = answering(true);
        let _ = detector.push_with_echo(&frame(6_000), 1_000);
        let steps = detector.push_with_echo(&frame(6_000), 1_000);
        assert!(matches!(&steps[..], [TurnStep::BargeIn(_)]), "{steps:?}");
    }

    #[test]
    fn the_pods_buffered_answer_is_not_heard_as_a_new_request() {
        let mut tail = SpeakerTail::default();
        tail.answer_sent();
        // The pod keeps playing what it had queued, with a short pause inside.
        for level in [900, 800, 5, 5, 700, 600] {
            assert!(!tail.speaker_frame(level), "still playing at {level}");
        }
        let quiet: Vec<bool> = (0..10).map(|_| tail.speaker_frame(3)).collect();
        assert_eq!(
            quiet.iter().filter(|&&done| done).count(),
            1,
            "done once: {quiet:?}"
        );
        assert_eq!(quiet.last(), Some(&true));
    }

    #[test]
    fn no_answer_sent_means_nothing_to_wait_for() {
        let mut tail = SpeakerTail::default();
        assert!(!tail.speaker_frame(3));
    }

    #[test]
    fn a_half_duplex_pod_is_never_interrupted_by_speech() {
        let mut detector = answering(false);
        for _ in 0..20 {
            assert!(detector.push(&frame(9_000)).is_empty());
        }
    }

    #[test]
    fn an_interrupting_turn_ends_like_any_other() {
        let mut detector = answering(true);
        let _ = detector.push(&frame(6_000));
        let _ = detector.push(&frame(6_000));
        let mut done = false;
        for _ in 0..5 {
            done = detector.push(&frame(0)).last() == Some(&TurnStep::Done);
        }
        assert!(done);
    }

    #[test]
    fn a_turn_starts_after_sustained_speech_and_includes_the_preroll() {
        let mut detector = TurnDetector::new(settings());
        assert!(detector.push(&frame(0)).is_empty());
        assert!(detector.push(&frame(5_000)).is_empty());
        let steps = detector.push(&frame(5_000));
        assert!(matches!(&steps[..], [TurnStep::Start(preroll)] if preroll.len() == 2));
    }

    #[test]
    fn a_turn_ends_after_silence_and_mutes_until_answered() {
        let mut detector = TurnDetector::new(settings());
        let _ = detector.push(&frame(5_000));
        let _ = detector.push(&frame(5_000));
        // 200 ms of silence is five 40 ms frames.
        for _ in 0..4 {
            assert_eq!(detector.push(&frame(0)), vec![TurnStep::Audio(frame(0))]);
        }
        assert_eq!(detector.push(&frame(0)).last(), Some(&TurnStep::Done));
        assert!(
            detector.push(&frame(9_000)).is_empty(),
            "muted while answering"
        );
        detector.listen();
        let _ = detector.push(&frame(9_000));
        assert!(matches!(
            detector.push(&frame(9_000)).first(),
            Some(TurnStep::Start(_))
        ));
    }

    #[test]
    fn a_long_turn_is_cut_at_the_limit() {
        let mut detector = TurnDetector::new(settings());
        let _ = detector.push(&frame(5_000));
        let _ = detector.push(&frame(5_000));
        // 2 s of continuous speech is 50 frames.
        let mut done = false;
        for _ in 0..50 {
            done = detector.push(&frame(5_000)).last() == Some(&TurnStep::Done);
        }
        assert!(done);
    }

    #[test]
    fn quiet_rooms_never_start_a_turn() {
        let mut detector = TurnDetector::new(settings());
        for _ in 0..50 {
            assert!(detector.push(&frame(200)).is_empty());
        }
    }
}
