//! Whisper-based STT adapter (whisper-rs / whisper.cpp).

use crate::SttError;
use async_trait::async_trait;
#[cfg(feature = "whisper")]
use core_observability::record_error;
use core_observability::{record_stage_duration, Stage};
use core_orchestrator::SttStream;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
#[cfg(not(feature = "whisper"))]
use std::path::PathBuf;
#[cfg(not(feature = "whisper"))]
use std::process::Command;
use std::time::Instant;
#[cfg(not(feature = "whisper"))]
use tempfile::Builder;

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[cfg(not(feature = "whisper"))]
fn is_whisper_interrupted(status_code: Option<i32>, status_text: &str) -> bool {
    matches!(status_code, Some(-1073741510))
        || status_text.to_ascii_lowercase().contains("0xc000013a")
}

#[cfg(not(feature = "whisper"))]
fn is_whisper_access_violation(status_code: Option<i32>, status_text: &str) -> bool {
    matches!(status_code, Some(-1073741819))
        || status_text.to_ascii_lowercase().contains("0xc0000005")
}

fn should_skip_short_transcription(sample_count: usize) -> bool {
    sample_count < 800
}

#[cfg(feature = "whisper")]
fn native_decode_threads() -> i32 {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let threads = cpus.clamp(1, 8);
    threads as i32
}

/// STT adapter that buffers PCM and runs Whisper on flush.
pub struct WhisperSttStream {
    buffer: Vec<i16>,
    #[cfg(not(feature = "whisper"))]
    model_path: PathBuf,
    #[cfg(not(feature = "whisper"))]
    cli_bin: String,
    #[cfg(feature = "whisper")]
    native_ctx: whisper_rs::WhisperContext,
}

impl WhisperSttStream {
    /// Create a new Whisper STT stream loading the model at `model_path`.
    /// Uses native whisper-rs backend when compiled with feature `whisper`,
    /// otherwise falls back to whisper-cli subprocess execution.
    pub fn new(model_path: &Path) -> Result<Self, SttError> {
        #[cfg(not(feature = "whisper"))]
        let cli_bin =
            std::env::var("WHISPER_CLI_BIN").unwrap_or_else(|_| "whisper-cli".to_string());
        #[cfg(feature = "whisper")]
        whisper_rs::install_logging_hooks();
        #[cfg(feature = "whisper")]
        let native_ctx = whisper_rs::WhisperContext::new_with_params(
            model_path,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| SttError::Whisper(format!("failed to initialize whisper context: {e}")))?;

        Ok(Self {
            buffer: Vec::new(),
            #[cfg(not(feature = "whisper"))]
            model_path: model_path.to_path_buf(),
            #[cfg(not(feature = "whisper"))]
            cli_bin,
            #[cfg(feature = "whisper")]
            native_ctx,
        })
    }

    #[cfg(feature = "whisper")]
    fn flush_native(&mut self) -> Result<String, SttError> {
        if self.buffer.is_empty() || should_skip_short_transcription(self.buffer.len()) {
            return Ok(String::new());
        }
        let mut float_audio = vec![0.0_f32; self.buffer.len()];
        whisper_rs::convert_integer_to_float_audio(&self.buffer, &mut float_audio)
            .map_err(|e| SttError::Whisper(e.to_string()))?;
        let mut state = self
            .native_ctx
            .create_state()
            .map_err(|e| SttError::Whisper(e.to_string()))?;
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(native_decode_threads());
        params.set_language(Some("en"));
        params.set_detect_language(false);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_single_segment(true);
        params.set_token_timestamps(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        state.full(params, &float_audio).map_err(|e| {
            record_error("stt_flush");
            SttError::Whisper(e.to_string())
        })?;
        Ok(state
            .as_iter()
            .map(|s| {
                s.to_str_lossy()
                    .map(|text| text.into_owned())
                    .map_err(|e| SttError::Whisper(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(" ")
            .trim()
            .to_string())
    }

    #[cfg(not(feature = "whisper"))]
    fn flush_cli(&mut self) -> Result<String, SttError> {
        if self.buffer.is_empty() || should_skip_short_transcription(self.buffer.len()) {
            return Ok(String::new());
        }
        let dir = Builder::new()
            .prefix("aice-whisper-")
            .tempdir()
            .map_err(|e| SttError::Whisper(format!("tempdir: {e}")))?;
        let wav_path = dir.path().join("input.wav");
        let out_base = dir.path().join("out");
        let out_txt = dir.path().join("out.txt");

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav_path, spec)
            .map_err(|e| SttError::Whisper(e.to_string()))?;
        for sample in &self.buffer {
            writer
                .write_sample(*sample)
                .map_err(|e| SttError::Whisper(e.to_string()))?;
        }
        writer
            .finalize()
            .map_err(|e| SttError::Whisper(e.to_string()))?;

        let mut cmd = Command::new(&self.cli_bin);
        cmd.arg("-m")
            .arg(&self.model_path)
            .arg("-f")
            .arg(&wav_path)
            .arg("-otxt")
            .arg("-of")
            .arg(&out_base);
        #[cfg(windows)]
        {
            // Keep whisper-cli in its own process group so console Ctrl+C targets runner reliably.
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }

        let output = cmd
            .output()
            .map_err(|e| SttError::Whisper(format!("failed to run whisper-cli: {e}")))?;

        if !output.status.success() {
            let status_text = output.status.to_string();
            if is_whisper_interrupted(output.status.code(), &status_text) {
                return Err(SttError::Whisper(
                    "whisper-cli interrupted by console control event".to_string(),
                ));
            }
            if is_whisper_access_violation(output.status.code(), &status_text) {
                return Err(SttError::Whisper(
                    "whisper-cli access violation (0xc0000005)".to_string(),
                ));
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SttError::Whisper(format!(
                "whisper-cli exited with {}: {}",
                output.status, stderr
            )));
        }

        let transcript = std::fs::read_to_string(&out_txt)
            .map_err(|e| SttError::Whisper(format!("read transcript: {e}")))?;
        Ok(transcript.trim().to_string())
    }
}

#[async_trait]
impl SttStream for WhisperSttStream {
    async fn push_audio(
        &mut self,
        pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.buffer.extend_from_slice(pcm);
        Ok(())
    }

    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let t0 = Instant::now();
        let out = {
            #[cfg(feature = "whisper")]
            {
                self.flush_native()?
            }
            #[cfg(not(feature = "whisper"))]
            {
                self.flush_cli()?
            }
        };
        self.buffer.clear();
        record_stage_duration(Stage::Stt, t0.elapsed());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::should_skip_short_transcription;
    #[cfg(not(feature = "whisper"))]
    use super::{is_whisper_access_violation, is_whisper_interrupted};

    #[cfg(not(feature = "whisper"))]
    #[test]
    fn detects_windows_ctrl_c_exit_code() {
        assert!(is_whisper_interrupted(Some(-1073741510), ""));
        assert!(is_whisper_interrupted(None, "exit code: 0xC000013A"));
    }

    #[cfg(not(feature = "whisper"))]
    #[test]
    fn ignores_normal_error_codes() {
        assert!(!is_whisper_interrupted(Some(1), "exit code: 1"));
    }

    #[cfg(not(feature = "whisper"))]
    #[test]
    fn detects_whisper_access_violation_exit_code() {
        assert!(is_whisper_access_violation(Some(-1073741819), ""));
        assert!(is_whisper_access_violation(None, "exit code: 0xC0000005"));
    }

    #[test]
    fn short_audio_is_skipped_for_transcription() {
        assert!(should_skip_short_transcription(160));
        assert!(!should_skip_short_transcription(1600));
    }
}
