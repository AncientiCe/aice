//! Media skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct MediaResult {
    pub summary: String,
    pub now_playing: Option<String>,
    pub state: String,
}

impl MediaResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let np = self.now_playing.as_deref().unwrap_or("(nothing)");
        format!(
            "{}. State: {}. Now playing: {}.",
            self.summary, self.state, np
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum MediaSkillError {
    #[error("auth error: {0}")]
    Auth(String),
    #[error("playback or device error: {0}")]
    Playback(String),
    #[error("no active source or device")]
    NoSource,
    #[error("unsupported action: {0}")]
    UnsupportedAction(String),
}

/// Media skill: play, pause, source, room; action and optional target from intent.
#[async_trait]
pub trait MediaSkill: Send + Sync {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<MediaResult, MediaSkillError>;
}
