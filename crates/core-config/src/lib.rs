//! Typed configuration with wake word and pod settings.

mod config;

pub use config::{
    AppleMusicConfig, AssistantProfileConfig, AudioRuntimeConfig, Config, HueConfig, LlmConfig,
    MediaConfig, MemoryConfig, SearchProviderConfig, ServiceConfig, SmartHomeConfig, SttConfig,
    TtsConfig, WakeWordConfig,
};
