//! Integration tests: desktop mic path (fake) produces PCM chunks for STT.

use core_audio::{AudioCapture, FakeCapture, SAMPLE_RATE};
use std::time::Duration;

pub trait TestOptionExt<T> {
    fn must(self) -> T;
}

impl<T> TestOptionExt<T> for Option<T> {
    fn must(self) -> T {
        match self {
            Some(value) => value,
            None => panic!("expected Some(..) in test"),
        }
    }
}

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
    fn must_err(self) -> E;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }

    fn must_err(self) -> E {
        match self {
            Ok(_) => panic!("expected Err(..) in test, got Ok"),
            Err(error) => error,
        }
    }
}

#[test]
fn fake_capture_yields_pcm_chunk_for_stt() {
    let pcm = vec![0_i16; 1600]; // 100 ms at 16 kHz
    let mut cap = FakeCapture::single_chunk(pcm.clone());
    let (rate, _) = cap.format();
    assert_eq!(rate, SAMPLE_RATE);

    let chunk = cap.read_chunk(Duration::from_secs(1)).must();
    assert_eq!(chunk, pcm);

    let empty = cap.read_chunk(Duration::from_millis(1)).must();
    assert!(empty.is_empty());
}
