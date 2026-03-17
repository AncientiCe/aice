//! Behavioral tests for STT: push_audio + flush yields transcript.
//! Uses FakeSttStream to define contract; WhisperSttStream implements it.

use core_orchestrator::SttStream;
use core_stt::FakeSttStream;

#[tokio::test]
async fn stt_push_audio_then_flush_returns_transcript() {
    let mut stt = FakeSttStream::new("hello world");
    let pcm = vec![0_i16; 1600];
    stt.push_audio(&pcm).await.unwrap();
    let transcript = stt.flush().await.unwrap();
    assert_eq!(transcript, "hello world");
}

#[tokio::test]
async fn stt_flush_without_push_returns_configured_transcript() {
    let mut stt = FakeSttStream::new("empty");
    let transcript = stt.flush().await.unwrap();
    assert_eq!(transcript, "empty");
}
