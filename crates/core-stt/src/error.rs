//! STT errors.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SttError {
    #[error("STT not initialized")]
    NotInitialized,
    #[error("Whisper error: {0}")]
    Whisper(String),
}
