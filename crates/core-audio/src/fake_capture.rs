//! Test double: yields predefined PCM chunks.

use crate::capture::{AudioCapture, CaptureError};
use std::time::Duration;

/// Fake capture that yields chunks from a queue. Used in tests.
pub struct FakeCapture {
    chunks: std::vec::IntoIter<Vec<i16>>,
}

impl FakeCapture {
    /// Build a fake capture that will yield the given chunks in order.
    pub fn new(chunks: Vec<Vec<i16>>) -> Self {
        Self {
            chunks: chunks.into_iter(),
        }
    }

    /// One chunk of PCM (e.g. for "desktop mic -> STT" test).
    pub fn single_chunk(samples: Vec<i16>) -> Self {
        Self::new(vec![samples])
    }
}

impl AudioCapture for FakeCapture {
    fn read_chunk(&mut self, _timeout: Duration) -> Result<Vec<i16>, CaptureError> {
        Ok(self.chunks.next().unwrap_or_default())
    }
}
