//! Fake STT for tests; implements SttStream with configurable transcript.

use async_trait::async_trait;
use core_orchestrator::SttStream;

/// Fake STT that returns a fixed transcript on flush (for tests).
pub struct FakeSttStream {
    transcript: String,
}

impl FakeSttStream {
    pub fn new(transcript: &str) -> Self {
        Self {
            transcript: transcript.to_string(),
        }
    }
}

#[async_trait]
impl SttStream for FakeSttStream {
    async fn push_audio(
        &mut self,
        _pcm: &[i16],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn flush(&mut self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.transcript.clone())
    }
}
