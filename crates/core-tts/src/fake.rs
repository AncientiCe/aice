//! Fake TTS for tests; implements TtsSink with buffered text.

use async_trait::async_trait;
use core_orchestrator::TtsSink;

/// Fake TTS that buffers text (for tests).
pub struct FakeTtsSink {
    buffer: String,
}

impl FakeTtsSink {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    pub fn buffered(&self) -> &str {
        &self.buffer
    }
}

impl Default for FakeTtsSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TtsSink for FakeTtsSink {
    async fn push_text(
        &mut self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.buffer.push_str(text);
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}
