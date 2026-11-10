use discard_core::{model::{Project, ProjectId}, store::Store};

#[test]
fn project_profile_survives_reopening_without_changing_identity_or_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.db");
    let project = Project { id: ProjectId::new(), name: "original".into(), path: "/workspace".into() };
    {
        let store = Store::open(&path).unwrap();
        store.insert_project(&project).unwrap();
        assert_eq!(store.project_avatar(&project.id).unwrap(), None);
        store.update_project_profile(&project.id, "  Mission Control  ", Some("lucide:rocket:#123456")).unwrap();
        assert!(store.update_project_profile(&project.id, "  ", None).is_err());
        assert!(store.update_project_profile(&project.id, "bad", Some("lucide:rocket:red")).is_err());
        assert!(store.update_project_profile(&ProjectId::new(), "missing", None).is_err());
    }
    let store = Store::open(&path).unwrap();
    let saved = &store.list_projects().unwrap()[0];
    assert_eq!(saved.id, project.id);
    assert_eq!(saved.path, project.path);
    assert_eq!(saved.name, "Mission Control");
    assert_eq!(store.project_avatar(&project.id).unwrap().as_deref(), Some("lucide:rocket:#123456"));
    store.update_project_profile(&project.id, "Mission Control", None).unwrap();
    assert_eq!(store.project_avatar(&project.id).unwrap(), None);
}

#[test]
fn version_five_projects_gain_optional_appearance_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migration.db");
    let project = Project { id: ProjectId::new(), name: "Existing".into(), path: "/existing".into() };
    {
        let store = Store::open(&path).unwrap();
        store.insert_project(&project).unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("ALTER TABLE projects DROP COLUMN avatar;
            ALTER TABLE messages DROP COLUMN author_json;
            DROP INDEX one_project_active_branch;
            ALTER TABLE project_channels DROP COLUMN repository;
            CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id,branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;
            DROP TABLE always_allow_rules; PRAGMA user_version=5;").unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 10);
    assert_eq!(store.list_projects().unwrap(), vec![project.clone()]);
    assert_eq!(store.project_avatar(&project.id).unwrap(), None);
}
