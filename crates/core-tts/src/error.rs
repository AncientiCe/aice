//! TTS errors.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TtsError {
    #[error("TTS synthesis failed: {0}")]
    Synthesis(String),
}
