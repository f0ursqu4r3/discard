use discard_core::{
    memory::{MemoryStore, Scope},
    memory_items::ItemKind,
};

#[test]
fn app_creation_reports_committed_index_failures_and_retries_without_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("index.db");
    let memory = MemoryStore::open(&dir.path().join("memory"), &database).unwrap();
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute_batch("DROP TABLE mem_fts").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let (item, warning) = memory
        .create_user_item(&id, &Scope::Global, ItemKind::Fact, "saved fact")
        .unwrap();
    assert_eq!(item.id, id);
    assert!(warning.is_some());
    let (_, retry_warning) = memory
        .create_user_item(&id, &Scope::Global, ItemKind::Fact, "saved fact")
        .unwrap();
    assert!(retry_warning.is_some());
    assert_eq!(memory.list_items(&[Scope::Global]).unwrap().len(), 1);
    assert!(
        memory
            .create_user_item(&id, &Scope::Global, ItemKind::Fact, "different fact")
            .is_err()
    );
    sql.execute_batch(
        "CREATE VIRTUAL TABLE mem_fts USING fts5(scope UNINDEXED, path UNINDEXED, content)",
    )
    .unwrap();
    let (_, warning) = memory
        .create_user_item(&id, &Scope::Global, ItemKind::Fact, "saved fact")
        .unwrap();
    assert!(warning.is_none());
    assert_eq!(
        memory.search(&[Scope::Global], "saved", 10).unwrap().len(),
        1
    );
    memory.forget_item(&id, 1).unwrap();
    assert!(
        memory
            .create_user_item(&id, &Scope::Global, ItemKind::Fact, "saved fact")
            .is_err()
    );
    assert!(memory.list_items(&[Scope::Global]).unwrap().is_empty());
}
