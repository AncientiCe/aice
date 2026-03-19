//! macOS volume skill via `osascript`.

use crate::types::{VolumeResult, VolumeSkill, VolumeSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const VOLUME_SKILL_EXECUTE_TOTAL: &str = "volume_skill_execute_total";
const VOLUME_SKILL_ERRORS_TOTAL: &str = "volume_skill_errors_total";
const VOLUME_SKILL_EXECUTE_DURATION_SECONDS: &str = "volume_skill_execute_duration_seconds";
const DEFAULT_STEP: u8 = 10;

#[derive(Clone)]
pub struct MacOsVolumeSkill {
    dry_run: bool,
}

impl MacOsVolumeSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn normalize_action(action: Option<&str>) -> String {
        action.unwrap_or("get").trim().to_ascii_lowercase()
    }

    fn validate_level(level: u8) -> Result<u8, VolumeSkillError> {
        if level > 100 {
            return Err(VolumeSkillError::InvalidLevel(level));
        }
        Ok(level)
    }

    fn run_osascript(&self, script: &str) -> Result<String, VolumeSkillError> {
        if self.dry_run {
            return Ok(String::new());
        }
        if !cfg!(target_os = "macos") {
            return Err(VolumeSkillError::Execution(
                "volume skill is only available on macOS".to_string(),
            ));
        }

        let output = Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| VolumeSkillError::Execution(e.to_string()))?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(VolumeSkillError::Execution(if stderr.is_empty() {
            "osascript command failed".to_string()
        } else {
            stderr
        }))
    }

    fn read_current_volume(&self) -> Result<u8, VolumeSkillError> {
        if self.dry_run {
            return Ok(50);
        }
        let out = self.run_osascript("output volume of (get volume settings)")?;
        out.parse::<u8>()
            .map_err(|e| VolumeSkillError::Execution(format!("failed to parse volume: {e}")))
            .and_then(Self::validate_level)
    }

    fn set_volume(&self, level: u8) -> Result<(), VolumeSkillError> {
        Self::validate_level(level)?;
        let script = format!("set volume output volume {level}");
        self.run_osascript(&script)?;
        Ok(())
    }

    async fn execute_inner(
        &self,
        action: Option<&str>,
        level: Option<u8>,
    ) -> Result<VolumeResult, VolumeSkillError> {
        let action = Self::normalize_action(action);
        match action.as_str() {
            "set" => {
                let value = level.ok_or_else(|| {
                    VolumeSkillError::Execution("level must be provided for set action".to_string())
                })?;
                let value = Self::validate_level(value)?;
                self.set_volume(value)?;
                Ok(VolumeResult {
                    summary: format!("Done. I set the volume to {value}."),
                    action_done: format!("set volume output volume {value}"),
                    resulting_level: Some(value),
                })
            }
            "up" => {
                let current = if self.dry_run {
                    0
                } else {
                    self.read_current_volume()?
                };
                let next = current.saturating_add(DEFAULT_STEP).min(100);
                self.set_volume(next)?;
                Ok(VolumeResult {
                    summary: format!("Done. I increased the volume to {next}."),
                    action_done: format!("set volume output volume {next}"),
                    resulting_level: Some(next),
                })
            }
            "down" => {
                let current = if self.dry_run {
                    0
                } else {
                    self.read_current_volume()?
                };
                let next = current.saturating_sub(DEFAULT_STEP);
                self.set_volume(next)?;
                Ok(VolumeResult {
                    summary: format!("Done. I decreased the volume to {next}."),
                    action_done: format!("set volume output volume {next}"),
                    resulting_level: Some(next),
                })
            }
            "mute" => {
                self.run_osascript("set volume output muted true")?;
                Ok(VolumeResult {
                    summary: "Done. I muted the volume.".to_string(),
                    action_done: "set volume output muted true".to_string(),
                    resulting_level: None,
                })
            }
            "unmute" => {
                self.run_osascript("set volume output muted false")?;
                Ok(VolumeResult {
                    summary: "Done. I unmuted the volume.".to_string(),
                    action_done: "set volume output muted false".to_string(),
                    resulting_level: None,
                })
            }
            "get" => {
                let current = self.read_current_volume()?;
                Ok(VolumeResult {
                    summary: format!("The current volume is {current}."),
                    action_done: "output volume of (get volume settings)".to_string(),
                    resulting_level: Some(current),
                })
            }
            other => Err(VolumeSkillError::UnsupportedAction(other.to_string())),
        }
    }
}

impl Default for MacOsVolumeSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VolumeSkill for MacOsVolumeSkill {
    async fn execute(
        &self,
        action: Option<&str>,
        level: Option<u8>,
    ) -> Result<VolumeResult, VolumeSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(action, level).await;
        match &result {
            Ok(_) => {
                counter!(VOLUME_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(VOLUME_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(VOLUME_SKILL_ERRORS_TOTAL, 1, "kind" => e.to_string());
            }
        }
        histogram!(
            VOLUME_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}
