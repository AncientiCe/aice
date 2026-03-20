//! macOS app switcher skill via AppleScript and local shell commands.

use crate::types::{AppSwitcherResult, AppSwitcherSkill, AppSwitcherSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const APP_SWITCHER_SKILL_EXECUTE_TOTAL: &str = "app_switcher_skill_execute_total";
const APP_SWITCHER_SKILL_ERRORS_TOTAL: &str = "app_switcher_skill_errors_total";
const APP_SWITCHER_SKILL_EXECUTE_DURATION_SECONDS: &str =
    "app_switcher_skill_execute_duration_seconds";

#[derive(Clone)]
pub struct MacOsAppSwitcherSkill {
    dry_run: bool,
}

impl MacOsAppSwitcherSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn normalize_action(action: Option<&str>) -> String {
        let normalized = action.unwrap_or("switch").trim().to_ascii_lowercase();
        match normalized.as_str() {
            "close" | "exit" => "quit".to_string(),
            _ => normalized,
        }
    }

    fn normalize_target(target: Option<&str>) -> Option<String> {
        target
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    }

    fn requires_target(action: &str) -> bool {
        matches!(action, "switch" | "hide" | "quit" | "force_quit")
    }

    fn escape_applescript_string(input: &str) -> String {
        input.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn script_for_action(
        action: &str,
        target: Option<&str>,
    ) -> Result<(String, String, Option<String>), AppSwitcherSkillError> {
        match action {
            "switch" => {
                let target = target.ok_or_else(|| {
                    AppSwitcherSkillError::Execution("target must be provided for switch".to_string())
                })?;
                let escaped = Self::escape_applescript_string(target);
                Ok((
                    format!("tell application \"{escaped}\" to activate"),
                    format!("activate {target}"),
                    Some(target.to_string()),
                ))
            }
            "next" => Ok((
                "tell application \"System Events\" to key code 48 using {command down}".to_string(),
                "app switcher next".to_string(),
                None,
            )),
            "previous" => Ok((
                "tell application \"System Events\" to key code 48 using {command down, shift down}".to_string(),
                "app switcher previous".to_string(),
                None,
            )),
            "hide" => {
                let target = target.ok_or_else(|| {
                    AppSwitcherSkillError::Execution("target must be provided for hide".to_string())
                })?;
                let escaped = Self::escape_applescript_string(target);
                Ok((
                    format!("tell application \"{escaped}\" to hide"),
                    format!("hide {target}"),
                    Some(target.to_string()),
                ))
            }
            "hide_others" => Ok((
                "tell application \"System Events\" to keystroke \"h\" using {command down, option down}".to_string(),
                "hide others".to_string(),
                None,
            )),
            "show_all_windows" => Ok((
                "tell application \"System Events\" to key code 126 using {control down}".to_string(),
                "show all windows".to_string(),
                None,
            )),
            "quit" => {
                let target = target.ok_or_else(|| {
                    AppSwitcherSkillError::Execution("target must be provided for quit".to_string())
                })?;
                let escaped = Self::escape_applescript_string(target);
                Ok((
                    format!("tell application \"{escaped}\" to quit"),
                    format!("quit {target}"),
                    Some(target.to_string()),
                ))
            }
            "force_quit" => {
                let target = target.ok_or_else(|| {
                    AppSwitcherSkillError::Execution(
                        "target must be provided for force_quit".to_string(),
                    )
                })?;
                let escaped = Self::escape_applescript_string(target);
                Ok((
                    format!("do shell script \"killall -9 \" & quoted form of \"{escaped}\""),
                    format!("force quit {target}"),
                    Some(target.to_string()),
                ))
            }
            other => Err(AppSwitcherSkillError::UnsupportedAction(other.to_string())),
        }
    }

    fn run_osascript(&self, script: &str) -> Result<(), AppSwitcherSkillError> {
        if self.dry_run {
            return Ok(());
        }
        if !cfg!(target_os = "macos") {
            return Err(AppSwitcherSkillError::Execution(
                "app switcher skill is only available on macOS".to_string(),
            ));
        }

        let output = Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| AppSwitcherSkillError::Execution(e.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(AppSwitcherSkillError::Execution(if stderr.is_empty() {
            "osascript command failed".to_string()
        } else {
            stderr
        }))
    }

    async fn execute_inner(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<AppSwitcherResult, AppSwitcherSkillError> {
        let action = Self::normalize_action(action);
        let target = Self::normalize_target(target);

        if Self::requires_target(&action) && target.is_none() {
            return Err(AppSwitcherSkillError::Execution(format!(
                "target must be provided for {action}"
            )));
        }

        let (script, action_done, target) = Self::script_for_action(&action, target.as_deref())?;
        self.run_osascript(&script)?;

        Ok(AppSwitcherResult {
            summary: match target.as_deref() {
                Some(name) => format!("Done. I {} {}.", action.replace('_', " "), name),
                None => format!("Done. I {}.", action.replace('_', " ")),
            },
            action_done,
            target,
        })
    }
}

impl Default for MacOsAppSwitcherSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AppSwitcherSkill for MacOsAppSwitcherSkill {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<AppSwitcherResult, AppSwitcherSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(action, target).await;
        match &result {
            Ok(_) => {
                counter!(APP_SWITCHER_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(APP_SWITCHER_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(APP_SWITCHER_SKILL_ERRORS_TOTAL, 1, "kind" => e.to_string());
            }
        }
        histogram!(
            APP_SWITCHER_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}
