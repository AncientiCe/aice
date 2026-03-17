//! Whisper-based STT adapter (whisper-rs / whisper.cpp).

use crate::SttError;
use async_trait::async_trait;
use core_observability::{record_stage_duration, Stage};
use core_orchestrator::SttStream;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tempfile::Builder;

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn is_whisper_interrupted(status_code: Option<i32>, status_text: &str) -> bool {
    matches!(status_code, Some(-1073741510))
        || status_text.to_ascii_lowercase().contains("0xc000013a")
}

fn is_whisper_access_violation(status_code: Option<i32>, status_text: &str) -> bool {
    matches!(status_code, Some(-1073741819))
        || status_text.to_ascii_lowercase().contains("0xc0000005")
}

fn should_skip_short_transcription(sample_count: usize) -> bool {
    sample_count < 800
}

/// STT adapter that buffers PCM and runs Whisper on flush.
pub struct WhisperSttStream {
    buffer: Vec<i16>,
    model_path: PathBuf,
    cli_bin: String,
    #[cfg(feature = "whisper")]
    native_ctx: Option<whisper_rs::WhisperContext>,
}

impl WhisperSttStream {
    /// Create a new Whisper STT stream loading the model at `model_path`.
    /// Uses native whisper-rs backend when compiled with feature `whisper`,
    /// otherwise falls back to whisper-cli subprocess execution.
    pub fn new(model_path: &Path) -> Result<Self, SttError> {
        let cli_bin =
            std::env::var("WHISPER_CLI_BIN").unwrap_or_else(|_| "whisper-cli".to_string());
        #[cfg(feature = "whisper")]
        let native_ctx = whisper_rs::WhisperContext::new_with_params(
            model_path,
            whisper_rs::WhisperContextParameters::default(),
        )
        .ok();

        Ok(Self {
            buffer: Vec::new(),
            model_path: model_path.to_path_buf(),
            cli_bin,
            #[cfg(feature = "whisper")]
            native_ctx,
        })
    }

    #[cfg(feature = "whisper")]
    fn flush_native(&mut self) -> Result<String, SttError> {
        let Some(ctx) = self.native_ctx.as_ref() else {
            return Err(SttError::NotInitialized);
        };
        if self.buffer.is_empty() || should_skip_short_transcription(self.buffer.len()) {
            return Ok(String::new());
        }
        let float_audio = whisper_rs::convert_integer_to_float_audio(&self.buffer);
        let mut state = ctx
            .create_state()
            .map_err(|e| SttError::Whisper(e.to_string()))?;
        let params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        state.full(params, &float_audio).map_err(|e| {
            record_error("stt_flush");
            SttError::Whisper(e.to_string())
        })?;
        Ok(state
            .as_iter()
            .map(|s| s.to_str_lossy().to_string())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string())
    }

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
                if self.native_ctx.is_some() {
                    self.flush_native()?
                } else {
                    self.flush_cli()?
                }
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
    use super::{
        is_whisper_access_violation, is_whisper_interrupted, should_skip_short_transcription,
    };

    #[test]
    fn detects_windows_ctrl_c_exit_code() {
        assert!(is_whisper_interrupted(Some(-1073741510), ""));
        assert!(is_whisper_interrupted(None, "exit code: 0xC000013A"));
    }

    #[test]
    fn ignores_normal_error_codes() {
        assert!(!is_whisper_interrupted(Some(1), "exit code: 1"));
    }

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
