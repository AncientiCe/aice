//! Voice activity detection (speech start/end) and wake-word gating.

pub mod wake_word;

pub use wake_word::WakeWordGate;

/// Placeholder for VAD state; Phase 1 will integrate webrtc-vad or similar.
#[derive(Default)]
pub struct VadState;

impl VadState {
    pub fn new() -> Self {
        Self
    }
}
