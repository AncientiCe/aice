//! macOS Reminders.app integration via AppleScript.

use crate::types::{ReminderResult, ReminderSkill, ReminderSkillError};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDateTime, Timelike};
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const REMINDER_SKILL_EXECUTE_TOTAL: &str = "reminder_skill_execute_total";
const REMINDER_SKILL_ERRORS_TOTAL: &str = "reminder_skill_errors_total";
const REMINDER_SKILL_EXECUTE_DURATION_SECONDS: &str = "reminder_skill_execute_duration_seconds";

/// macOS Reminders.app skill via AppleScript.
#[derive(Clone)]
pub struct MacOsReminderSkill {
    dry_run: bool,
}

impl MacOsReminderSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn escape_applescript_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Parse an ISO 8601 date-time string (e.g. "2026-03-20T17:00" or "2026-03-20 17:00").
    fn parse_iso_datetime(when: &str) -> Option<NaiveDateTime> {
        let normalized = when.trim().replace('T', " ");
        // Try full datetime
        if let Ok(dt) = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S") {
            return Some(dt);
        }
        if let Ok(dt) = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M") {
            return Some(dt);
        }
        // Try date-only → midnight
        if let Ok(d) = chrono::NaiveDate::parse_from_str(normalized.trim(), "%Y-%m-%d") {
            return d.and_hms_opt(0, 0, 0);
        }
        None
    }

    /// Build the AppleScript to create a reminder, optionally with a due date.
    fn build_create_script(title: &str, dt: Option<NaiveDateTime>) -> String {
        let escaped_title = Self::escape_applescript_string(title);
        match dt {
            None => format!(
                "tell application \"Reminders\"\n\
                 set theList to default list\n\
                 make new reminder at end of reminders of theList \
                 with properties {{name:\"{escaped_title}\"}}\n\
                 end tell"
            ),
            Some(dt) => {
                let year = dt.year();
                let month = dt.month();
                let day = dt.day();
                let hour = dt.hour();
                let minute = dt.minute();
                let second = dt.second();
                format!(
                    "tell application \"Reminders\"\n\
                     set theList to default list\n\
                     set r to make new reminder at end of reminders of theList \
                     with properties {{name:\"{escaped_title}\"}}\n\
                     set dueDate to current date\n\
                     set year of dueDate to {year}\n\
                     set month of dueDate to {month}\n\
                     set day of dueDate to {day}\n\
                     set hours of dueDate to {hour}\n\
                     set minutes of dueDate to {minute}\n\
                     set seconds of dueDate to {second}\n\
                     set due date of r to dueDate\n\
                     set remind me date of r to dueDate\n\
                     end tell"
                )
            }
        }
    }

    fn run_script(&self, script: &str) -> Result<String, ReminderSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(ReminderSkillError::Unavailable);
        }
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| ReminderSkillError::Execution(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ReminderSkillError::Execution(if stderr.is_empty() {
                "osascript failed".to_string()
            } else {
                stderr
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn execute_inner(
        &self,
        title: &str,
        when: Option<&str>,
    ) -> Result<ReminderResult, ReminderSkillError> {
        if title.trim().is_empty() {
            return Err(ReminderSkillError::Execution(
                "reminder title must not be empty".to_string(),
            ));
        }
        let dt = match when {
            None => None,
            Some(w) => {
                let parsed = Self::parse_iso_datetime(w);
                if parsed.is_none() {
                    return Err(ReminderSkillError::InvalidDate(w.to_string()));
                }
                parsed
            }
        };

        if !self.dry_run {
            let script = Self::build_create_script(title, dt);
            self.run_script(&script)?;
        }

        let when_display = dt.map(|d| d.format("%d %b %Y at %H:%M").to_string());
        let summary = match &when_display {
            Some(w) => format!("Reminder '{}' created for {}", title, w),
            None => format!("Reminder '{}' created without due date", title),
        };
        Ok(ReminderResult {
            summary,
            title: title.to_string(),
            when: when_display,
        })
    }
}

impl Default for MacOsReminderSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReminderSkill for MacOsReminderSkill {
    async fn execute(
        &self,
        title: &str,
        when: Option<&str>,
    ) -> Result<ReminderResult, ReminderSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(title, when).await;
        match &result {
            Ok(_) => {
                counter!(REMINDER_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(REMINDER_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(
                    REMINDER_SKILL_ERRORS_TOTAL,
                    1,
                    "kind" => e.to_string()
                );
            }
        }
        histogram!(
            REMINDER_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::MacOsReminderSkill;
    use chrono::{NaiveDateTime, Timelike};

    #[test]
    fn escape_applescript_string_escapes_quotes_and_backslashes() {
        let s = MacOsReminderSkill::escape_applescript_string("call \"mom\" on \\n");
        assert_eq!(s, "call \\\"mom\\\" on \\\\n");
    }

    #[test]
    fn parse_iso_datetime_handles_full_datetime() {
        let dt = MacOsReminderSkill::parse_iso_datetime("2026-03-20T17:00");
        assert!(dt.is_some());
        let dt = dt.unwrap();
        assert_eq!(dt.hour(), 17);
        assert_eq!(dt.minute(), 0);
    }

    #[test]
    fn parse_iso_datetime_handles_date_only_as_midnight() {
        let dt = MacOsReminderSkill::parse_iso_datetime("2026-03-20");
        assert!(dt.is_some());
        let dt = dt.unwrap();
        assert_eq!(dt.hour(), 0);
        assert_eq!(dt.minute(), 0);
    }

    #[test]
    fn parse_iso_datetime_returns_none_for_garbage() {
        assert!(MacOsReminderSkill::parse_iso_datetime("tomorrow at 5pm").is_none());
    }

    #[test]
    fn build_create_script_without_date_omits_due_date() {
        let script = MacOsReminderSkill::build_create_script("Buy milk", None);
        assert!(script.contains("Buy milk"));
        assert!(!script.contains("due date"));
    }

    #[test]
    fn build_create_script_with_date_includes_date_components() {
        let dt = NaiveDateTime::parse_from_str("2026-03-20 17:00", "%Y-%m-%d %H:%M").unwrap();
        let script = MacOsReminderSkill::build_create_script("Buy milk", Some(dt));
        assert!(script.contains("Buy milk"));
        assert!(script.contains("due date"));
        assert!(script.contains("2026"));
        assert!(script.contains("17"));
    }
}
