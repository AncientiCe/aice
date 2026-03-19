//! Piper-based TTS adapter backed by the local `piper` CLI.

use async_trait::async_trait;
use core_observability::{record_error, record_stage_duration, Stage};
use core_orchestrator::TtsSink;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;
use tempfile::Builder;

use crate::TtsError;

fn le_u16(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 2 {
        return None;
    }
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn normalize_wav_to_pcm16k_mono(wav_bytes: &[u8]) -> Result<Vec<u8>, TtsError> {
    if wav_bytes.len() < 44 || &wav_bytes[0..4] != b"RIFF" || &wav_bytes[8..12] != b"WAVE" {
        return Err(TtsError::Synthesis("invalid wav header".to_string()));
    }

    let mut offset = 12usize;
    let mut fmt_audio_format: Option<u16> = None;
    let mut fmt_channels: Option<u16> = None;
    let mut fmt_sample_rate: Option<u32> = None;
    let mut fmt_bits_per_sample: Option<u16> = None;
    let mut data_chunk: Option<&[u8]> = None;

    while offset + 8 <= wav_bytes.len() {
        let id = &wav_bytes[offset..offset + 4];
        let size = le_u32(&wav_bytes[offset + 4..offset + 8])
            .ok_or_else(|| TtsError::Synthesis("invalid wav chunk size".to_string()))?
            as usize;
        let data_start = offset + 8;
        let data_end = data_start.saturating_add(size);
        if data_end > wav_bytes.len() {
            return Err(TtsError::Synthesis(
                "wav chunk exceeds file size".to_string(),
            ));
        }
        let chunk_data = &wav_bytes[data_start..data_end];

        if id == b"fmt " {
            if chunk_data.len() < 16 {
                return Err(TtsError::Synthesis("wav fmt chunk too small".to_string()));
            }
            fmt_audio_format = le_u16(&chunk_data[0..2]);
            fmt_channels = le_u16(&chunk_data[2..4]);
            fmt_sample_rate = le_u32(&chunk_data[4..8]);
            fmt_bits_per_sample = le_u16(&chunk_data[14..16]);
        } else if id == b"data" {
            data_chunk = Some(chunk_data);
            break;
        }

        // RIFF chunks are padded to even sizes.
        offset = data_end + (size % 2);
    }

    let audio_format =
        fmt_audio_format.ok_or_else(|| TtsError::Synthesis("wav missing fmt chunk".to_string()))?;
    let channels =
        fmt_channels.ok_or_else(|| TtsError::Synthesis("wav missing channels".to_string()))?;
    let sample_rate = fmt_sample_rate
        .ok_or_else(|| TtsError::Synthesis("wav missing sample rate".to_string()))?;
    let bits_per_sample = fmt_bits_per_sample
        .ok_or_else(|| TtsError::Synthesis("wav missing bits_per_sample".to_string()))?;
    let data =
        data_chunk.ok_or_else(|| TtsError::Synthesis("wav missing data chunk".to_string()))?;

    if audio_format != 1 || bits_per_sample != 16 {
        return Err(TtsError::Synthesis(format!(
            "unsupported wav format: audio_format={audio_format} bits_per_sample={bits_per_sample}"
        )));
    }
    if channels != 1 && channels != 2 {
        return Err(TtsError::Synthesis(format!(
            "unsupported wav channels: {channels}"
        )));
    }
    if data.len() < 2 {
        return Ok(Vec::new());
    }

    let mut mono: Vec<i16> = Vec::new();
    if channels == 1 {
        mono.reserve(data.len() / 2);
        for frame in data.chunks_exact(2) {
            mono.push(i16::from_le_bytes([frame[0], frame[1]]));
        }
    } else {
        // Downmix stereo PCM16 by averaging left/right.
        mono.reserve(data.len() / 4);
        for frame in data.chunks_exact(4) {
            let l = i16::from_le_bytes([frame[0], frame[1]]) as i32;
            let r = i16::from_le_bytes([frame[2], frame[3]]) as i32;
            mono.push(((l + r) / 2) as i16);
        }
    }

    const TARGET_RATE: u32 = 16_000;
    let mut resampled: Vec<i16> = if sample_rate == TARGET_RATE {
        mono
    } else if mono.is_empty() {
        Vec::new()
    } else {
        // Linear interpolation resampler; good enough for voice transport.
        let out_len = ((mono.len() as u64 * TARGET_RATE as u64 + (sample_rate as u64 / 2))
            / sample_rate as u64)
            .max(1) as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = (i as f64) * (sample_rate as f64) / (TARGET_RATE as f64);
            let idx = pos.floor() as usize;
            let frac = pos - idx as f64;
            let a = mono[idx.min(mono.len() - 1)] as f64;
            let b = mono[(idx + 1).min(mono.len() - 1)] as f64;
            out.push((a + (b - a) * frac) as i16);
        }
        out
    };

    // Pod speaker is tiny and tends to sound harsh; apply a mild low-pass
    // after resampling to improve intelligibility ("less squeaky" voice).
    if !resampled.is_empty() {
        const LP_ALPHA_Q15: i32 = 9830; // ~0.30 smoothing
        let mut state_q15 = (resampled[0] as i64) << 15;
        for s in &mut resampled {
            let x_q15 = (*s as i64) << 15;
            state_q15 += ((x_q15 - state_q15) * LP_ALPHA_Q15 as i64) >> 15;
            *s = (state_q15 >> 15) as i16;
        }
    }

    let mut pcm = Vec::with_capacity(resampled.len() * 2);
    for s in resampled {
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    Ok(pcm)
}

/// TTS adapter that buffers text and on flush synthesizes + plays audio.
pub struct PiperTtsSink {
    buffer: String,
    model_path: PathBuf,
    config_path: Option<PathBuf>,
    piper_bin: String,
    playback_tx: mpsc::Sender<PlaybackCommand>,
}

enum PlaybackCommand {
    Play(Vec<u8>),
    Stop,
}

impl PiperTtsSink {
    /// Create a new Piper TTS sink using model path.
    /// Environment variable `PIPER_BIN` can override CLI binary path.
    pub fn new(model_path: &Path) -> Result<Self, TtsError> {
        if !model_path.exists() {
            return Err(TtsError::Synthesis(format!(
                "piper model not found: {}",
                model_path.display()
            )));
        }
        let config_candidate = format!("{}.json", model_path.display());
        let config_path = {
            let candidate = PathBuf::from(&config_candidate);
            if candidate.exists() {
                Some(candidate)
            } else {
                None
            }
        };
        Ok(Self {
            buffer: String::new(),
            model_path: model_path.to_path_buf(),
            config_path,
            piper_bin: std::env::var("PIPER_BIN").unwrap_or_else(|_| "piper".to_string()),
            playback_tx: Self::spawn_playback_worker(),
        })
    }

    fn synthesize_to_wav(&self, text: &str, wav_path: &Path) -> Result<(), TtsError> {
        let mut cmd = Command::new(&self.piper_bin);
        cmd.arg("--model").arg(&self.model_path);
        if let Some(config_path) = &self.config_path {
            cmd.arg("--config").arg(config_path);
        }
        cmd.arg("--output_file").arg(wav_path);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| TtsError::Synthesis(format!("failed to start piper: {e}")))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| TtsError::Synthesis(format!("failed to write piper stdin: {e}")))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|e| TtsError::Synthesis(format!("failed waiting for piper: {e}")))?;
        if !output.status.success() {
            return Err(TtsError::Synthesis(format!(
                "piper failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        if !wav_path.exists() {
            return Err(TtsError::Synthesis(
                "piper did not produce output wav".to_string(),
            ));
        }
        Ok(())
    }

    /// Synthesize text to normalized 16kHz mono PCM16 bytes. No playback.
    /// Used when sending TTS to a pod over the network.
    pub fn synthesize_to_pcm(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let dir = Builder::new()
            .prefix("aice-piper-")
            .tempdir()
            .map_err(|e| TtsError::Synthesis(format!("tempdir error: {e}")))?;
        let wav_path = dir.path().join("tts.wav");
        self.synthesize_to_wav(text, &wav_path)?;
        let mut f = std::fs::File::open(&wav_path)
            .map_err(|e| TtsError::Synthesis(format!("open wav: {e}")))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)
            .map_err(|e| TtsError::Synthesis(format!("read wav: {e}")))?;
        normalize_wav_to_pcm16k_mono(&bytes)
    }

    fn spawn_playback_worker() -> mpsc::Sender<PlaybackCommand> {
        let (tx, rx) = mpsc::channel::<PlaybackCommand>();
        std::thread::spawn(move || {
            let stream_and_handle: Option<(OutputStream, OutputStreamHandle)> =
                OutputStream::try_default().ok();
            let mut current_sink: Option<Sink> = None;

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    PlaybackCommand::Stop => {
                        if let Some(sink) = current_sink.take() {
                            sink.stop();
                        }
                    }
                    PlaybackCommand::Play(bytes) => {
                        if let Some(sink) = current_sink.take() {
                            sink.stop();
                        }
                        let Some((_, handle)) = stream_and_handle.as_ref() else {
                            continue;
                        };
                        let Ok(sink) = Sink::try_new(handle) else {
                            continue;
                        };
                        let Ok(source) = Decoder::new(Cursor::new(bytes)) else {
                            continue;
                        };
                        sink.append(source);
                        sink.play();
                        current_sink = Some(sink);
                    }
                }
            }
        });
        tx
    }

    fn play_wav_nonblocking(&self, wav_bytes: Vec<u8>) -> Result<(), TtsError> {
        self.playback_tx
            .send(PlaybackCommand::Play(wav_bytes))
            .map_err(|e| TtsError::Synthesis(format!("playback channel send failed: {e}")))
    }
}

#[async_trait]
impl TtsSink for PiperTtsSink {
    fn request_stop_playback(&mut self) {
        let _ = self.playback_tx.send(PlaybackCommand::Stop);
    }

    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.buffer.push_str(text);
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let t0 = Instant::now();
        let text = std::mem::take(&mut self.buffer);
        if text.trim().is_empty() {
            record_stage_duration(Stage::Tts, t0.elapsed());
            return Ok(());
        }

        let dir = Builder::new()
            .prefix("aice-piper-")
            .tempdir()
            .map_err(|e| TtsError::Synthesis(format!("tempdir error: {e}")))?;
        let wav_path = dir.path().join("tts.wav");

        self.synthesize_to_wav(&text, &wav_path).inspect_err(|_| {
            record_error("tts_synthesize");
        })?;
        let mut f = std::fs::File::open(&wav_path)
            .map_err(|e| TtsError::Synthesis(format!("open wav: {e}")))?;
        let mut wav_bytes = Vec::new();
        f.read_to_end(&mut wav_bytes)
            .map_err(|e| TtsError::Synthesis(format!("read wav: {e}")))?;
        self.play_wav_nonblocking(wav_bytes).inspect_err(|_| {
            record_error("tts_playback");
        })?;
        record_stage_duration(Stage::Tts, t0.elapsed());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    pub trait TestResultExt<T, E> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(value) => value,
                Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
            }
        }
    }
    use super::normalize_wav_to_pcm16k_mono;

    fn build_pcm16_wav(sample_rate: u32, channels: u16, samples_interleaved: &[i16]) -> Vec<u8> {
        let bits_per_sample: u16 = 16;
        let block_align: u16 = channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * block_align as u32;
        let data_size: u32 = (samples_interleaved.len() * 2) as u32;
        let riff_size: u32 = 4 + (8 + 16) + (8 + data_size);

        let mut out = Vec::with_capacity((riff_size + 8) as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_size.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits_per_sample.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_size.to_le_bytes());
        for s in samples_interleaved {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    fn decode_i16_le(bytes: &[u8]) -> Vec<i16> {
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
    fn normalize_wav_resamples_to_16k_mono() {
        let input_rate = 22050u32;
        let src: Vec<i16> = (0..2205).map(|i| ((i % 200) as i16) - 100).collect(); // ~100ms mono
        let wav = build_pcm16_wav(input_rate, 1, &src);

        let pcm = normalize_wav_to_pcm16k_mono(&wav).must();
        let out = decode_i16_le(&pcm);

        // 2205 samples at 22.05kHz is 100ms => expect ~1600 samples at 16kHz.
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn normalize_wav_downmixes_stereo_to_mono() {
        let input_rate = 16000u32;
        let mut interleaved = Vec::new();
        for _ in 0..160 {
            interleaved.push(1000i16); // L
            interleaved.push(-1000i16); // R
        }
        let wav = build_pcm16_wav(input_rate, 2, &interleaved);

        let pcm = normalize_wav_to_pcm16k_mono(&wav).must();
        let out = decode_i16_le(&pcm);
        assert_eq!(out.len(), 160);
        // Average of +1000 and -1000 should be around 0.
        assert!(out.iter().all(|s| s.abs() <= 1));
    }

    #[test]
    fn normalize_wav_reduces_harsh_high_frequency_edges() {
        // Alternating max-ish waveform at 16kHz is very harsh on the tiny pod speaker.
        let input_rate = 16000u32;
        let mut src = Vec::new();
        for i in 0..1600 {
            src.push(if i % 2 == 0 { 12000 } else { -12000 });
        }
        let wav = build_pcm16_wav(input_rate, 1, &src);

        let pcm = normalize_wav_to_pcm16k_mono(&wav).must();
        let out = decode_i16_le(&pcm);
        assert_eq!(out.len(), src.len());

        // Expect reduced edge energy (smoother output) for pod playback clarity.
        let in_edge = mean_abs_diff(&src);
        let out_edge = mean_abs_diff(&out);
        assert!(out_edge < in_edge);
    }
}
