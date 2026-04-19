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

/// Spotify Web API settings for media skill.
///
/// Auth credentials become a `StaticSpotifyAuth` once the engine refreshes;
/// this struct only carries the OAuth client + refresh-token material.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SpotifyConfig {
    /// Enable the Spotify backend skill.
    #[serde(default)]
    pub enabled: bool,
    /// Spotify OAuth client id.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Spotify OAuth client secret.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Long-lived refresh token; engine exchanges it for short-lived access tokens.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Optional default device name to target when the user does not specify one.
    #[serde(default)]
    pub default_device_name: Option<String>,
}

/// Media feature settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MediaConfig {
    #[serde(default)]
    pub macos_music: MacOsMusicConfig,
    /// Optional preferred provider override (e.g. `"spotify"`, `"macos_music"`,
    /// `"windows_apple_music"`). When `None`, the backend infers from `media_target`
    /// or platform defaults.
    #[serde(default)]
    pub preferred_provider: Option<String>,
    /// Spotify Web API provider configuration.
    #[serde(default)]
    pub spotify: SpotifyConfig,
}

fn default_search_timeout_secs() -> u64 {
    10
}

/// Google Calendar OAuth + default-calendar settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GoogleCalendarConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub default_calendar_name: Option<String>,
}

/// Calendar feature settings.
///
/// `provider` selects how SkillCalendar is fulfilled: `"google"` runs server-side,
/// `"apple"` is forwarded to the desktop runner via FrontendSkillIntent.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CalendarConfig {
    #[serde(default = "default_calendar_provider")]
    pub provider: String,
    #[serde(default)]
    pub google: GoogleCalendarConfig,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            provider: default_calendar_provider(),
            google: GoogleCalendarConfig::default(),
        }
    }
}

fn default_calendar_provider() -> String {
    "google".to_string()
}

/// IMAP credentials for the email skill (used when provider = `"imap"`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImapEmailConfig {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default = "default_imap_port")]
    pub port: u16,
    #[serde(default = "default_imap_tls")]
    pub tls: bool,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_imap_mailbox")]
    pub default_mailbox: String,
}

impl Default for ImapEmailConfig {
    fn default() -> Self {
        Self {
            host: None,
            port: default_imap_port(),
            tls: default_imap_tls(),
            username: None,
            password: None,
            default_mailbox: default_imap_mailbox(),
        }
    }
}

fn default_imap_port() -> u16 {
    993
}

fn default_imap_tls() -> bool {
    true
}

fn default_imap_mailbox() -> String {
    "INBOX".to_string()
}

/// Email feature settings.
///
/// `provider` selects how SkillEmail is fulfilled: `"imap"` runs server-side,
/// `"apple"` is forwarded to the desktop runner via FrontendSkillIntent.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmailConfig {
    #[serde(default = "default_email_provider")]
    pub provider: String,
    #[serde(default)]
    pub imap: ImapEmailConfig,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            provider: default_email_provider(),
            imap: ImapEmailConfig::default(),
        }
    }
}

fn default_email_provider() -> String {
    "imap".to_string()
}

/// Personal journal feature settings (SQLite-backed).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JournalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_journal_sqlite_path")]
    pub sqlite_path: String,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sqlite_path: default_journal_sqlite_path(),
        }
    }
}

fn default_journal_sqlite_path() -> String {
    "journal.sqlite".to_string()
}

/// Daily briefing composition settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BriefingConfig {
    #[serde(default = "default_true")]
    pub include_weather: bool,
    #[serde(default = "default_true")]
    pub include_calendar: bool,
    #[serde(default)]
    pub include_email: bool,
    #[serde(default = "default_true")]
    pub include_news: bool,
    #[serde(default)]
    pub news_topic: Option<String>,
    #[serde(default = "default_briefing_news_limit")]
    pub news_limit: usize,
}

impl Default for BriefingConfig {
    fn default() -> Self {
        Self {
            include_weather: true,
            include_calendar: true,
            include_email: false,
            include_news: true,
            news_topic: None,
            news_limit: default_briefing_news_limit(),
        }
    }
}

fn default_briefing_news_limit() -> usize {
    3
}

fn default_true() -> bool {
    true
}

/// Screen OCR settings — capture stays on the runtime, OCR engine is selected here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScreenOcrConfig {
    /// `"tesseract"` (cross-platform) or `"apple_vision"` (macOS only).
    #[serde(default = "default_ocr_engine")]
    pub ocr_engine: String,
    /// Optional explicit path to the `tesseract` binary. If `None`, look it up on `PATH`.
    #[serde(default)]
    pub tesseract_path: Option<String>,
}

impl Default for ScreenOcrConfig {
    fn default() -> Self {
        Self {
            ocr_engine: default_ocr_engine(),
            tesseract_path: None,
        }
    }
}

fn default_ocr_engine() -> String {
    "tesseract".to_string()
}

/// News feature settings (in addition to per-call NewsHeadlinesQuery).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct NewsConfig {
    /// Stream LLM-summarized headlines after the structured `NewsHeadlines` answer.
    /// Off by default — opt in to trade latency for richer per-headline summaries.
    #[serde(default)]
    pub enable_summary_streaming: bool,
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
    /// Enable tuned endpointing values for lower latency (Pass A).
    #[serde(default)]
    pub enable_endpointing_tuning: bool,
    /// Tuned timeout waiting for one capture chunk (milliseconds).
    #[serde(default = "default_audio_tuned_chunk_timeout_ms")]
    pub tuned_chunk_timeout_ms: u64,
    /// Tuned transcription window size in milliseconds.
    #[serde(default = "default_audio_tuned_turn_window_ms")]
    pub tuned_turn_window_ms: u64,
    /// Tuned speech end silence threshold in milliseconds.
    #[serde(default = "default_audio_tuned_speech_end_silence_ms")]
    pub tuned_speech_end_silence_ms: u64,
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
            enable_endpointing_tuning: false,
            tuned_chunk_timeout_ms: default_audio_tuned_chunk_timeout_ms(),
            tuned_turn_window_ms: default_audio_tuned_turn_window_ms(),
            tuned_speech_end_silence_ms: default_audio_tuned_speech_end_silence_ms(),
        }
    }
}

fn default_audio_chunk_timeout_ms() -> u64 {
    80
}

fn default_audio_turn_window_ms() -> u64 {
    1500
}

fn default_audio_idle_sleep_ms() -> u64 {
    20
}

fn default_audio_speech_end_silence_ms() -> u64 {
    180
}

fn default_audio_speech_rms_threshold() -> f32 {
    0.008
}

fn default_audio_tuned_chunk_timeout_ms() -> u64 {
    40
}

fn default_audio_tuned_turn_window_ms() -> u64 {
    900
}

fn default_audio_tuned_speech_end_silence_ms() -> u64 {
    120
}

/// STT settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SttConfig {
    /// Path to whisper model file.
    #[serde(default = "default_whisper_model_path")]
    pub whisper_model_path: String,
    /// Warm Whisper model/state at startup to reduce first-turn latency.
    #[serde(default = "default_stt_preload_model_on_startup")]
    pub preload_model_on_startup: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            whisper_model_path: default_whisper_model_path(),
            preload_model_on_startup: default_stt_preload_model_on_startup(),
        }
    }
}

fn default_whisper_model_path() -> String {
    "models/whisper/ggml-base.en.bin".to_string()
}

fn default_stt_preload_model_on_startup() -> bool {
    true
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
    /// Enable larger TTS text chunks to reduce push/flush overhead.
    #[serde(default)]
    pub enable_chunked_push_optimization: bool,
    /// Number of bytes per TTS text chunk when chunked push optimization is enabled.
    #[serde(default = "default_tts_push_chunk_bytes")]
    pub push_chunk_bytes: usize,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            piper_model_path: default_piper_model_path(),
            piper_config_path: None,
            enable_chunked_push_optimization: false,
            push_chunk_bytes: default_tts_push_chunk_bytes(),
        }
    }
}

fn default_piper_model_path() -> String {
    "models/piper/model.onnx".to_string()
}

fn default_tts_push_chunk_bytes() -> usize {
    24
}

/// Service/runtime operational settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Bind address for HTTP health endpoint (`GET /healthz`).
    #[serde(default = "default_health_bind")]
    pub health_bind: String,
    /// Enable local Prometheus scrape endpoint for runtime metrics.
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
    /// Bind address for Prometheus metrics HTTP endpoint.
    #[serde(default = "default_metrics_bind")]
    pub metrics_bind: String,
    /// Crash restart backoff for wrapper scripts.
    #[serde(default = "default_restart_backoff_secs")]
    pub restart_backoff_secs: u64,
    /// Backend audio turn idle timeout in milliseconds.
    #[serde(default = "default_audio_session_idle_timeout_ms")]
    pub audio_session_idle_timeout_ms: u64,
    /// Backend audio turn maximum duration in milliseconds.
    #[serde(default = "default_audio_session_max_duration_ms")]
    pub audio_session_max_duration_ms: u64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            health_bind: default_health_bind(),
            metrics_enabled: default_metrics_enabled(),
            metrics_bind: default_metrics_bind(),
            restart_backoff_secs: default_restart_backoff_secs(),
            audio_session_idle_timeout_ms: default_audio_session_idle_timeout_ms(),
            audio_session_max_duration_ms: default_audio_session_max_duration_ms(),
        }
    }
}

fn default_health_bind() -> String {
    "127.0.0.1:8780".to_string()
}

fn default_restart_backoff_secs() -> u64 {
    3
}

fn default_metrics_enabled() -> bool {
    true
}

fn default_metrics_bind() -> String {
    "127.0.0.1:9001".to_string()
}

fn default_audio_session_idle_timeout_ms() -> u64 {
    5_000
}

fn default_audio_session_max_duration_ms() -> u64 {
    20_000
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
    /// Skip secondary LLM answer-composer calls for backend skill summaries.
    #[serde(default)]
    pub skip_secondary_llm_for_skill_answers: bool,
    /// Context window cap for classifier calls (Ollama `num_ctx`). Smaller saves VRAM and
    /// reduces prefill time.
    #[serde(default = "default_classifier_num_ctx")]
    pub classifier_num_ctx: Option<u32>,
    /// Optional separate Ollama URL for classification (e.g. `http://127.0.0.1:11435`).
    /// Keeps classifier KV cache isolated from chat generation on the main instance.
    #[serde(default)]
    pub classifier_ollama_url: Option<String>,
    /// Warm the LLM model at startup to reduce first-turn latency.
    #[serde(default = "default_preload_model_on_startup")]
    pub preload_model_on_startup: bool,
    /// Keep model resident in provider memory (e.g. Ollama `keep_alive` value like `30m`).
    #[serde(default = "default_model_keep_alive")]
    pub model_keep_alive: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            short_replies: default_short_replies(),
            max_output_tokens: default_max_output_tokens(),
            system_prompt: None,
            skip_secondary_llm_for_skill_answers: false,
            classifier_num_ctx: default_classifier_num_ctx(),
            classifier_ollama_url: None,
            preload_model_on_startup: default_preload_model_on_startup(),
            model_keep_alive: default_model_keep_alive(),
        }
    }
}

fn default_short_replies() -> bool {
    true
}

fn default_max_output_tokens() -> u32 {
    48
}

fn default_classifier_num_ctx() -> Option<u32> {
    Some(1024)
}

fn default_preload_model_on_startup() -> bool {
    true
}

fn default_model_keep_alive() -> Option<String> {
    Some("30m".to_string())
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
    /// Path to the memory palace SQLite database file.
    #[serde(default = "default_palace_db_path")]
    pub palace_db_path: String,
    /// Path to the memory palace identity file (L0 context).
    #[serde(default = "default_palace_identity_path")]
    pub palace_identity_path: String,
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

fn default_palace_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.mempalace/palace/palace.db")
}

fn default_palace_identity_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.mempalace/identity.txt")
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
            palace_db_path: default_palace_db_path(),
            palace_identity_path: default_palace_identity_path(),
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
    /// Model name for chat (e.g. qwen2.5:7b).
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
    /// Calendar provider settings.
    #[serde(default)]
    pub calendar: CalendarConfig,
    /// Email provider settings.
    #[serde(default)]
    pub email: EmailConfig,
    /// Personal journal settings.
    #[serde(default)]
    pub journal: JournalConfig,
    /// Daily briefing composition settings.
    #[serde(default)]
    pub briefing: BriefingConfig,
    /// Screen OCR engine selection.
    #[serde(default)]
    pub screen_ocr: ScreenOcrConfig,
    /// News skill behavior (e.g. opt-in summary streaming).
    #[serde(default)]
    pub news: NewsConfig,
}

fn default_ollama_url() -> String {
    "http://127.0.0.1:11434".to_string()
}

fn default_model() -> String {
    "qwen2.5:7b".to_string()
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
            assistant_profile: AssistantProfileConfig::default(),
            smart_home: SmartHomeConfig::default(),
            media: MediaConfig::default(),
            memory: MemoryConfig::default(),
            calendar: CalendarConfig::default(),
            email: EmailConfig::default(),
            journal: JournalConfig::default(),
            briefing: BriefingConfig::default(),
            screen_ocr: ScreenOcrConfig::default(),
            news: NewsConfig::default(),
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
    use super::{Config, MacOsMusicConfig};
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let path = Path::new("nonexistent_config_that_does_not_exist_12345.json");
        let config = Config::load(path).must();
        assert_eq!(config.ollama_url, "http://127.0.0.1:11434");
        assert_eq!(config.model, "qwen2.5:7b");
        assert!(config.llm.short_replies);
        assert_eq!(config.llm.max_output_tokens, 48);
        assert_eq!(config.llm.classifier_num_ctx, Some(1024));
        assert!(config.llm.preload_model_on_startup);
        assert_eq!(config.llm.model_keep_alive.as_deref(), Some("30m"));
        assert_eq!(config.pod_bind, "0.0.0.0:8765");
        assert_eq!(config.audio.chunk_timeout_ms, 80);
        assert_eq!(config.audio.turn_window_ms, 1500);
        assert_eq!(config.audio.speech_end_silence_ms, 180);
        assert_eq!(config.audio.speech_rms_threshold, 0.008);
        assert!(!config.audio.enable_endpointing_tuning);
        assert_eq!(config.audio.tuned_chunk_timeout_ms, 40);
        assert_eq!(config.audio.tuned_turn_window_ms, 900);
        assert_eq!(config.audio.tuned_speech_end_silence_ms, 120);
        assert_eq!(
            config.stt.whisper_model_path,
            "models/whisper/ggml-base.en.bin"
        );
        assert!(config.stt.preload_model_on_startup);
        assert_eq!(config.tts.piper_model_path, "models/piper/model.onnx");
        assert_eq!(config.service.health_bind, "127.0.0.1:8780");
        assert!(config.service.metrics_enabled);
        assert_eq!(config.service.metrics_bind, "127.0.0.1:9001");
        assert_eq!(config.service.audio_session_idle_timeout_ms, 5_000);
        assert_eq!(config.service.audio_session_max_duration_ms, 20_000);
        assert_eq!(config.assistant_profile.unit_system, "metric");
        assert_eq!(config.assistant_profile.time_format, "24h");
        assert!(config.memory.enabled);
        assert_eq!(config.memory.path, "memory.json");
        assert_eq!(config.memory.max_recent_turns, 10);
        assert_eq!(config.memory.sqlite_path, "memory.sqlite");
        assert!(config
            .memory
            .palace_db_path
            .ends_with(".mempalace/palace/palace.db"));
        assert!(config
            .memory
            .palace_identity_path
            .ends_with(".mempalace/identity.txt"));
    }

    #[test]
    fn load_parses_valid_json_and_overrides_defaults() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_test_valid.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "ollama_url":"http://localhost:11434",
                "model":"tinyllama",
                "llm":{"short_replies":false,"max_output_tokens":256,"system_prompt":"You are concise."},
                "pod_bind":"0.0.0.0:9000",
                "audio":{"turn_window_ms":900},
                "stt":{"whisper_model_path":"models/custom-whisper.bin"},
                "tts":{"piper_model_path":"models/custom-piper.onnx","piper_config_path":"models/custom-piper.onnx.json"},
                "service":{
                    "health_bind":"127.0.0.1:9898",
                    "restart_backoff_secs":5,
                    "metrics_enabled":false,
                    "metrics_bind":"127.0.0.1:9100",
                    "audio_session_idle_timeout_ms":2500,
                    "audio_session_max_duration_ms":12000
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.model, "tinyllama");
        assert!(!config.llm.short_replies);
        assert_eq!(config.llm.max_output_tokens, 256);
        assert_eq!(
            config.llm.system_prompt.as_deref(),
            Some("You are concise.")
        );
        assert_eq!(config.llm.classifier_num_ctx, Some(1024));
        assert!(config.llm.preload_model_on_startup);
        assert_eq!(config.llm.model_keep_alive.as_deref(), Some("30m"));
        assert_eq!(config.pod_bind, "0.0.0.0:9000");
        assert_eq!(config.audio.turn_window_ms, 900);
        assert_eq!(config.audio.speech_end_silence_ms, 180);
        assert_eq!(config.audio.speech_rms_threshold, 0.008);
        assert!(!config.audio.enable_endpointing_tuning);
        assert_eq!(config.stt.whisper_model_path, "models/custom-whisper.bin");
        assert!(config.stt.preload_model_on_startup);
        assert_eq!(config.tts.piper_model_path, "models/custom-piper.onnx");
        assert_eq!(
            config.tts.piper_config_path.as_deref(),
            Some("models/custom-piper.onnx.json")
        );
        assert_eq!(config.service.health_bind, "127.0.0.1:9898");
        assert_eq!(config.service.restart_backoff_secs, 5);
        assert!(!config.service.metrics_enabled);
        assert_eq!(config.service.metrics_bind, "127.0.0.1:9100");
        assert_eq!(config.service.audio_session_idle_timeout_ms, 2500);
        assert_eq!(config.service.audio_session_max_duration_ms, 12000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_assistant_profile_and_memory() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_profile_memory.json");
        let mut f = std::fs::File::create(&path).must();
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
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
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
    fn load_parses_llm_preload_and_keep_alive_overrides() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_llm_preload.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "llm": {
                    "preload_model_on_startup": false,
                    "model_keep_alive": "2h"
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert!(!config.llm.preload_model_on_startup);
        assert_eq!(config.llm.model_keep_alive.as_deref(), Some("2h"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_stt_preload_override() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_stt_preload.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "stt": {
                    "preload_model_on_startup": false
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert!(!config.stt.preload_model_on_startup);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_returns_error_for_invalid_json() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_test_invalid.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(b"{ invalid json }").must();
        f.sync_all().must();
        drop(f);

        let result = Config::load(&path);
        assert!(result.is_err());
        assert_eq!(result.must_err().kind(), std::io::ErrorKind::InvalidData);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn macos_music_config_serialization_has_no_auth_fields() {
        let cfg = MacOsMusicConfig::default();
        let value = serde_json::to_value(cfg).must();
        let obj = value.as_object().must();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("enabled"));
    }

    #[test]
    fn media_config_serializes_macos_music_key() {
        let cfg = Config::default();
        let value = serde_json::to_value(cfg).must();
        let media = value.get("media").and_then(|v| v.as_object()).must();
        assert!(media.contains_key("macos_music"));
        assert!(!media.contains_key("apple_music"));
    }

    #[test]
    fn load_parses_macos_music_config() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_macos_music.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "media": {
                    "macos_music": {
                        "enabled": true
                    }
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert!(config.media.macos_music.enabled);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn defaults_include_new_skill_config_blocks() {
        let cfg = Config::default();
        assert_eq!(cfg.calendar.provider, "google");
        assert!(cfg.calendar.google.client_id.is_none());
        assert_eq!(cfg.email.provider, "imap");
        assert_eq!(cfg.email.imap.port, 993);
        assert!(cfg.email.imap.tls);
        assert_eq!(cfg.email.imap.default_mailbox, "INBOX");
        assert!(!cfg.journal.enabled);
        assert_eq!(cfg.journal.sqlite_path, "journal.sqlite");
        assert!(cfg.briefing.include_weather);
        assert!(cfg.briefing.include_calendar);
        assert!(!cfg.briefing.include_email);
        assert!(cfg.briefing.include_news);
        assert_eq!(cfg.briefing.news_limit, 3);
        assert_eq!(cfg.screen_ocr.ocr_engine, "tesseract");
        assert!(cfg.screen_ocr.tesseract_path.is_none());
        assert!(!cfg.news.enable_summary_streaming);
        assert!(cfg.media.preferred_provider.is_none());
        assert!(!cfg.media.spotify.enabled);
    }

    #[test]
    fn load_parses_media_preferred_provider_spotify() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_media_spotify.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "media": {
                    "preferred_provider": "spotify",
                    "spotify": {
                        "enabled": true,
                        "client_id": "abc",
                        "client_secret": "def",
                        "refresh_token": "rt-xyz",
                        "default_device_name": "Living Room"
                    }
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert_eq!(config.media.preferred_provider.as_deref(), Some("spotify"));
        assert!(config.media.spotify.enabled);
        assert_eq!(config.media.spotify.client_id.as_deref(), Some("abc"));
        assert_eq!(config.media.spotify.client_secret.as_deref(), Some("def"));
        assert_eq!(
            config.media.spotify.refresh_token.as_deref(),
            Some("rt-xyz")
        );
        assert_eq!(
            config.media.spotify.default_device_name.as_deref(),
            Some("Living Room")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_calendar_provider_apple() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_calendar_apple.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "calendar": {
                    "provider": "apple"
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert_eq!(config.calendar.provider, "apple");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_calendar_provider_google_with_credentials() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_calendar_google.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "calendar": {
                    "provider": "google",
                    "google": {
                        "client_id": "cid",
                        "client_secret": "csec",
                        "refresh_token": "rtok",
                        "default_calendar_name": "Work"
                    }
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert_eq!(config.calendar.provider, "google");
        assert_eq!(config.calendar.google.client_id.as_deref(), Some("cid"));
        assert_eq!(
            config.calendar.google.refresh_token.as_deref(),
            Some("rtok")
        );
        assert_eq!(
            config.calendar.google.default_calendar_name.as_deref(),
            Some("Work")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_email_imap_settings() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_email_imap.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "email": {
                    "provider": "imap",
                    "imap": {
                        "host": "imap.example.com",
                        "port": 143,
                        "tls": false,
                        "username": "me@example.com",
                        "password": "secret",
                        "default_mailbox": "Work"
                    }
                }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert_eq!(config.email.provider, "imap");
        assert_eq!(config.email.imap.host.as_deref(), Some("imap.example.com"));
        assert_eq!(config.email.imap.port, 143);
        assert!(!config.email.imap.tls);
        assert_eq!(
            config.email.imap.username.as_deref(),
            Some("me@example.com")
        );
        assert_eq!(config.email.imap.password.as_deref(), Some("secret"));
        assert_eq!(config.email.imap.default_mailbox, "Work");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_parses_journal_briefing_screen_ocr_news() {
        let dir = std::env::temp_dir();
        let path = dir.join("aice_config_journal_briefing_ocr_news.json");
        let mut f = std::fs::File::create(&path).must();
        f.write_all(
            br#"{
                "journal": { "enabled": true, "sqlite_path": "data/journal.sqlite" },
                "briefing": {
                    "include_weather": false,
                    "include_calendar": true,
                    "include_email": true,
                    "include_news": false,
                    "news_topic": "technology",
                    "news_limit": 5
                },
                "screen_ocr": { "ocr_engine": "apple_vision", "tesseract_path": "/usr/local/bin/tesseract" },
                "news": { "enable_summary_streaming": true }
            }"#,
        )
        .must();
        f.sync_all().must();
        drop(f);

        let config = Config::load(&path).must();
        assert!(config.journal.enabled);
        assert_eq!(config.journal.sqlite_path, "data/journal.sqlite");
        assert!(!config.briefing.include_weather);
        assert!(config.briefing.include_calendar);
        assert!(config.briefing.include_email);
        assert!(!config.briefing.include_news);
        assert_eq!(config.briefing.news_topic.as_deref(), Some("technology"));
        assert_eq!(config.briefing.news_limit, 5);
        assert_eq!(config.screen_ocr.ocr_engine, "apple_vision");
        assert_eq!(
            config.screen_ocr.tesseract_path.as_deref(),
            Some("/usr/local/bin/tesseract")
        );
        assert!(config.news.enable_summary_streaming);

        let _ = std::fs::remove_file(&path);
    }
}
