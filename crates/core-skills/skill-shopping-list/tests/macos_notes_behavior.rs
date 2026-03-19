//! Behavioral tests for macOS Notes shopping list skill (dry-run mode).

use skill_shopping_list::{MacOsNotesShoppingListSkill, ShoppingListSkill};

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
async fn dry_run_add_items_to_empty_list() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill
        .execute("add", "strawberries, salami and celery", Some("today"))
        .await
        .must();
    // dry_run starts with empty body so all items are added
    assert_eq!(result.added.len(), 3);
    assert!(result.already_present.is_empty());
    assert!(result.removed.is_empty());
    assert!(result.not_found.is_empty());
    assert!(result.added.contains(&"strawberries".to_string()));
    assert!(result.added.contains(&"salami".to_string()));
    assert!(result.added.contains(&"celery".to_string()));
}

#[tokio::test]
async fn dry_run_remove_from_list_reports_not_found_on_empty() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill.execute("remove", "milk", None).await.must();
    // dry_run starts with empty body
    assert!(result.removed.is_empty());
    assert_eq!(result.not_found, vec!["milk"]);
}

#[tokio::test]
async fn note_title_uses_today_when_no_when_given() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let today = chrono::Local::now().date_naive();
    let expected_title = MacOsNotesShoppingListSkill::note_title(today);
    let result = skill.execute("add", "bread", None).await.must();
    assert_eq!(result.note_title, expected_title);
}

#[tokio::test]
async fn note_title_uses_specified_date() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill
        .execute("add", "milk", Some("2026-03-19"))
        .await
        .must();
    assert_eq!(result.note_title, "Shopping List 19 Mar 2026");
}

#[tokio::test]
async fn dry_run_rejects_invalid_action() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill.execute("update", "milk", None).await;
    assert!(result.is_err());
    let err = result.must_err();
    assert!(err.to_string().contains("invalid action"));
}

#[tokio::test]
async fn dry_run_rejects_empty_items() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill.execute("add", "", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn to_prompt_context_describes_added_items() {
    let skill = MacOsNotesShoppingListSkill::new_for_tests();
    let result = skill
        .execute("add", "apples and oranges", Some("2026-03-19"))
        .await
        .must();
    let context = result.to_prompt_context();
    assert!(context.contains("Shopping List 19 Mar 2026"));
    assert!(context.contains("Added:"));
    assert!(context.contains("apples"));
    assert!(context.contains("oranges"));
}

#[tokio::test]
async fn parse_items_trims_whitespace() {
    let items = MacOsNotesShoppingListSkill::parse_items("  milk ,  bread  ");
    assert_eq!(items, vec!["milk", "bread"]);
}
