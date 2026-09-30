//! Room bridge between room pods and the aice backend.

mod bridge;
mod echo;

pub use bridge::{
    spawn_bridge, BargeInSettings, BridgeHandle, BridgeSettings, PiperSpeech, Speech, TurnDetector,
    TurnStep, VadSettings, MAX_AUDIO_PAYLOAD_BYTES,
};
