//! Streaming text-to-speech adapter (e.g. Piper).

pub mod error;
pub mod fake;
pub mod piper;

pub use error::TtsError;
pub use fake::FakeTtsSink;
pub use piper::PiperTtsSink;
