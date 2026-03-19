//! macOS computer-use skill via `open`.

use crate::types::{ComputerResult, ComputerSkill, ComputerSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const COMPUTER_SKILL_EXECUTE_TOTAL: &str = "computer_skill_execute_total";
const COMPUTER_SKILL_ERRORS_TOTAL: &str = "computer_skill_errors_total";
const COMPUTER_SKILL_EXECUTE_DURATION_SECONDS: &str = "computer_skill_execute_duration_seconds";

/// macOS computer skill that opens apps, files, or URLs.
#[derive(Clone)]
pub struct MacOsComputerSkill {
    dry_run: bool,
}

impl MacOsComputerSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn normalize_action(action: Option<&str>) -> String {
        action.unwrap_or("open").trim().to_ascii_lowercase()
    }

    fn normalize_target(target: Option<&str>) -> Result<String, ComputerSkillError> {
        let Some(target) = target else {
            return Err(ComputerSkillError::Execution(
                "target must be provided".to_string(),
            ));
        };
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err(ComputerSkillError::Execution(
                "target must not be empty".to_string(),
            ));
        }
        Ok(trimmed.to_string())
    }

    fn is_file_target(target: &str) -> bool {
        target.starts_with('/') || target.starts_with("~/")
    }

    fn is_url_target(target: &str) -> bool {
        target.starts_with("http://") || target.starts_with("https://")
    }

    fn action_requires_url(action: &str) -> bool {
        matches!(action, "open_url" | "browse" | "open_browser")
    }

    fn normalize_url_target(target: &str) -> String {
        if Self::is_url_target(target) {
            target.to_string()
        } else {
            format!("https://{}", target.trim_start_matches('/'))
        }
    }

    fn build_open_command(action: &str, target: &str) -> (String, Vec<String>, String) {
        if Self::action_requires_url(action) || Self::is_url_target(target) {
            let normalized = Self::normalize_url_target(target);
            return (
                "open".to_string(),
                vec![normalized.clone()],
                format!("open \"{normalized}\""),
            );
        }

        if Self::is_file_target(target) {
            return (
                "open".to_string(),
                vec![target.to_string()],
                format!("open \"{target}\""),
            );
        }

        (
            "open".to_string(),
            vec!["-a".to_string(), target.to_string()],
            format!("open -a \"{target}\""),
        )
    }

    fn run_open_command(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<Option<String>, ComputerSkillError> {
        if self.dry_run {
            return Ok(None);
        }
        if !cfg!(target_os = "macos") {
            return Err(ComputerSkillError::Execution(
                "computer skill is only available on macOS".to_string(),
            ));
        }

        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| ComputerSkillError::Execution(e.to_string()))?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok((!stdout.is_empty()).then_some(stdout));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(ComputerSkillError::Execution(if stderr.is_empty() {
            "open command failed".to_string()
        } else {
            stderr
        }))
    }

    async fn execute_inner(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<ComputerResult, ComputerSkillError> {
        let action = Self::normalize_action(action);
        let target = Self::normalize_target(target)?;
        let (program, args, action_done) = Self::build_open_command(&action, &target);
        let output = self.run_open_command(&program, &args)?;

        Ok(ComputerResult {
            summary: format!("Done. I opened {}.", target),
            action_done,
            output,
        })
    }
}

impl Default for MacOsComputerSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComputerSkill for MacOsComputerSkill {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<ComputerResult, ComputerSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(action, target).await;
        match &result {
            Ok(_) => {
                counter!(COMPUTER_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(COMPUTER_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(COMPUTER_SKILL_ERRORS_TOTAL, 1, "kind" => e.to_string());
            }
        }
        histogram!(
            COMPUTER_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::MacOsComputerSkill;
    use crate::types::{ComputerSkill, ComputerSkillError};

    #[tokio::test]
    async fn dry_run_open_app_succeeds() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill.execute(Some("open"), Some("GoLand")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dry_run_open_url_by_scheme_succeeds() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill
            .execute(Some("open"), Some("https://github.com"))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dry_run_open_url_by_action_succeeds() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill
            .execute(Some("open_browser"), Some("github.com"))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dry_run_open_file_by_path_succeeds() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill.execute(Some("open"), Some("/Users/x/doc.pdf")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn empty_target_returns_execution_error() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill.execute(Some("open"), Some("")).await;
        assert!(matches!(result, Err(ComputerSkillError::Execution(_))));
    }

    #[tokio::test]
    async fn missing_target_returns_execution_error() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill.execute(Some("open"), None).await;
        assert!(matches!(result, Err(ComputerSkillError::Execution(_))));
    }

    #[tokio::test]
    async fn action_launch_treated_as_open_app() {
        let skill = MacOsComputerSkill::new_for_tests();
        let result = skill.execute(Some("launch"), Some("Spotify")).await;
        let result = result.expect("should be ok");
        assert!(result.action_done.contains("open -a"));
    }
}
