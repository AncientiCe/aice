//! Audio capture from pod gateway ingest channel.

use core_audio::{AudioCapture, CaptureError};
use pod_gateway::PodIngestEvent;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::time::Duration;

/// Captures audio from the pod gateway ingest channel (one process = gateway + pipeline).
pub struct PodIngestCapture {
    rx: Receiver<PodIngestEvent>,
    last_device_id: Mutex<Option<String>>,
}

impl PodIngestCapture {
    pub fn new(rx: Receiver<PodIngestEvent>) -> Self {
        Self {
            rx,
            last_device_id: Mutex::new(None),
        }
    }
}

impl AudioCapture for PodIngestCapture {
    fn read_chunk(&mut self, timeout: Duration) -> Result<Vec<i16>, CaptureError> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => {
                *self
                    .last_device_id
                    .lock()
                    .map_err(|_| CaptureError::Device("lock poisoned".into()))? =
                    Some(event.device_id);
                Ok(event.pcm)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(CaptureError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(CaptureError::Timeout),
        }
    }

    fn source_device_id(&self) -> Option<String> {
        self.last_device_id.lock().ok().and_then(|g| g.clone())
    }
}
