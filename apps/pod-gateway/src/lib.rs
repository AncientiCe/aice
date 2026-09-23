//! Room bridge between M5Stack pods and the aice backend.

mod bridge;

pub use bridge::{
    spawn_bridge, BridgeHandle, BridgeSettings, PiperSpeech, Speech, TurnDetector, TurnStep,
    VadSettings, MAX_AUDIO_PAYLOAD_BYTES,
};
