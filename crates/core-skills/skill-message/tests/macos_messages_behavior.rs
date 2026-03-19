//! Behavioral tests for macOS Messages skill (dry-run mode).

use skill_message::{MacOsMessagesSkill, MessageSkill, MessageSkillError};

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

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
    fn must_err(self) -> E;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }

    fn must_err(self) -> E {
        match self {
            Ok(_) => panic!("expected Err(..) in test, got Ok"),
            Err(error) => error,
        }
    }
}

#[tokio::test]
async fn dry_run_sends_message_and_returns_result() {
    let skill = MacOsMessagesSkill::new_for_tests();
    let result = skill
        .execute("my wife", "How are you feeling?")
        .await
        .must();

    assert_eq!(result.recipient_name, "my wife");
    assert_eq!(result.recipient_handle, "my wife");
    assert_eq!(result.message, "How are you feeling?");
    assert!(result.summary.contains("Sent iMessage"));
}

#[tokio::test]
async fn dry_run_rejects_empty_contact() {
    let skill = MacOsMessagesSkill::new_for_tests();
    let err = skill.execute("", "How are you?").await.must_err();

    assert!(matches!(err, MessageSkillError::Execution(_)));
}

#[tokio::test]
async fn dry_run_rejects_empty_message() {
    let skill = MacOsMessagesSkill::new_for_tests();
    let err = skill.execute("my wife", "").await.must_err();

    assert!(matches!(err, MessageSkillError::Execution(_)));
}

#[test]
fn to_prompt_context_includes_contact_and_message() {
    let result = skill_message::MessageResult {
        summary: "Sent iMessage to Jane Doe".to_string(),
        recipient_name: "Jane Doe".to_string(),
        recipient_handle: "+15551234567".to_string(),
        message: "How are you?".to_string(),
    };

    let prompt = result.to_prompt_context();
    assert!(prompt.contains("Jane Doe"));
    assert!(prompt.contains("How are you?"));
}

#[test]
fn send_script_uses_variables_and_messages_buddy_send_shape() {
    let contacts_script = MacOsMessagesSkill::build_send_script_for_tests();
    assert!(contacts_script.contains("tell application \"Messages\""));
    assert!(contacts_script.contains("first service whose service type is iMessage"));
    assert!(contacts_script.contains("participant targetHandle of targetService"));

    let send_script = MacOsMessagesSkill::build_send_script_for_tests();
    assert!(send_script.contains("on run argv"));
    assert!(send_script.contains("set targetHandle to item 1 of argv"));
    assert!(send_script.contains("set outgoingText to item 2 of argv"));
    assert!(send_script.contains("first service whose service type is iMessage"));
    assert!(
        send_script.contains("set targetParticipant to participant targetHandle of targetService")
    );
    assert!(send_script.contains("send outgoingText to targetParticipant"));
}

#[test]
fn send_script_does_not_inline_user_text_literals() {
    let script = MacOsMessagesSkill::build_send_script_for_tests();
    assert!(
        !script.contains("How are you"),
        "send script should not inline user-provided text"
    );
    assert!(
        !script.contains("+1555"),
        "send script should not inline user-provided handle"
    );
}

#[test]
fn parse_contacts_output_parses_service_and_buddy_lines() {
    let parsed =
        MacOsMessagesSkill::parse_resolve_contact_output_for_tests("Jane Doe|+15551234567");
    assert!(parsed.is_some());
    let parsed = parsed.must();
    assert_eq!(parsed.0, "Jane Doe");
    assert_eq!(parsed.1, "+15551234567");
}

#[test]
fn contact_lookup_matches_my_prefix_and_case_insensitive() {
    assert_eq!(
        MacOsMessagesSkill::normalize_contact_key_for_tests("My Wife"),
        "wife"
    );
    assert_eq!(
        MacOsMessagesSkill::normalize_contact_key_for_tests("the Husband"),
        "husband"
    );
}
