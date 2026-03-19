use skill_memory::{MemorySkill, SqliteMemorySkill};

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
async fn stores_and_recalls_fact_from_query() {
    let skill = SqliteMemorySkill::new_in_memory().must();

    let store = skill
        .execute(Some("remember that my favorite color is blue"), Some(true))
        .await
        .must();
    assert!(store.stored);

    let recall = skill
        .execute(Some("what is my favorite color"), Some(false))
        .await
        .must();
    assert!(!recall.facts.is_empty());
    assert!(recall
        .facts
        .iter()
        .any(|f| f.value.to_lowercase().contains("blue")));
}

#[tokio::test]
async fn extracts_memory_from_regular_turns_when_enabled() {
    let skill = SqliteMemorySkill::new_in_memory().must();
    skill
        .ingest_turn("I prefer warm white lights in the evening")
        .await
        .must();

    let recall = skill
        .execute(Some("what lights do i prefer"), Some(false))
        .await
        .must();
    assert!(!recall.facts.is_empty());
}
