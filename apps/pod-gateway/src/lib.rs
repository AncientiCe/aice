//! Pod gateway: accepts WebSocket connections from M5Stack pods and forwards audio.

mod server;

pub use server::{run_gateway, PodEgressCommand, PodIngestEvent, TapSender};
