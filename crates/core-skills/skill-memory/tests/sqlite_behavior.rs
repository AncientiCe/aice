use skill_memory::{MemorySkill, SqliteMemorySkill};

#[tokio::test]
async fn stores_and_recalls_fact_from_query() {
    let skill = SqliteMemorySkill::new_in_memory().expect("in-memory db");

    let store = skill
        .execute(Some("remember that my favorite color is blue"), Some(true))
        .await
        .expect("store ok");
    assert!(store.stored);

    let recall = skill
        .execute(Some("what is my favorite color"), Some(false))
        .await
        .expect("recall ok");
    assert!(!recall.facts.is_empty());
    assert!(recall
        .facts
        .iter()
        .any(|f| f.value.to_lowercase().contains("blue")));
}

#[tokio::test]
async fn extracts_memory_from_regular_turns_when_enabled() {
    let skill = SqliteMemorySkill::new_in_memory().expect("in-memory db");
    skill
        .ingest_turn("I prefer warm white lights in the evening")
        .await
        .expect("ingest turn");

    let recall = skill
        .execute(Some("what lights do i prefer"), Some(false))
        .await
        .expect("recall ok");
    assert!(!recall.facts.is_empty());
}
