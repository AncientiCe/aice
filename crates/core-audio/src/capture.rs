//! Audio capture abstraction: desktop mic or test source.

use crate::format::description;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("capture not started")]
    NotStarted,
    #[error("device error: {0}")]
    Device(String),
    #[error("timeout waiting for audio")]
    Timeout,
}

/// Source of PCM audio chunks (16 kHz, mono, 16-bit).
/// Implemented by desktop mic capture or test doubles.
pub trait AudioCapture: Send {
    /// Read one chunk of PCM samples. Blocks until data is available or timeout.
    /// Returns empty vec on end-of-stream or timeout.
    fn read_chunk(&mut self, timeout: Duration) -> Result<Vec<i16>, CaptureError>;

    /// Sample rate and channel count: (rate, channels).
    fn format(&self) -> (u32, u16) {
        description()
    }

    /// When set, the pipeline should send TTS output to this device (e.g. pod device_id).
    /// Default is None (local playback).
    fn source_device_id(&self) -> Option<String> {
        None
    }
}
