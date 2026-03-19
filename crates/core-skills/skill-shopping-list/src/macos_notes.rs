//! Apple Notes shopping list integration via AppleScript.
//!
//! ## Format contract
//!
//! **Writing**: note body is stored as a single-line HTML checklist that Apple
//! Notes renders as its native interactive checklist:
//!
//!   ```html
//!   <ul class="apple-converted-space">
//!     <li style="list-style-type:none"><input type="checkbox" disabled="">milk</li>
//!     ...
//!   </ul>
//!   ```
//!
//! **Reading**: Apple Notes returns the note `body` via AppleScript as plain
//! text (one item per line, no checkbox markers). We also accept our own HTML
//! format (strips tags) and the legacy `□`/`☑` prefixes from the previous
//! implementation so that existing notes continue to work.
//!
//! ## Title vs content
//!
//! The note `name` (title) is set explicitly via the AppleScript `name`
//! property. The `body` contains **only** the checklist items — the title is
//! never duplicated inside the body.

use crate::types::{ShoppingListResult, ShoppingListSkill, ShoppingListSkillError};
use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate};
use metrics::{counter, histogram};
use std::process::Command;
use std::time::Instant;

const SHOPPING_LIST_SKILL_EXECUTE_TOTAL: &str = "shopping_list_skill_execute_total";
const SHOPPING_LIST_SKILL_ERRORS_TOTAL: &str = "shopping_list_skill_errors_total";
const SHOPPING_LIST_SKILL_EXECUTE_DURATION_SECONDS: &str =
    "shopping_list_skill_execute_duration_seconds";

const NOT_FOUND_SENTINEL: &str = "AICE_NOTE_NOT_FOUND";

/// Apple Notes shopping list skill via AppleScript.
#[derive(Clone)]
pub struct MacOsNotesShoppingListSkill {
    dry_run: bool,
}

impl MacOsNotesShoppingListSkill {
    pub fn new() -> Self {
        Self { dry_run: false }
    }

    pub fn new_for_tests() -> Self {
        Self { dry_run: true }
    }

    /// Resolve a natural `when` string to a `NaiveDate`. Defaults to today.
    pub fn resolve_date(when: Option<&str>) -> NaiveDate {
        let today = Local::now().date_naive();
        match when {
            None | Some("today") => today,
            Some("tomorrow") => today + chrono::Duration::days(1),
            Some(s) => {
                if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                    return d;
                }
                let lower = s.to_lowercase();
                match lower.as_str() {
                    "monday" | "mon" => Self::next_weekday(today, 0),
                    "tuesday" | "tue" => Self::next_weekday(today, 1),
                    "wednesday" | "wed" => Self::next_weekday(today, 2),
                    "thursday" | "thu" => Self::next_weekday(today, 3),
                    "friday" | "fri" => Self::next_weekday(today, 4),
                    "saturday" | "sat" => Self::next_weekday(today, 5),
                    "sunday" | "sun" => Self::next_weekday(today, 6),
                    _ => today,
                }
            }
        }
    }

    fn next_weekday(from: NaiveDate, target_weekday: i64) -> NaiveDate {
        use chrono::Datelike;
        let current = from.weekday().num_days_from_monday() as i64;
        let days_ahead = (target_weekday - current).rem_euclid(7);
        let days_ahead = if days_ahead == 0 { 7 } else { days_ahead };
        from + chrono::Duration::days(days_ahead)
    }

    /// Format a date as "DD Mon YYYY" (e.g. "19 Mar 2026").
    pub fn format_note_date(date: NaiveDate) -> String {
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let month_str = months[(date.month() as usize).saturating_sub(1)];
        format!("{} {} {}", date.day(), month_str, date.year())
    }

    /// Build the note title for a given date.
    pub fn note_title(date: NaiveDate) -> String {
        format!("Shopping List {}", Self::format_note_date(date))
    }

    /// Parse a comma/conjunction-separated items string into a list of trimmed, non-empty items.
    pub fn parse_items(items: &str) -> Vec<String> {
        items
            .split(',')
            .flat_map(|part| part.split(" and "))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Normalize an item name for case-insensitive comparison (lowercase, trimmed).
    pub fn normalize_item(item: &str) -> String {
        item.trim().to_lowercase()
    }

    /// Strip HTML tags from a string (simple single-pass, no crate needed).
    fn strip_html_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// Decode HTML entities we might have encoded ourselves.
    fn decode_html_entities(s: &str) -> String {
        s.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
    }

    /// Extract item names from a note body.
    ///
    /// Accepts three formats:
    /// 1. **HTML checklist** (what we write): `<li ...><input ...>item</li>` → insert `\n`
    ///    before each `</li>` then strip tags.
    /// 2. **Legacy text checkboxes**: `□ item` / `☑ item` lines.
    /// 3. **Plain text** (what Notes returns when reading): one item per line.
    pub fn items_from_body(body: &str) -> Vec<String> {
        // Treat </li> and <br> as line separators before stripping tags.
        let normalised = body
            .replace("</li>", "\n")
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("<br />", "\n");
        let plain = Self::strip_html_tags(&normalised);

        plain
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                // Strip legacy checkbox prefixes if present.
                let item = line
                    .strip_prefix("□ ")
                    .or_else(|| line.strip_prefix("☑ "))
                    .unwrap_or(line)
                    .trim();
                let item = Self::decode_html_entities(item);
                if item.is_empty() {
                    None
                } else {
                    Some(item)
                }
            })
            .collect()
    }

    /// Build an updated item list by adding new items.
    /// Returns `(all_items, added_items, already_present_items)`.
    pub fn apply_add(body: &str, new_items: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
        let current = Self::items_from_body(body);
        let current_norm: Vec<String> = current.iter().map(|i| Self::normalize_item(i)).collect();

        let mut added = Vec::new();
        let mut already_present = Vec::new();

        for item in new_items {
            if current_norm.contains(&Self::normalize_item(item)) {
                already_present.push(item.clone());
            } else {
                added.push(item.clone());
            }
        }

        let all_items: Vec<String> = current.into_iter().chain(added.iter().cloned()).collect();

        (all_items, added, already_present)
    }

    /// Build an updated item list by removing items.
    /// Returns `(remaining_items, removed_items, not_found_items)`.
    pub fn apply_remove(
        body: &str,
        items_to_remove: &[String],
    ) -> (Vec<String>, Vec<String>, Vec<String>) {
        let current = Self::items_from_body(body);
        let mut removed = Vec::new();
        let mut not_found = Vec::new();

        for item in items_to_remove {
            let norm = Self::normalize_item(item);
            if current.iter().any(|i| Self::normalize_item(i) == norm) {
                removed.push(item.clone());
            } else {
                not_found.push(item.clone());
            }
        }

        let removed_norm: Vec<String> = removed.iter().map(|i| Self::normalize_item(i)).collect();
        let remaining: Vec<String> = current
            .into_iter()
            .filter(|i| !removed_norm.contains(&Self::normalize_item(i)))
            .collect();

        (remaining, removed, not_found)
    }

    /// Escape a string value for use inside an AppleScript double-quoted string.
    fn escape_applescript_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Escape text for inclusion in HTML content (not attributes).
    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// Convert an item list to a single-line HTML body that Apple Notes renders
    /// as its native interactive checklist.
    pub fn items_to_html_body(items: &[String]) -> String {
        if items.is_empty() {
            return String::new();
        }
        let lis: Vec<String> = items
            .iter()
            .map(|item| {
                let escaped = Self::html_escape(item);
                format!(
                    r#"<li style="list-style-type:none"><input type="checkbox" disabled="">{escaped}</li>"#
                )
            })
            .collect();
        format!(r#"<ul class="apple-converted-space">{}</ul>"#, lis.join(""))
    }

    /// Build an AppleScript to read the body of a note by title.
    fn build_read_script(title: &str) -> String {
        let escaped = Self::escape_applescript_string(title);
        format!(
            "tell application \"Notes\"\n\
             set noteTitle to \"{escaped}\"\n\
             set theNote to missing value\n\
             repeat with a in accounts\n\
               repeat with n in (notes of a)\n\
                 if name of n is noteTitle then\n\
                   set theNote to n\n\
                   exit repeat\n\
                 end if\n\
               end repeat\n\
               if theNote is not missing value then exit repeat\n\
             end repeat\n\
             if theNote is missing value then\n\
               return \"{NOT_FOUND_SENTINEL}\"\n\
             end if\n\
             return body of theNote\n\
             end tell"
        )
    }

    /// Build an AppleScript to create or update a note.
    /// The note `name` (title) is always kept separate from the body checklist.
    fn build_write_script(title: &str, body: &str) -> String {
        let escaped_title = Self::escape_applescript_string(title);
        let escaped_body = Self::escape_applescript_string(body);
        format!(
            "tell application \"Notes\"\n\
             set noteTitle to \"{escaped_title}\"\n\
             set theNote to missing value\n\
             repeat with a in accounts\n\
               repeat with n in (notes of a)\n\
                 if name of n is noteTitle then\n\
                   set theNote to n\n\
                   exit repeat\n\
                 end if\n\
               end repeat\n\
               if theNote is not missing value then exit repeat\n\
             end repeat\n\
             if theNote is missing value then\n\
               set theNote to make new note with properties \
               {{name:\"{escaped_title}\", body:\"{escaped_body}\"}}\n\
             else\n\
               set body of theNote to \"{escaped_body}\"\n\
             end if\n\
             end tell"
        )
    }

    fn run_script(&self, script: &str) -> Result<String, ShoppingListSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(ShoppingListSkillError::Unavailable);
        }
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| ShoppingListSkillError::Execution(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShoppingListSkillError::Execution(if stderr.is_empty() {
                "osascript failed".to_string()
            } else {
                stderr
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn execute_inner(
        &self,
        action: &str,
        items: &str,
        when: Option<&str>,
    ) -> Result<ShoppingListResult, ShoppingListSkillError> {
        let action_lower = action.trim().to_lowercase();
        if action_lower != "add" && action_lower != "remove" {
            return Err(ShoppingListSkillError::InvalidAction(action.to_string()));
        }

        let date = Self::resolve_date(when);
        let title = Self::note_title(date);
        let parsed_items = Self::parse_items(items);

        if parsed_items.is_empty() {
            return Err(ShoppingListSkillError::Execution(
                "no items specified".to_string(),
            ));
        }

        // Step 1: Read current note body (plain text returned by Notes).
        let current_body = if self.dry_run {
            String::new()
        } else {
            let read_script = Self::build_read_script(&title);
            let raw = self.run_script(&read_script)?;
            if raw == NOT_FOUND_SENTINEL {
                String::new()
            } else {
                raw
            }
        };

        // Step 2: Compute the new item list.
        let (all_items, added, already_present, removed, not_found) = if action_lower == "add" {
            let (all, added, present) = Self::apply_add(&current_body, &parsed_items);
            (all, added, present, vec![], vec![])
        } else {
            let (remaining, removed, not_found) = Self::apply_remove(&current_body, &parsed_items);
            (remaining, vec![], vec![], removed, not_found)
        };

        // Step 3: Write back as native Notes HTML checklist.
        if !self.dry_run {
            let new_body = Self::items_to_html_body(&all_items);
            let write_script = Self::build_write_script(&title, &new_body);
            self.run_script(&write_script)?;
        }

        let summary = if action_lower == "add" {
            if added.is_empty() {
                format!("All items already on '{title}'")
            } else {
                format!("Updated '{title}'")
            }
        } else if removed.is_empty() {
            format!("No items removed from '{title}'")
        } else {
            format!("Updated '{title}'")
        };

        Ok(ShoppingListResult {
            summary,
            note_title: title,
            added,
            already_present,
            removed,
            not_found,
        })
    }
}

impl Default for MacOsNotesShoppingListSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ShoppingListSkill for MacOsNotesShoppingListSkill {
    async fn execute(
        &self,
        action: &str,
        items: &str,
        when: Option<&str>,
    ) -> Result<ShoppingListResult, ShoppingListSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(action, items, when).await;
        match &result {
            Ok(_) => {
                counter!(SHOPPING_LIST_SKILL_EXECUTE_TOTAL, 1, "result" => "success", "action" => action.to_string());
            }
            Err(e) => {
                counter!(SHOPPING_LIST_SKILL_EXECUTE_TOTAL, 1, "result" => "error", "action" => action.to_string());
                counter!(
                    SHOPPING_LIST_SKILL_ERRORS_TOTAL,
                    1,
                    "kind" => e.to_string()
                );
            }
        }
        histogram!(
            SHOPPING_LIST_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64(),
            "action" => action.to_string()
        );
        result
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

    use super::MacOsNotesShoppingListSkill;
    use chrono::NaiveDate;

    #[test]
    fn note_title_formats_correctly() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 19).must();
        assert_eq!(
            MacOsNotesShoppingListSkill::note_title(date),
            "Shopping List 19 Mar 2026"
        );
    }

    #[test]
    fn note_title_formats_single_digit_day() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 5).must();
        assert_eq!(
            MacOsNotesShoppingListSkill::note_title(date),
            "Shopping List 5 Mar 2026"
        );
    }

    #[test]
    fn parse_items_splits_comma_and_and() {
        let items = MacOsNotesShoppingListSkill::parse_items("strawberries, salami and celery");
        assert_eq!(items, vec!["strawberries", "salami", "celery"]);
    }

    #[test]
    fn parse_items_handles_oxford_comma() {
        let items = MacOsNotesShoppingListSkill::parse_items("apples, bananas, and oranges");
        assert_eq!(items, vec!["apples", "bananas", "oranges"]);
    }

    #[test]
    fn parse_items_handles_single_item() {
        let items = MacOsNotesShoppingListSkill::parse_items("milk");
        assert_eq!(items, vec!["milk"]);
    }

    // --- items_from_body --------------------------------------------------

    #[test]
    fn items_from_body_parses_plain_text_lines() {
        // This is the format Notes returns when reading via AppleScript.
        let body = "milk\nbread\ncheese";
        let items = MacOsNotesShoppingListSkill::items_from_body(body);
        assert_eq!(items, vec!["milk", "bread", "cheese"]);
    }

    #[test]
    fn items_from_body_strips_legacy_checkbox_prefixes() {
        let body = "□ milk\n☑ bread";
        let items = MacOsNotesShoppingListSkill::items_from_body(body);
        assert_eq!(items, vec!["milk", "bread"]);
    }

    #[test]
    fn items_from_body_parses_html_checklist() {
        let body = MacOsNotesShoppingListSkill::items_to_html_body(&[
            "milk".to_string(),
            "bread".to_string(),
        ]);
        let items = MacOsNotesShoppingListSkill::items_from_body(&body);
        assert_eq!(items, vec!["milk", "bread"]);
    }

    #[test]
    fn items_from_body_ignores_empty_lines() {
        let body = "milk\n\n\nbread\n";
        let items = MacOsNotesShoppingListSkill::items_from_body(body);
        assert_eq!(items, vec!["milk", "bread"]);
    }

    // --- items_to_html_body -----------------------------------------------

    #[test]
    fn items_to_html_body_creates_checklist_html() {
        let items = vec!["milk".to_string(), "bread".to_string()];
        let html = MacOsNotesShoppingListSkill::items_to_html_body(&items);
        assert!(html.contains(r#"type="checkbox""#));
        assert!(html.contains("milk"));
        assert!(html.contains("bread"));
        assert!(html.contains("ul"));
        assert!(html.contains("li"));
    }

    #[test]
    fn items_to_html_body_returns_empty_for_no_items() {
        assert_eq!(
            MacOsNotesShoppingListSkill::items_to_html_body(&[]),
            String::new()
        );
    }

    #[test]
    fn items_to_html_body_escapes_special_characters() {
        let items = vec!["bread & butter".to_string()];
        let html = MacOsNotesShoppingListSkill::items_to_html_body(&items);
        assert!(html.contains("bread &amp; butter"));
    }

    // --- apply_add --------------------------------------------------------

    #[test]
    fn apply_add_adds_new_items_only() {
        // Body as plain text (what Notes returns on read).
        let body = "strawberries\nsalami";
        let new_items = vec!["celery".to_string(), "strawberries".to_string()];
        let (all_items, added, already_present) =
            MacOsNotesShoppingListSkill::apply_add(body, &new_items);
        assert_eq!(added, vec!["celery"]);
        assert_eq!(already_present, vec!["strawberries"]);
        assert!(all_items.contains(&"celery".to_string()));
        assert!(all_items.contains(&"strawberries".to_string()));
        assert!(all_items.contains(&"salami".to_string()));
    }

    #[test]
    fn apply_add_is_case_insensitive() {
        // Notes may return items with different capitalisation.
        let body = "Strawberries";
        let new_items = vec!["strawberries".to_string()];
        let (_, added, already_present) = MacOsNotesShoppingListSkill::apply_add(body, &new_items);
        assert!(added.is_empty(), "should not re-add strawberries");
        assert_eq!(already_present, vec!["strawberries"]);
    }

    #[test]
    fn apply_add_with_legacy_checkbox_body_detects_existing() {
        let body = "□ Strawberries";
        let new_items = vec!["strawberries".to_string()];
        let (_, added, already_present) = MacOsNotesShoppingListSkill::apply_add(body, &new_items);
        assert!(added.is_empty());
        assert_eq!(already_present, vec!["strawberries"]);
    }

    // --- apply_remove -----------------------------------------------------

    #[test]
    fn apply_remove_removes_matching_items() {
        let body = "strawberries\nsalami\ncelery";
        let to_remove = vec!["salami".to_string()];
        let (remaining, removed, not_found) =
            MacOsNotesShoppingListSkill::apply_remove(body, &to_remove);
        assert_eq!(removed, vec!["salami"]);
        assert!(not_found.is_empty());
        assert!(!remaining.contains(&"salami".to_string()));
        assert!(remaining.contains(&"strawberries".to_string()));
        assert!(remaining.contains(&"celery".to_string()));
    }

    #[test]
    fn apply_remove_reports_not_found() {
        let body = "strawberries";
        let to_remove = vec!["milk".to_string()];
        let (remaining, removed, not_found) =
            MacOsNotesShoppingListSkill::apply_remove(body, &to_remove);
        assert!(removed.is_empty());
        assert_eq!(not_found, vec!["milk"]);
        assert_eq!(remaining, vec!["strawberries"]);
    }

    #[test]
    fn apply_remove_is_case_insensitive() {
        let body = "Salami\nStrawberries";
        let to_remove = vec!["salami".to_string()];
        let (remaining, removed, not_found) =
            MacOsNotesShoppingListSkill::apply_remove(body, &to_remove);
        assert_eq!(removed, vec!["salami"]);
        assert!(not_found.is_empty());
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn apply_remove_on_html_body_works() {
        let body = MacOsNotesShoppingListSkill::items_to_html_body(&[
            "strawberries".to_string(),
            "salami".to_string(),
        ]);
        let to_remove = vec!["salami".to_string()];
        let (remaining, removed, not_found) =
            MacOsNotesShoppingListSkill::apply_remove(&body, &to_remove);
        assert_eq!(removed, vec!["salami"]);
        assert!(not_found.is_empty());
        assert_eq!(remaining, vec!["strawberries"]);
    }

    // --- resolve_date -----------------------------------------------------

    #[test]
    fn resolve_date_defaults_to_today() {
        let today = chrono::Local::now().date_naive();
        assert_eq!(MacOsNotesShoppingListSkill::resolve_date(None), today);
        assert_eq!(
            MacOsNotesShoppingListSkill::resolve_date(Some("today")),
            today
        );
    }

    #[test]
    fn resolve_date_tomorrow() {
        let tomorrow = chrono::Local::now().date_naive() + chrono::Duration::days(1);
        assert_eq!(
            MacOsNotesShoppingListSkill::resolve_date(Some("tomorrow")),
            tomorrow
        );
    }

    #[test]
    fn resolve_date_iso_string() {
        let d = MacOsNotesShoppingListSkill::resolve_date(Some("2026-03-25"));
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 3, 25).must());
    }
}
