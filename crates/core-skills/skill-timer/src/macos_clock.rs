//! In-memory macOS timers with native notification + sound on expiry.
//!
//! ## Design
//!
//! Each timer is a fully-detached background shell process:
//!
//! ```text
//! nohup /bin/sh -c 'sleep N; /usr/bin/osascript <script>; rm <script>' &
//! ```
//!
//! `nohup` ensures the process survives even if the parent application exits
//! before the timer fires. When the countdown ends, an AppleScript notification
//! is displayed and the Glass sound plays — identical in feel to a Clock timer
//! alert but without requiring Accessibility permissions or Clock.app to be
//! open.
//!
//! ## Timer names
//!
//! If the user names the timer (e.g. "pasta timer") that name appears in the
//! notification. Otherwise the timer is named by ordinal position within the
//! current session ("first timer", "second timer", …).

use crate::types::{TimerResult, TimerSkill, TimerSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::process::{Command, Stdio};
use std::time::Instant;

const TIMER_SKILL_EXECUTE_TOTAL: &str = "timer_skill_execute_total";
const TIMER_SKILL_ERRORS_TOTAL: &str = "timer_skill_errors_total";
const TIMER_SKILL_EXECUTE_DURATION_SECONDS: &str = "timer_skill_execute_duration_seconds";

/// In-memory macOS timer that fires a notification + sound on expiry.
#[derive(Clone)]
pub struct MacOsClockTimerSkill {
    dry_run: bool,
}

impl MacOsClockTimerSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    fn escape_applescript_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Parse a natural duration string into total seconds.
    /// Handles patterns like "5 minutes", "1 hour 30 minutes", "90 seconds".
    pub fn parse_duration_seconds(duration: &str) -> Result<u64, TimerSkillError> {
        let s = duration.to_lowercase();
        let mut total: u64 = 0;
        let mut found = false;

        let mut remaining = s.as_str();
        while !remaining.is_empty() {
            let stripped = remaining
                .trim_start_matches(|c: char| c.is_whitespace() || c == ',')
                .strip_prefix("and")
                .map(|s| s.trim_start())
                .unwrap_or(remaining.trim_start_matches(|c: char| c.is_whitespace() || c == ','));
            if stripped.is_empty() {
                break;
            }

            let (num_str, rest) = Self::take_number(stripped);
            if num_str.is_empty() {
                break;
            }
            let n: u64 = num_str
                .parse()
                .map_err(|_| TimerSkillError::InvalidDuration(duration.to_string()))?;

            let rest = rest.trim_start();

            if rest.starts_with("hour") {
                total += n * 3600;
                found = true;
                remaining = Self::skip_word(rest);
            } else if rest.starts_with("minute") || rest.starts_with("min") {
                total += n * 60;
                found = true;
                remaining = Self::skip_word(rest);
            } else if rest.starts_with("second") || rest.starts_with("sec") {
                total += n;
                found = true;
                remaining = Self::skip_word(rest);
            } else {
                break;
            }
        }

        if !found || total == 0 {
            return Err(TimerSkillError::InvalidDuration(duration.to_string()));
        }
        Ok(total)
    }

    fn take_number(s: &str) -> (&str, &str) {
        let end = s
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_digit())
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        (&s[..end], &s[end..])
    }

    fn skip_word(s: &str) -> &str {
        s.char_indices()
            .find(|(_, c)| c.is_whitespace() || *c == ',')
            .map(|(i, _)| s[i..].trim_start_matches(|c: char| c.is_whitespace() || c == ','))
            .unwrap_or("")
    }

    /// Format seconds into a human-readable string like "5 minutes".
    pub fn format_duration(seconds: u64) -> String {
        if seconds == 0 {
            return "0 seconds".to_string();
        }
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        let mut parts = Vec::new();
        if hours > 0 {
            parts.push(format!(
                "{} {}",
                hours,
                if hours == 1 { "hour" } else { "hours" }
            ));
        }
        if minutes > 0 {
            parts.push(format!(
                "{} {}",
                minutes,
                if minutes == 1 { "minute" } else { "minutes" }
            ));
        }
        if secs > 0 {
            parts.push(format!(
                "{} {}",
                secs,
                if secs == 1 { "second" } else { "seconds" }
            ));
        }
        parts.join(" ")
    }

    /// Return an ordinal word for a 1-based position.
    pub fn ordinal_name(n: u64) -> String {
        match n {
            1 => "first".to_string(),
            2 => "second".to_string(),
            3 => "third".to_string(),
            4 => "fourth".to_string(),
            5 => "fifth".to_string(),
            6 => "sixth".to_string(),
            7 => "seventh".to_string(),
            8 => "eighth".to_string(),
            9 => "ninth".to_string(),
            10 => "tenth".to_string(),
            n => format!("{n}th"),
        }
    }

    /// Spawn a fully-detached background timer.
    ///
    /// Writes a small AppleScript to a temp file (side-steps multi-level shell
    /// escaping) then runs:
    ///
    /// ```text
    /// nohup /bin/sh -c 'sleep N; /usr/bin/osascript <file>; rm <file>' &
    /// ```
    ///
    /// The AppleScript plays the Glass sound and shows a macOS notification.
    /// `nohup` ensures delivery even if the voice-assistant process exits first.
    fn start_timer(&self, seconds: u64, name: &str) -> Result<(), TimerSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(TimerSkillError::Unavailable);
        }

        let escaped_name = Self::escape_applescript_string(&format!("{name} done"));

        // Write the notification script to a unique temp file.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let script_path = std::env::temp_dir().join(format!("aice_timer_{ts}.scpt"));

        // Play the Glass sound then show a dialog.
        //
        // `display notification` from a detached nohup process requires the
        // osascript binary to have notification permission in System Settings,
        // which most users haven't granted — it silently drops the notification.
        //
        // `display dialog` bypasses the Notification Center entirely: it
        // renders a window directly via the Script Editor GUI context and is
        // always visible from any process. `giving up after 60` auto-dismisses
        // if the user doesn't click OK within a minute.
        let script = format!(
            "do shell script \"/usr/bin/afplay /System/Library/Sounds/Glass.aiff\"\n\
             display dialog \"\u{23F0} {escaped_name}\" buttons {{\"OK\"}} giving up after 60"
        );

        std::fs::write(&script_path, &script).map_err(|e| {
            TimerSkillError::Execution(format!("failed to write timer script: {e}"))
        })?;

        let path = script_path.to_string_lossy().to_string();

        // nohup detaches the child from this process's session so it continues
        // running even after the voice-assistant process exits.
        Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "nohup /bin/sh -c \
                 'sleep {seconds}; /usr/bin/osascript \"{path}\"; rm -f \"{path}\"' \
                 >/dev/null 2>&1 &"
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| TimerSkillError::Execution(format!("failed to spawn timer: {e}")))?;

        Ok(())
    }

    async fn execute_inner(
        &self,
        duration: &str,
        name: Option<&str>,
        // session_count is the number of timers already started this session,
        // used for ordinal naming when no explicit name is given.
        session_count: u64,
    ) -> Result<TimerResult, TimerSkillError> {
        let seconds = Self::parse_duration_seconds(duration)?;
        let timer_name = match name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => format!("{} timer", Self::ordinal_name(session_count + 1)),
        };
        if !self.dry_run {
            self.start_timer(seconds, &timer_name)?;
        }
        let duration_display = Self::format_duration(seconds);
        Ok(TimerResult {
            summary: format!("Timer '{}' started for {}", timer_name, duration_display),
            timer_name,
            duration_display,
            duration_seconds: seconds,
        })
    }
}

impl Default for MacOsClockTimerSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TimerSkill for MacOsClockTimerSkill {
    async fn execute(
        &self,
        duration: &str,
        name: Option<&str>,
    ) -> Result<TimerResult, TimerSkillError> {
        let t0 = Instant::now();
        // session_count = 0: unnamed timers are always "first timer" within a
        // single invocation. The caller (runtime) can pass a counter if needed.
        let result = self.execute_inner(duration, name, 0).await;
        match &result {
            Ok(_) => {
                counter!(TIMER_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(TIMER_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(
                    TIMER_SKILL_ERRORS_TOTAL,
                    1,
                    "kind" => e.to_string()
                );
            }
        }
        histogram!(
            TIMER_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::MacOsClockTimerSkill;

    #[test]
    fn parse_duration_five_minutes() {
        assert_eq!(
            MacOsClockTimerSkill::parse_duration_seconds("5 minutes").unwrap(),
            300
        );
    }

    #[test]
    fn parse_duration_one_hour() {
        assert_eq!(
            MacOsClockTimerSkill::parse_duration_seconds("1 hour").unwrap(),
            3600
        );
    }

    #[test]
    fn parse_duration_one_hour_thirty_minutes() {
        assert_eq!(
            MacOsClockTimerSkill::parse_duration_seconds("1 hour 30 minutes").unwrap(),
            5400
        );
    }

    #[test]
    fn parse_duration_ninety_seconds() {
        assert_eq!(
            MacOsClockTimerSkill::parse_duration_seconds("90 seconds").unwrap(),
            90
        );
    }

    #[test]
    fn parse_duration_complex() {
        assert_eq!(
            MacOsClockTimerSkill::parse_duration_seconds("2 hours 15 minutes 30 seconds").unwrap(),
            2 * 3600 + 15 * 60 + 30
        );
    }

    #[test]
    fn parse_duration_invalid_returns_error() {
        assert!(MacOsClockTimerSkill::parse_duration_seconds("tomorrow").is_err());
        assert!(MacOsClockTimerSkill::parse_duration_seconds("").is_err());
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(MacOsClockTimerSkill::format_duration(300), "5 minutes");
    }

    #[test]
    fn format_duration_hour_and_half() {
        assert_eq!(
            MacOsClockTimerSkill::format_duration(5400),
            "1 hour 30 minutes"
        );
    }

    #[test]
    fn ordinal_name_first_through_tenth() {
        assert_eq!(MacOsClockTimerSkill::ordinal_name(1), "first");
        assert_eq!(MacOsClockTimerSkill::ordinal_name(2), "second");
        assert_eq!(MacOsClockTimerSkill::ordinal_name(3), "third");
        assert_eq!(MacOsClockTimerSkill::ordinal_name(10), "tenth");
    }

    #[test]
    fn ordinal_name_fallback_for_large_n() {
        assert_eq!(MacOsClockTimerSkill::ordinal_name(11), "11th");
    }
}
