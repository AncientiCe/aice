//! macOS screenshot skill via `screencapture`.

use crate::types::{ScreenshotResult, ScreenshotSkill, ScreenshotSkillError};
use async_trait::async_trait;
use chrono::Local;
use metrics::{counter, histogram};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const SCREENSHOT_SKILL_EXECUTE_TOTAL: &str = "screenshot_skill_execute_total";
const SCREENSHOT_SKILL_ERRORS_TOTAL: &str = "screenshot_skill_errors_total";
const SCREENSHOT_SKILL_EXECUTE_DURATION_SECONDS: &str = "screenshot_skill_execute_duration_seconds";

#[derive(Clone)]
pub struct MacOsScreenshotSkill {
    dry_run: bool,
}

impl MacOsScreenshotSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn default_filename() -> String {
        format!("screenshot-{}.png", Local::now().format("%Y-%m-%d-%H%M%S"))
    }

    fn normalize_filename(filename: Option<&str>) -> String {
        let Some(filename) = filename else {
            return Self::default_filename();
        };
        let trimmed = filename.trim();
        if trimmed.is_empty() {
            return Self::default_filename();
        }
        trimmed.to_string()
    }

    fn resolve_output_path(filename: Option<&str>) -> Result<PathBuf, ScreenshotSkillError> {
        let home = env::var_os("HOME").ok_or_else(|| {
            ScreenshotSkillError::Execution(
                "HOME is not set, cannot resolve screenshot path".to_string(),
            )
        })?;
        let output_dir = PathBuf::from(home).join("Pictures").join("aice");
        let filename = Self::normalize_filename(filename);
        Ok(output_dir.join(filename))
    }

    fn run_screencapture(&self, path: &Path) -> Result<(), ScreenshotSkillError> {
        if self.dry_run {
            return Ok(());
        }
        if !cfg!(target_os = "macos") {
            return Err(ScreenshotSkillError::Execution(
                "screenshot skill is only available on macOS".to_string(),
            ));
        }

        let output = Command::new("screencapture")
            .args(["-x", path.to_string_lossy().as_ref()])
            .output()
            .map_err(|e| ScreenshotSkillError::Execution(e.to_string()))?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(ScreenshotSkillError::Execution(if stderr.is_empty() {
            "screencapture command failed".to_string()
        } else {
            stderr
        }))
    }

    async fn execute_inner(
        &self,
        filename: Option<&str>,
    ) -> Result<ScreenshotResult, ScreenshotSkillError> {
        let path = Self::resolve_output_path(filename)?;
        let output_dir = path.parent().ok_or_else(|| {
            ScreenshotSkillError::Execution(
                "failed to resolve screenshot output directory".to_string(),
            )
        })?;
        if !self.dry_run {
            fs::create_dir_all(output_dir)
                .map_err(|e| ScreenshotSkillError::Execution(e.to_string()))?;
        }
        self.run_screencapture(&path)?;
        Ok(ScreenshotResult { path })
    }
}

impl Default for MacOsScreenshotSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScreenshotSkill for MacOsScreenshotSkill {
    async fn execute(
        &self,
        filename: Option<&str>,
    ) -> Result<ScreenshotResult, ScreenshotSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(filename).await;
        match &result {
            Ok(_) => {
                counter!(SCREENSHOT_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(SCREENSHOT_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(SCREENSHOT_SKILL_ERRORS_TOTAL, 1, "kind" => e.to_string());
            }
        }
        histogram!(
            SCREENSHOT_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}
