//! Streaming speech-to-text adapter (e.g. whisper.cpp binding).

pub mod error;
pub mod fake;
pub mod whisper;

pub use error::SttError;
pub use fake::FakeSttStream;
pub use whisper::WhisperSttStream;
