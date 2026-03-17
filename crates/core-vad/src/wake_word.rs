//! Wake-word gating: config-driven cooldown and phrase list.
//! Caller triggers activation (e.g. from real detector or tap); gate enforces cooldown.

use core_config::WakeWordConfig;
use std::time::{Duration, Instant};

/// Gate that enforces wake-word config: when enabled, listening is allowed only
/// after activation and until cooldown expires.
#[derive(Debug)]
pub struct WakeWordGate {
    config: WakeWordConfig,
    last_activation: Option<Instant>,
}

impl WakeWordGate {
    pub fn new(config: WakeWordConfig) -> Self {
        Self {
            config,
            last_activation: None,
        }
    }

    /// Whether wake word is enabled (phrases and cooldown apply).
    pub fn is_enabled(&self) -> bool {
        self.config.enabled && !self.config.phrases.is_empty()
    }

    /// Configured phrases (e.g. ["computer", "assistant"]).
    pub fn phrases(&self) -> &[String] {
        &self.config.phrases
    }

    /// Sensitivity 0.0–1.0 from config.
    pub fn sensitivity(&self) -> f32 {
        self.config.sensitivity
    }

    /// Record that the wake word was detected (or user tapped). Enables listening.
    pub fn activate(&mut self, now: Instant) {
        self.last_activation = Some(now);
    }

    /// Close the gate immediately until the next wake activation.
    pub fn deactivate(&mut self) {
        self.last_activation = None;
    }

    /// Returns true if the pipeline should listen (when disabled, always true;
    /// when enabled, true only after activation and outside cooldown).
    pub fn should_listen(&self, now: Instant) -> bool {
        if !self.is_enabled() {
            return true;
        }
        let Some(last) = self.last_activation else {
            return false;
        };
        let cooldown = Duration::from_secs(self.config.cooldown_secs);
        now.saturating_duration_since(last) < cooldown
    }

    /// Seconds remaining in cooldown; 0 if not in cooldown or disabled.
    pub fn cooldown_remaining_secs(&self, now: Instant) -> u64 {
        if !self.is_enabled() {
            return 0;
        }
        let Some(last) = self.last_activation else {
            return self.config.cooldown_secs;
        };
        let elapsed = now.saturating_duration_since(last);
        let cooldown = Duration::from_secs(self.config.cooldown_secs);
        cooldown.saturating_sub(elapsed).as_secs()
    }
}
