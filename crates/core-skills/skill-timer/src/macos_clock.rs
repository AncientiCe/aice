//! macOS Clock.app timer via System Events UI scripting.
//!
//! The `clock-timer://` URL scheme is iOS-only and does nothing on macOS.
//! The Clock app has no AppleScript dictionary, so the only programmatic path
//! is System Events UI scripting (Accessibility API).
//!
//! ## Permissions
//!
//! macOS requires the calling process to hold Accessibility permission
//! (System Settings → Privacy & Security → Accessibility).
//! If the permission is absent osascript returns a "not authorized" error,
//! which this skill surfaces verbatim so the user knows exactly what to fix.
//!
//! ## UI scripting strategy
//!
//! 1. Activate Clock.app and wait for its window.
//! 2. Click the "Timer" toolbar button (tries by name then by index).
//! 3. Find the three time-input fields (hours / minutes / seconds), clear each
//!    one with Cmd-A and type the desired value.
//! 4. Click the "Start" button.
//!
//! If any step fails the script propagates an AppleScript `error` string which
//! is returned as `TimerSkillError::Execution`.

use crate::types::{TimerResult, TimerSkill, TimerSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const TIMER_SKILL_EXECUTE_TOTAL: &str = "timer_skill_execute_total";
const TIMER_SKILL_ERRORS_TOTAL: &str = "timer_skill_errors_total";
const TIMER_SKILL_EXECUTE_DURATION_SECONDS: &str = "timer_skill_execute_duration_seconds";

/// macOS Clock.app timer skill using System Events UI scripting.
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

    /// Try to count active Clock timers via UI scripting. Returns None on any failure.
    fn try_count_active_timers(&self) -> Option<u64> {
        if self.dry_run || !cfg!(target_os = "macos") {
            return None;
        }
        let script = "tell application \"System Events\"\n\
                      if (name of every process) contains \"Clock\" then\n\
                      tell process \"Clock\"\n\
                      count (buttons of tab group 1 of window 1 \
                      whose name is \"Timer\" or name is \"Timers\")\n\
                      end tell\n\
                      else\n\
                      return 0\n\
                      end if\n\
                      end tell";
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    }

    /// Start a Clock.app timer for `seconds` using System Events UI scripting.
    ///
    /// The script uses two strategies in order:
    ///
    /// **Strategy A – text fields**: some macOS versions expose three
    /// `AXTextField` controls inside `group 1` of the timer window.
    ///
    /// **Strategy B – keyboard digit entry**: click the timer display area
    /// (coordinates captured *before* entering `tell window 1`, which avoids
    /// the "Can't get window 1 of window 1" -1728 error) then type HHMMSS.
    ///
    /// Returns `TimerSkillError::Execution` with a descriptive message on any
    /// failure, including a hint about Accessibility permissions when relevant.
    fn start_clock_timer(&self, seconds: u64) -> Result<(), TimerSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(TimerSkillError::Unavailable);
        }

        let hrs = seconds / 3600;
        let mins = (seconds % 3600) / 60;
        let secs = seconds % 60;

        // Pre-format as zero-padded 2-digit strings in Rust so the AppleScript
        // does not need to do string manipulation.
        let hrs_str = format!("{hrs:02}");
        let mins_str = format!("{mins:02}");
        let secs_str = format!("{secs:02}");
        let digits = format!("{hrs_str}{mins_str}{secs_str}");

        // Braces that must appear literally in AppleScript are doubled
        // ({{ → {, }} → }) by format!.
        let script = format!(
            r#"
tell application "Clock" to activate
delay 0.7

tell application "System Events"
    tell process "Clock"
        -- Wait for the main window
        set wc to 0
        repeat until (exists window 1) or wc > 30
            delay 0.1
            set wc to wc + 1
        end repeat
        if wc >= 30 then error "Clock window did not open"

        -- Click the Timer toolbar tab (name first, index 4 as fallback)
        try
            click button "Timer" of toolbar of window 1
        on error
            try
                click button 4 of toolbar of window 1
            on error
                error "Cannot find Timer tab in Clock toolbar"
            end try
        end try
        delay 0.6

        -- IMPORTANT: capture window bounds HERE, outside any `tell window 1`
        -- block. Inside `tell window 1`, writing `window 1` would look for a
        -- *child* window and raise error -1728.
        set winPos to position of window 1
        set winSz to size of window 1
        set winCx to (((item 1 of winPos) + (item 1 of winSz) / 2) as integer)
        -- Y: roughly the lower half of the content area (below the clock face)
        set winPickerY to (((item 2 of winPos) + (item 2 of winSz) * 3 / 4) as integer)

        -- Strategy A: text fields inside group 1 (works on some macOS versions)
        set didSet to false
        tell window 1
            try
                set f1 to text field 1 of group 1
                set f2 to text field 2 of group 1
                set f3 to text field 3 of group 1
                click f1
                delay 0.1
                keystroke "a" using command down
                keystroke "{hrs_str}"
                click f2
                delay 0.1
                keystroke "a" using command down
                keystroke "{mins_str}"
                click f3
                delay 0.1
                keystroke "a" using command down
                keystroke "{secs_str}"
                set didSet to true
            on error
                -- text fields not present on this macOS version; fall through
            end try
        end tell

        -- Strategy B: click the picker area then type HHMMSS digits.
        -- Uses the pre-captured screen coordinates (no window 1 reference).
        if not didSet then
            click at {{winCx, winPickerY}}
            delay 0.4
            -- Clear any existing value first
            repeat 6 times
                key code 51
                delay 0.05
            end repeat
            keystroke "{digits}"
            delay 0.2
        end if

        delay 0.3

        -- Click Start
        try
            click button "Start" of window 1
        on error
            error "Could not click the Start button in Clock"
        end try
    end tell
end tell
"#
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TimerSkillError::Execution(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let msg = if stderr.to_lowercase().contains("not authorized") {
                "Accessibility permission required: go to System Settings → Privacy & Security → Accessibility and enable the app.".to_string()
            } else if stderr.is_empty() {
                "Clock timer could not be started via UI scripting".to_string()
            } else {
                stderr
            };
            return Err(TimerSkillError::Execution(msg));
        }

        Ok(())
    }

    async fn execute_inner(
        &self,
        duration: &str,
        name: Option<&str>,
    ) -> Result<TimerResult, TimerSkillError> {
        let seconds = Self::parse_duration_seconds(duration)?;
        let active = self.try_count_active_timers().unwrap_or(0);
        let timer_name = match name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => {
                let ordinal = Self::ordinal_name(active + 1);
                format!("{ordinal} timer")
            }
        };
        if !self.dry_run {
            self.start_clock_timer(seconds)?;
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
        let result = self.execute_inner(duration, name).await;
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
