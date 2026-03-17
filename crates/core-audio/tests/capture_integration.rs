//! Integration tests: desktop mic path (fake) produces PCM chunks for STT.

use core_audio::{AudioCapture, FakeCapture, SAMPLE_RATE};
use std::time::Duration;

#[test]
fn fake_capture_yields_pcm_chunk_for_stt() {
    let pcm = vec![0_i16; 1600]; // 100 ms at 16 kHz
    let mut cap = FakeCapture::single_chunk(pcm.clone());
    let (rate, _) = cap.format();
    assert_eq!(rate, SAMPLE_RATE);

    let chunk = cap.read_chunk(Duration::from_secs(1)).unwrap();
    assert_eq!(chunk, pcm);

    let empty = cap.read_chunk(Duration::from_millis(1)).unwrap();
    assert!(empty.is_empty());
}
