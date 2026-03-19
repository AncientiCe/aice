//! Behavioral tests for STT: push_audio + flush yields transcript.
//! Uses FakeSttStream to define contract; WhisperSttStream implements it.

use core_orchestrator::SttStream;
use core_stt::FakeSttStream;

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

#[tokio::test]
async fn stt_push_audio_then_flush_returns_transcript() {
    let mut stt = FakeSttStream::new("hello world");
    let pcm = vec![0_i16; 1600];
    stt.push_audio(&pcm).await.must();
    let transcript = stt.flush().await.must();
    assert_eq!(transcript, "hello world");
}

#[tokio::test]
async fn stt_flush_without_push_returns_configured_transcript() {
    let mut stt = FakeSttStream::new("empty");
    let transcript = stt.flush().await.must();
    assert_eq!(transcript, "empty");
}
