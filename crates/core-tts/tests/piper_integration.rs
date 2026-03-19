//! Behavioral tests for Piper TTS adapter paths.

use core_orchestrator::TtsSink;
use core_tts::PiperTtsSink;
use std::io::Write;
use tempfile::Builder;

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
async fn piper_flush_without_push_succeeds() {
    let dir = Builder::new().prefix("aice-piper-model-").tempdir().must();
    let model_path = dir.path().join("voice.onnx");
    std::fs::File::create(&model_path).must();
    let mut tts = PiperTtsSink::new(&model_path).must();
    tts.flush().await.must();
}

#[tokio::test]
async fn piper_flush_with_text_and_missing_binary_errors() {
    let dir = Builder::new().prefix("aice-piper-model-").tempdir().must();
    let model_path = dir.path().join("voice.onnx");
    let mut f = std::fs::File::create(&model_path).must();
    f.write_all(b"dummy-model").must();
    f.sync_all().must();
    std::env::set_var("PIPER_BIN", "definitely_missing_piper_binary_xyz");
    let mut tts = PiperTtsSink::new(&model_path).must();
    tts.push_text("Hello world").await.must();
    let err = tts.flush().await.must_err().to_string();
    assert!(err.contains("failed to start piper"));
    std::env::remove_var("PIPER_BIN");
}
