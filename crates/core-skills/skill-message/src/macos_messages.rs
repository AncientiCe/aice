//! macOS Messages.app integration via AppleScript + native Contacts (CNContactStore) helper.

use crate::types::{MessageResult, MessageSkill, MessageSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MESSAGE_SKILL_EXECUTE_TOTAL: &str = "message_skill_execute_total";
const MESSAGE_SKILL_ERRORS_TOTAL: &str = "message_skill_errors_total";
const MESSAGE_SKILL_EXECUTE_DURATION_SECONDS: &str = "message_skill_execute_duration_seconds";

const CONTACTS_SWIFT_HELPER: &str = r#"import Foundation
import Contacts

func lower(_ s: String) -> String { s.lowercased() }

func normalize(_ s: String) -> String {
    var out = s.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    if out.hasPrefix("my ") { out = String(out.dropFirst(3)).trimmingCharacters(in: .whitespaces) }
    if out.hasPrefix("the ") { out = String(out.dropFirst(4)).trimmingCharacters(in: .whitespaces) }
    return out
}

let query = CommandLine.arguments.dropFirst().first ?? ""
if query.isEmpty {
    print("")
    exit(0)
}

let store = CNContactStore()
let status = CNContactStore.authorizationStatus(for: .contacts)
if status == .notDetermined {
    let sem = DispatchSemaphore(value: 0)
    store.requestAccess(for: .contacts) { _, _ in sem.signal() }
    sem.wait()
}
let postStatus = CNContactStore.authorizationStatus(for: .contacts)
if postStatus != .authorized {
    fputs("contacts not authorized\n", stderr)
    exit(2)
}

let keys: [CNKeyDescriptor] = [
    CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
    CNContactGivenNameKey as CNKeyDescriptor,
    CNContactFamilyNameKey as CNKeyDescriptor,
    CNContactNicknameKey as CNKeyDescriptor,
    CNContactPhoneNumbersKey as CNKeyDescriptor,
    CNContactEmailAddressesKey as CNKeyDescriptor,
    CNContactRelationsKey as CNKeyDescriptor
]

let q = normalize(query)

func bestHandle(_ c: CNContact) -> String? {
    if let phone = c.phoneNumbers.first?.value.stringValue, !phone.isEmpty { return phone }
    if let email = c.emailAddresses.first?.value as String?, !email.isEmpty { return email }
    return nil
}

func displayName(_ c: CNContact) -> String {
    if let full = CNContactFormatter.string(from: c, style: .fullName), !full.isEmpty {
        return full
    }
    let combined = (c.givenName + " " + c.familyName).trimmingCharacters(in: .whitespaces)
    return combined.isEmpty ? "Unknown" : combined
}

func match(_ c: CNContact, _ q: String) -> Bool {
    let full = lower(displayName(c))
    let nick = lower(c.nickname)
    if full.contains(q) || (!nick.isEmpty && nick.contains(q)) { return true }
    for rel in c.contactRelations {
        let label = lower(CNLabeledValue<NSString>.localizedString(forLabel: rel.label ?? ""))
        if label.contains(q) { return true }
    }
    return false
}

do {
    // Fast path: system name matching first.
    let predicate = CNContact.predicateForContacts(matchingName: query)
    let fast = try store.unifiedContacts(matching: predicate, keysToFetch: keys)
    for c in fast {
        if let handle = bestHandle(c), match(c, q) || !q.isEmpty {
            print("\(displayName(c))|\(handle)")
            exit(0)
        }
    }

    // Fallback path: enumerate once when predicate missed relation labels.
    let req = CNContactFetchRequest(keysToFetch: keys)
    var found: (String, String)? = nil
    try store.enumerateContacts(with: req) { c, stop in
        if found != nil { stop.pointee = true; return }
        if match(c, q), let handle = bestHandle(c) {
            found = (displayName(c), handle)
            stop.pointee = true
        }
    }
    if let f = found {
        print("\(f.0)|\(f.1)")
    } else {
        print("")
    }
} catch {
    fputs("contacts lookup failed: \(error)\n", stderr)
    exit(3)
}
"#;

#[derive(Clone)]
pub struct MacOsMessagesSkill {
    dry_run: bool,
    resolve_cache: Arc<Mutex<HashMap<String, CachedContact>>>,
}

#[derive(Clone, Debug)]
struct ContactEntry {
    display_name: String,
    handle: String,
}

#[derive(Clone, Debug)]
struct CachedContact {
    loaded_at: Instant,
    entry: ContactEntry,
}

impl MacOsMessagesSkill {
    const RESOLVE_CACHE_TTL_SECS: u64 = 3600;

    pub fn new() -> Self {
        Self {
            dry_run: false,
            resolve_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_for_tests() -> Self {
        Self {
            dry_run: true,
            resolve_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn run_script(script: &str, args: &[&str]) -> Result<String, MessageSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(MessageSkillError::Unavailable);
        }
        let mut command = Command::new("osascript");
        command.arg("-e").arg(script);
        for arg in args {
            command.arg(arg);
        }
        let output = command
            .output()
            .map_err(|e| MessageSkillError::Execution(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(MessageSkillError::Execution(if stderr.is_empty() {
                "osascript failed".to_string()
            } else {
                stderr
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn normalize_contact_key(value: &str) -> String {
        let mut out = value.trim().to_ascii_lowercase();
        for prefix in ["my ", "the "] {
            if let Some(rest) = out.strip_prefix(prefix) {
                out = rest.trim().to_string();
            }
        }
        out
    }

    fn build_send_script() -> String {
        "on run argv\n\
         set targetHandle to item 1 of argv\n\
         set outgoingText to item 2 of argv\n\
         tell application \"Messages\"\n\
             set targetService to first service whose service type is iMessage\n\
             set targetParticipant to participant targetHandle of targetService\n\
             send outgoingText to targetParticipant\n\
         end tell\n\
         end run"
            .to_string()
    }

    #[doc(hidden)]
    pub fn build_send_script_for_tests() -> String {
        Self::build_send_script()
    }

    #[doc(hidden)]
    pub fn normalize_contact_key_for_tests(value: &str) -> String {
        Self::normalize_contact_key(value)
    }

    #[doc(hidden)]
    pub fn parse_resolve_contact_output_for_tests(output: &str) -> Option<(String, String)> {
        Self::parse_resolve_contact_output(output).map(|e| (e.display_name, e.handle))
    }

    fn parse_resolve_contact_output(output: &str) -> Option<ContactEntry> {
        let line = output.trim();
        if line.is_empty() {
            return None;
        }
        let (display, handle) = line.split_once('|')?;
        let display = display.trim();
        let handle = handle.trim();
        if display.is_empty() || handle.is_empty() {
            return None;
        }
        Some(ContactEntry {
            display_name: display.to_string(),
            handle: handle.to_string(),
        })
    }

    fn resolve_contact_native(
        &self,
        contact: &str,
    ) -> Result<Option<ContactEntry>, MessageSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(MessageSkillError::Unavailable);
        }
        let output = Command::new("xcrun")
            .arg("swift")
            .arg("-")
            .arg(contact)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(CONTACTS_SWIFT_HELPER.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| {
                MessageSkillError::Execution(format!("swift contacts lookup failed: {e}"))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(MessageSkillError::Execution(if stderr.is_empty() {
                "swift contacts lookup failed".to_string()
            } else {
                stderr
            }));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(Self::parse_resolve_contact_output(&stdout))
    }

    fn resolve_contact(&self, contact: &str) -> Result<ContactEntry, MessageSkillError> {
        let key = Self::normalize_contact_key(contact);
        {
            let guard = self.resolve_cache.lock().map_err(|_| {
                MessageSkillError::Execution("resolve cache lock poisoned".to_string())
            })?;
            if let Some(cached) = guard.get(&key) {
                if cached.loaded_at.elapsed() < Duration::from_secs(Self::RESOLVE_CACHE_TTL_SECS) {
                    return Ok(cached.entry.clone());
                }
            }
        }

        let Some(found) = self.resolve_contact_native(contact.trim())? else {
            return Err(MessageSkillError::ContactNotFound(
                contact.trim().to_string(),
            ));
        };

        let mut guard = self
            .resolve_cache
            .lock()
            .map_err(|_| MessageSkillError::Execution("resolve cache lock poisoned".to_string()))?;
        guard.insert(
            key,
            CachedContact {
                loaded_at: Instant::now(),
                entry: found.clone(),
            },
        );
        Ok(found)
    }

    fn send_message(&self, target_handle: &str, message: &str) -> Result<(), MessageSkillError> {
        let script = Self::build_send_script();
        Self::run_script(&script, &[target_handle, message])
            .map(|_| ())
            .map_err(|e| match e {
                MessageSkillError::Execution(msg) => MessageSkillError::SendFailed(msg),
                other => other,
            })
    }

    fn list_messages_services_for_debug(&self) -> Result<String, MessageSkillError> {
        let script = "tell application \"Messages\"\n\
            set linesOut to \"\"\n\
            repeat with s in services\n\
                try\n\
                    set linesOut to linesOut & (id of s as text) & \"|\" & (service type of s as text) & linefeed\n\
                end try\n\
            end repeat\n\
            return linesOut\n\
        end tell";
        let out = Self::run_script(script, &[])?;
        Ok(out)
    }

    async fn execute_inner(
        &self,
        contact: &str,
        message: &str,
    ) -> Result<MessageResult, MessageSkillError> {
        let contact = contact.trim();
        let message = message.trim();
        if contact.is_empty() {
            return Err(MessageSkillError::Execution(
                "contact must not be empty".to_string(),
            ));
        }
        if message.is_empty() {
            return Err(MessageSkillError::Execution(
                "message must not be empty".to_string(),
            ));
        }

        if self.dry_run {
            return Ok(MessageResult {
                summary: format!("Sent iMessage to {}", contact),
                recipient_name: contact.to_string(),
                recipient_handle: contact.to_string(),
                message: message.to_string(),
            });
        }

        let matched = self.resolve_contact(contact)?;
        self.send_message(&matched.handle, message)
            .map_err(|e| match e {
                MessageSkillError::SendFailed(msg) => {
                    let services = self
                        .list_messages_services_for_debug()
                        .unwrap_or_else(|_| "services-unavailable".to_string());
                    MessageSkillError::SendFailed(format!(
                        "{} | target_handle={} | services={}",
                        msg, matched.handle, services
                    ))
                }
                other => other,
            })?;
        Ok(MessageResult {
            summary: format!("Sent iMessage to {}", matched.display_name),
            recipient_name: matched.display_name,
            recipient_handle: matched.handle,
            message: message.to_string(),
        })
    }
}

impl Default for MacOsMessagesSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageSkill for MacOsMessagesSkill {
    async fn execute(
        &self,
        contact: &str,
        message: &str,
    ) -> Result<MessageResult, MessageSkillError> {
        let t0 = Instant::now();
        let result = self.execute_inner(contact, message).await;
        match &result {
            Ok(_) => {
                counter!(MESSAGE_SKILL_EXECUTE_TOTAL, 1, "result" => "success");
            }
            Err(e) => {
                counter!(MESSAGE_SKILL_EXECUTE_TOTAL, 1, "result" => "error");
                counter!(
                    MESSAGE_SKILL_ERRORS_TOTAL,
                    1,
                    "error_kind" => e.to_string()
                );
            }
        }
        histogram!(
            MESSAGE_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64()
        );
        result
    }
}
