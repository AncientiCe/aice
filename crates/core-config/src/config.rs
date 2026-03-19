//! Configuration structures.

use serde::{Deserialize, Serialize};

/// Wake word detection settings (configurable phrase, sensitivity, cooldown).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WakeWordConfig {
    /// Whether wake word detection is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Phrases that activate the assistant (e.g. ["computer", "assistant"]).
    #[serde(default)]
    pub phrases: Vec<String>,
    /// Sensitivity 0.0–1.0 (higher = more sensitive).
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,
    /// Cooldown in seconds after activation before listening again.
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
}

fn default_sensitivity() -> f32 {
    0.5
}

fn default_cooldown_secs() -> u64 {
    2
}

/// Search provider settings (fallback web search after user confirms).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SearchProviderConfig {
    /// Base URL for search API (e.g. https://api.example.com/search). Empty = disabled.
    #[serde(default)]
    pub url: String,
    /// Optional API key (e.g. sent as Authorization: Bearer <key> or query param).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Timeout in seconds for search requests.
    #[serde(default = "default_search_timeout_secs")]
    pub timeout_secs: u64,
}

/// Philips Hue settings for smart-home skill.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HueConfig {
    /// Enable Hue smart-home skill.
    #[serde(default)]
    pub enabled: bool,
    /// Hue bridge host/ip (e.g. 192.168.1.25).
    #[serde(default)]
    pub bridge_host: Option<String>,
    /// Hue application key created by bridge button-link flow.
    #[serde(default)]
    pub app_key: Option<String>,
    /// Preferred light name for default target resolution.
    #[serde(default = "default_hue_light_name")]
    pub default_light_name: String,
}

impl Default for HueConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge_host: None,
            app_key: None,
            default_light_name: default_hue_light_name(),
        }
    }
}

fn default_hue_light_name() -> String {
    "Philips Hue White & Colour Ambience LED Table Light".to_string()
}

/// Smart-home feature settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SmartHomeConfig {
    #[serde(default)]
    pub hue: HueConfig,
}

/// macOS Music.app settings for media skill.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MacOsMusicConfig {
    /// Enable macOS Music.app media skill.
    #[serde(default)]
    pub enabled: bool,
}

/// Media feature settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MediaConfig {
    #[serde(default)]
    pub macos_music: MacOsMusicConfig,
}

fn default_search_timeout_secs() -> u64 {
    10
}

/// Audio runtime settings for capture/playback loop behavior.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioRuntimeConfig {
    /// Optional preferred input device name substring.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Optional preferred output device name substring.
    #[serde(default)]
    pub output_device: Option<String>,
    /// Timeout waiting for one capture chunk (milliseconds).
    #[serde(default = "default_audio_chunk_timeout_ms")]
    pub chunk_timeout_ms: u64,
    /// Approximate transcription window size in milliseconds.
    #[serde(default = "default_audio_turn_window_ms")]
    pub turn_window_ms: u64,
    /// Sleep between loop iterations when idle.
    #[serde(default = "default_audio_idle_sleep_ms")]
    pub idle_sleep_ms: u64,
    /// Consecutive silence before flushing buffered speech (milliseconds).
    #[serde(default = "default_audio_speech_end_silence_ms")]
    pub speech_end_silence_ms: u64,
    /// Minimum RMS amplitude considered speech (0.0-1.0).
    #[serde(default = "default_audio_speech_rms_threshold")]
    pub speech_rms_threshold: f32,
}

impl Default for AudioRuntimeConfig {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            chunk_timeout_ms: default_audio_chunk_timeout_ms(),
            turn_window_ms: default_audio_turn_window_ms(),
            idle_sleep_ms: default_audio_idle_sleep_ms(),
            speech_end_silence_ms: default_audio_speech_end_silence_ms(),
            speech_rms_threshold: default_audio_speech_rms_threshold(),
        }
    }
}

fn default_audio_chunk_timeout_ms() -> u64 {
    250
}

fn default_audio_turn_window_ms() -> u64 {
    1500
}

fn default_audio_idle_sleep_ms() -> u64 {
    20
}

fn default_audio_speech_end_silence_ms() -> u64 {
    300
}

fn default_audio_speech_rms_threshold() -> f32 {
    0.008
}

/// STT settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SttConfig {
    /// Path to whisper model file.
    #[serde(default = "default_whisper_model_path")]
    pub whisper_model_path: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            whisper_model_path: default_whisper_model_path(),
        }
    }
}

fn default_whisper_model_path() -> String {
    "models/whisper/ggml-tiny.en.bin".to_string()
}

/// TTS settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TtsConfig {
    /// Path to Piper voice model.
    #[serde(default = "default_piper_model_path")]
    pub piper_model_path: String,
    /// Optional Piper model config path.
    #[serde(default)]
    pub piper_config_path: Option<String>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            piper_model_path: default_piper_model_path(),
            piper_config_path: None,
        }
    }
}

fn default_piper_model_path() -> String {
    "models/piper/model.onnx".to_string()
}

/// Service/runtime operational settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Bind address for HTTP health endpoint (`GET /healthz`).
    #[serde(default = "default_health_bind")]
    pub health_bind: String,
    /// Crash restart backoff for wrapper scripts.
    #[serde(default = "default_restart_backoff_secs")]
    pub restart_backoff_secs: u64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            health_bind: default_health_bind(),
            restart_backoff_secs: default_restart_backoff_secs(),
        }
    }
}

fn default_health_bind() -> String {
    "127.0.0.1:8780".to_string()
}

fn default_restart_backoff_secs() -> u64 {
    3
}

/// LLM behavior settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LlmConfig {
    /// Prefer concise answers for low-latency voice UX.
    #[serde(default = "default_short_replies")]
    pub short_replies: bool,
    /// Max output tokens for each assistant turn.
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// Optional explicit system prompt override.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            short_replies: default_short_replies(),
            max_output_tokens: default_max_output_tokens(),
            system_prompt: None,
        }
    }
}

fn default_short_replies() -> bool {
    true
}

fn default_max_output_tokens() -> u32 {
    96
}

/// Assistant profile: persona, units, and user identity for prompt context.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssistantProfileConfig {
    /// Assistant name (e.g. "Jarvis").
    #[serde(default)]
    pub name: Option<String>,
    /// Optional persona / tone description.
    #[serde(default)]
    pub persona: Option<String>,
    /// Unit system: "metric" or "imperial". Default metric.
    #[serde(default = "default_unit_system")]
    pub unit_system: String,
    /// Time format: "24h" or "12h". Default 24h.
    #[serde(default = "default_time_format")]
    pub time_format: String,
    /// User's preferred name (how the assistant should address the user).
    #[serde(default)]
    pub user_name: Option<String>,
}

fn default_unit_system() -> String {
    "metric".to_string()
}

fn default_time_format() -> String {
    "24h".to_string()
}

impl Default for AssistantProfileConfig {
    fn default() -> Self {
        Self {
            name: None,
            persona: None,
            unit_system: default_unit_system(),
            time_format: default_time_format(),
            user_name: None,
        }
    }
}

/// Persistent memory store settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryConfig {
    /// Whether memory load/save is enabled.
    #[serde(default = "default_memory_enabled")]
    pub enabled: bool,
    /// File path for memory JSON (relative to cwd or absolute).
    #[serde(default = "default_memory_path")]
    pub path: String,
    /// Max recent (user, assistant) turns to keep and send as history.
    #[serde(default = "default_max_recent_turns")]
    pub max_recent_turns: usize,
    /// Max profile/facts entries to store.
    #[serde(default = "default_max_facts")]
    pub max_facts: usize,
    /// Save to disk after each completed turn when enabled.
    #[serde(default = "default_memory_autosave")]
    pub autosave: bool,
    /// SQLite database file path for long-term memory.
    #[serde(default = "default_memory_sqlite_path")]
    pub sqlite_path: String,
}

fn default_memory_enabled() -> bool {
    true
}

fn default_memory_path() -> String {
    "memory.json".to_string()
}

fn default_max_recent_turns() -> usize {
    10
}

fn default_max_facts() -> usize {
    50
}

fn default_memory_autosave() -> bool {
    true
}

fn default_memory_sqlite_path() -> String {
    "memory.sqlite".to_string()
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_memory_enabled(),
            path: default_memory_path(),
            max_recent_turns: default_max_recent_turns(),
            max_facts: default_max_facts(),
            autosave: default_memory_autosave(),
            sqlite_path: default_memory_sqlite_path(),
        }
    }
}

/// Root application configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub wake_word: WakeWordConfig,
    #[serde(default)]
    pub search_provider: SearchProviderConfig,
    /// Ollama base URL (e.g. http://127.0.0.1:11434).
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    /// Model name for chat (e.g. llama3.2).
    #[serde(default = "default_model")]
    pub model: String,
    /// LLM behavior options.
    #[serde(default)]
    pub llm: LlmConfig,
    /// Pod gateway bind address.
    #[serde(default = "default_pod_bind")]
    pub pod_bind: String,
    /// Audio runtime options.
    #[serde(default)]
    pub audio: AudioRuntimeConfig,
    /// STT options.
    #[serde(default)]
    pub stt: SttConfig,
    /// TTS options.
    #[serde(default)]
    pub tts: TtsConfig,
    /// Service/runtime operational options.
    #[serde(default)]
    pub service: ServiceConfig,
    /// Default location (e.g. city name) when startup geolocation fails; used for weather skill.
    #[serde(default)]
    pub default_location: Option<String>,
    /// Assistant profile (name, units, user identity) for prompt context.
    #[serde(default)]
    pub assistant_profile: AssistantProfileConfig,
    /// Smart-home providers.
    #[serde(default)]
    pub smart_home: SmartHomeConfig,
    /// Media providers.
    #[serde(default)]
    pub media: MediaConfig,
    /// Persistent memory store settings.
    #[serde(default)]
    pub memory: MemoryConfig,
}

fn default_ollama_url() -> String {
    "http://127.0.0.1:11434".to_string()
}

fn default_model() -> String {
    "llama3.2".to_string()
}

fn default_pod_bind() -> String {
    "0.0.0.0:8765".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wake_word: WakeWordConfig::default(),
            search_provider: SearchProviderConfig::default(),
            ollama_url: default_ollama_url(),
            model: default_model(),
            llm: LlmConfig::default(),
            pod_bind: default_pod_bind(),
            audio: AudioRuntimeConfig::default(),
            stt: SttConfig::default(),
            tts: TtsConfig::default(),
            service: ServiceConfig::default(),
            default_location: None,
            assistant_profile: AssistantProfileConfig::default(),
            smart_home: SmartHomeConfig::default(),
            media: MediaConfig::default(),
            memory: MemoryConfig::default(),
        }
    }
}

impl Config {
    /// Load config from a file if present; otherwise return defaults.
    pub fn load(path: &std::path::Path) -> Result<Self, std::io::Error> {
        if path.exists() {
            let s = std::fs::read_to_string(path)?;
            let config: Config = serde_json::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, MacOsMusicConfig};
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let path = Path::new("nonexistent_config_that_does_not_exist_12345.json");
        let config = Config::load(path).unwrap();
        assert_eq!(config.ollama_url, "http://127.0.0.1:11434");
        assert_eq!(config.model, "llama3.2");
        assert!(config.llm.short_replies);
        assert_eq!(config.llm.max_output_tokens, 96);
        assert_eq!(config.pod_bind, "0.0.0.0:8765");
        assert_eq!(config.audio.turn_window_ms, 1500);
        assert_eq!(config.audio.speech_end_silence_ms, 300);
        assert_eq!(config.audio.speech_rms_threshold, 0.008);
        assert_eq!(
            config.stt.whisper_model_path,
            "models/whisper/ggml-tiny.en.bin"
        );
        assert_eq!(config.tts.piper_model_path, "models/piper/model.onnx");
        assert_eq!(config.service.health_bind, "127.0.0.1:8780");
        assert_eq!(config.assistant_profile.unit_system, "metric");
        assert_eq!(config.assistant_profile.time_format, "24h");
        assert!(config.memory.enabled);
        assert_eq!(config.memory.path, "memory.json");
        assert_eq!(config.memory.max_recent_turns, 10);
        assert_eq!(config.memory.sqlite_path, "memory.sqlite");
    }

    #[test]
    fn load_parses_valid_json_and_overrides_defaults() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_test_valid.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"{
                "ollama_url":"http://localhost:11434",
                "model":"tinyllama",
                "llm":{"short_replies":false,"max_output_tokens":256,"system_prompt":"You are concise."},
                "pod_bind":"0.0.0.0:9000",
                "audio":{"turn_window_ms":900},
                "stt":{"whisper_model_path":"models/custom-whisper.bin"},
                "tts":{"piper_model_path":"models/custom-piper.onnx","piper_config_path":"models/custom-piper.onnx.json"},
                "service":{"health_bind":"127.0.0.1:9898","restart_backoff_secs":5}
            }"#,
        )
        .unwrap();
        f.sync_all().unwrap();
        drop(f);

        let config = Config::load(&path).unwrap();
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.model, "tinyllama");
        assert!(!config.llm.short_replies);
        assert_eq!(config.llm.max_output_tokens, 256);
        assert_eq!(
            config.llm.system_prompt.as_deref(),
            Some("You are concise.")
        );
        assert_eq!(config.pod_bind, "0.0.0.0:9000");
        assert_eq!(config.audio.turn_window_ms, 900);
        assert_eq!(config.audio.speech_end_silence_ms, 300);
        assert_eq!(config.audio.speech_rms_threshold, 0.008);
        assert_eq!(config.stt.whisper_model_path, "models/custom-whisper.bin");
        assert_eq!(config.tts.piper_model_path, "models/custom-piper.onnx");
        assert_eq!(
            config.tts.piper_config_path.as_deref(),
            Some("models/custom-piper.onnx.json")
        );
        assert_eq!(config.service.health_bind, "127.0.0.1:9898");
        assert_eq!(config.service.restart_backoff_secs, 5);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_assistant_profile_and_memory() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_profile_memory.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"{
                "assistant_profile": {
                    "name": "Jarvis",
                    "persona": "Helpful home assistant.",
                    "unit_system": "metric",
                    "time_format": "24h",
                    "user_name": "Ancie"
                },
                "memory": {
                    "enabled": true,
                    "path": "data/memory.json",
                    "max_recent_turns": 6,
                    "max_facts": 20,
                    "autosave": false,
                    "sqlite_path": "data/memory.sqlite"
                }
            }"#,
        )
        .unwrap();
        f.sync_all().unwrap();
        drop(f);

        let config = Config::load(&path).unwrap();
        assert_eq!(config.assistant_profile.name.as_deref(), Some("Jarvis"));
        assert_eq!(
            config.assistant_profile.persona.as_deref(),
            Some("Helpful home assistant.")
        );
        assert_eq!(config.assistant_profile.unit_system, "metric");
        assert_eq!(config.assistant_profile.user_name.as_deref(), Some("Ancie"));
        assert!(config.memory.enabled);
        assert_eq!(config.memory.path, "data/memory.json");
        assert_eq!(config.memory.max_recent_turns, 6);
        assert_eq!(config.memory.max_facts, 20);
        assert!(!config.memory.autosave);
        assert_eq!(config.memory.sqlite_path, "data/memory.sqlite");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_returns_error_for_invalid_json() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_test_invalid.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"{ invalid json }").unwrap();
        f.sync_all().unwrap();
        drop(f);

        let result = Config::load(&path);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn macos_music_config_serialization_has_no_auth_fields() {
        let cfg = MacOsMusicConfig::default();
        let value = serde_json::to_value(cfg).expect("serialize macos music config");
        let obj = value.as_object().expect("config object");
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("enabled"));
    }

    #[test]
    fn media_config_serializes_macos_music_key() {
        let cfg = Config::default();
        let value = serde_json::to_value(cfg).expect("serialize root config");
        let media = value
            .get("media")
            .and_then(|v| v.as_object())
            .expect("media object");
        assert!(media.contains_key("macos_music"));
        assert!(!media.contains_key("apple_music"));
    }

    #[test]
    fn load_parses_macos_music_config() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_macos_music.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"{
                "media": {
                    "macos_music": {
                        "enabled": true
                    }
                }
            }"#,
        )
        .unwrap();
        f.sync_all().unwrap();
        drop(f);

        let config = Config::load(&path).unwrap();
        assert!(config.media.macos_music.enabled);

        let _ = std::fs::remove_file(&path);
    }
}
