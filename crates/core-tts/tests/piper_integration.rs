//! Behavioral tests for Piper TTS adapter paths.

use core_orchestrator::TtsSink;
use core_tts::PiperTtsSink;
use std::io::Write;
use tempfile::Builder;

#[tokio::test]
async fn piper_flush_without_push_succeeds() {
    let dir = Builder::new()
        .prefix("aice-piper-model-")
        .tempdir()
        .unwrap();
    let model_path = dir.path().join("voice.onnx");
    std::fs::File::create(&model_path).unwrap();
    let mut tts = PiperTtsSink::new(&model_path).unwrap();
    tts.flush().await.unwrap();
}

#[tokio::test]
async fn piper_flush_with_text_and_missing_binary_errors() {
    let dir = Builder::new()
        .prefix("aice-piper-model-")
        .tempdir()
        .unwrap();
    let model_path = dir.path().join("voice.onnx");
    let mut f = std::fs::File::create(&model_path).unwrap();
    f.write_all(b"dummy-model").unwrap();
    f.sync_all().unwrap();
    std::env::set_var("PIPER_BIN", "definitely_missing_piper_binary_xyz");
    let mut tts = PiperTtsSink::new(&model_path).unwrap();
    tts.push_text("Hello world").await.unwrap();
    let err = tts.flush().await.unwrap_err().to_string();
    assert!(err.contains("failed to start piper"));
    std::env::remove_var("PIPER_BIN");
}
