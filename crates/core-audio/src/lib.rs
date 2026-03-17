//! Audio capture and playback abstraction (16kHz mono 16-bit PCM).
//! Phase 1 will implement desktop mic and buffer interfaces.

pub mod capture;
pub mod cpal_capture;
pub mod fake_capture;
pub mod format;

pub use capture::{AudioCapture, CaptureError};
pub use cpal_capture::CpalCapture;
pub use fake_capture::FakeCapture;

/// Target sample rate for pipeline.
pub const SAMPLE_RATE: u32 = 16_000;
/// Channels: mono.
pub const CHANNELS: u16 = 1;
