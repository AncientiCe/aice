//! Typed configuration with wake word and pod settings.

mod config;

pub use config::{
    AssistantProfileConfig, AudioRuntimeConfig, Config, HueConfig, LlmConfig, MacOsMusicConfig,
    MediaConfig, MemoryConfig, SearchProviderConfig, ServiceConfig, SmartHomeConfig, SttConfig,
    TtsConfig, WakeWordConfig,
};
