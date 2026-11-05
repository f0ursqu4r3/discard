use discard_core::memory::{MemoryStore, Scope};
use discard_core::memory_items::{ItemKind, MutationOrigin, SaveOutcome};
use discard_core::model::{AgentId, ChannelId, ProjectId};

#[test]
fn domains_versions_forget_and_undo_preserve_boundaries() {
    let d = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let a = AgentId("a".into());
    let b = AgentId("b".into());
    let p = ProjectId("p".into());
    let q = ProjectId("q".into());
    let source = ChannelId("chat".into());
    let scope = Scope::AgentProject(a.clone(), p.clone());
    assert_eq!(
        Scope::applicable(&a, Some(&p)),
        vec![
            scope.clone(),
            Scope::Project(p.clone()),
            Scope::Agent(a.clone()),
            Scope::Global
        ]
    );
    assert_eq!(
        Scope::applicable(&a, None),
        vec![Scope::Agent(a.clone()), Scope::Global]
    );
    let item = store
        .create_item(
            &scope,
            ItemKind::Preference,
            "prefer apples",
            Some(&source),
            MutationOrigin::Automatic,
        )
        .unwrap();
    assert!(
        store
            .search(&Scope::applicable(&b, Some(&p)), "apples", 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search(&Scope::applicable(&a, Some(&q)), "apples", 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .search(&Scope::applicable(&a, Some(&p)), "apples", 10)
            .unwrap()
            .len(),
        1
    );
    let updated = store
        .update_item(
            &item.id,
            item.version,
            "prefer oranges",
            MutationOrigin::User,
        )
        .unwrap();
    assert!(
        store
            .update_item(&item.id, item.version, "stale", MutationOrigin::User)
            .is_err()
    );
    assert!(
        store
            .update_item(
                &item.id,
                updated.version,
                "inferred",
                MutationOrigin::Automatic
            )
            .is_err()
    );
    assert!(
        store
            .search(&[scope.clone()], "apples", 10)
            .unwrap()
            .is_empty()
    );
    let moved = store
        .move_item(&item.id, updated.version, &Scope::Agent(a.clone()))
        .unwrap();
    assert!(store.search(&[scope], "oranges", 10).unwrap().is_empty());
    assert!(store.undo_item(&item.id, updated.version).is_err());
    let forgotten = store.forget_item(&item.id, moved.version).unwrap();
    store.rebuild_index().unwrap();
    assert!(
        store
            .search(&Scope::applicable(&a, Some(&p)), "oranges", 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .create_item(
                &Scope::Agent(a.clone()),
                ItemKind::Preference,
                "prefer oranges",
                Some(&source),
                MutationOrigin::Automatic
            )
            .is_err()
    );
    assert!(
        store
            .create_item(
                &Scope::Agent(a.clone()),
                ItemKind::Preference,
                "prefer oranges",
                Some(&source),
                MutationOrigin::User
            )
            .is_ok()
    );
    assert!(store.read_item(&item.id).unwrap().history.len() >= 3);
    let restored = store.undo_item(&item.id, forgotten.version).unwrap();
    assert!(!restored.deleted);
    assert_eq!(restored.content, "prefer oranges");
}

#[test]
fn shared_capture_is_a_proposal_and_only_app_can_approve() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let a = AgentId("a".into());
    let c = ChannelId("chat".into());
    let outcome = s
        .capture_item(
            &a,
            None,
            &c,
            &Scope::Global,
            ItemKind::Fact,
            "shared needle",
        )
        .unwrap();
    let SaveOutcome::Proposed(proposal) = outcome else {
        panic!("shared save bypassed approval")
    };
    assert!(s.search(&[Scope::Global], "needle", 10).unwrap().is_empty());
    assert_eq!(s.pending_proposals().unwrap().len(), 1);
    let item = s.approve_proposal(&proposal.id).unwrap();
    assert_eq!(item.source_conversation, Some(c));
    assert_eq!(s.search(&[Scope::Global], "needle", 10).unwrap().len(), 1);
    assert!(s.approve_proposal(&proposal.id).is_err());
    assert!(
        s.capture_item(
            &a,
            None,
            &ChannelId("chat".into()),
            &Scope::Agent(AgentId("other".into())),
            ItemKind::Fact,
            "private"
        )
        .is_err()
    );
}

#[test]
fn both_domain_ids_are_validated_and_cleanup_finds_orphan_domains() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    for (a, p) in [("../x", "p"), ("a", "../x")] {
        assert!(
            s.list(&Scope::AgentProject(AgentId(a.into()), ProjectId(p.into())))
                .is_err()
        );
    }
    let a = AgentId("a".into());
    s.create(
        &Scope::AgentProject(a.clone(), ProjectId("orphan".into())),
        "x.md",
        "x",
    )
    .unwrap();
    assert!(
        s.private_scopes(&a)
            .unwrap()
            .contains(&Scope::AgentProject(a, ProjectId("orphan".into())))
    );
}

#[test]
fn reopen_and_index_recovery_never_retrieve_history_or_deleted_items() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("memory");
    let index = d.path().join("index.db");
    let a = AgentId("a".into());
    let old = Scope::Agent(a.clone());
    let new = Scope::AgentProject(a.clone(), ProjectId("p".into()));
    let id;
    {
        let s = MemoryStore::open(&root, &index).unwrap();
        let i = s
            .create_item(
                &old,
                ItemKind::Fact,
                "historicalsecret",
                None,
                MutationOrigin::User,
            )
            .unwrap();
        id = i.id.clone();
        let i = s
            .update_item(&id, i.version, "livevalue", MutationOrigin::User)
            .unwrap();
        let i = s.move_item(&id, i.version, &new).unwrap();
        assert!(s.replace(&new, &i.path(), "bypass", "1").is_err());
        assert!(s.append(&new, &i.path(), "bypass").is_err());
        assert!(s.delete(&new, &i.path()).is_err());
    }
    std::fs::remove_file(&index).unwrap();
    let s = MemoryStore::open(&root, &index).unwrap();
    s.reconcile().unwrap();
    for rebuild in [false, true] {
        if rebuild {
            s.rebuild_index().unwrap();
        }
        assert!(
            s.search(&[old.clone(), new.clone()], "historicalsecret", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            s.search(&[old.clone()], "livevalue", 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(s.search(&[new.clone()], "livevalue", 10).unwrap().len(), 1);
    }
    let i = s.read_item(&id).unwrap();
    assert_eq!(i.history[0].content, "historicalsecret");
    s.forget_item(&id, i.version).unwrap();
    drop(s);
    let s = MemoryStore::open(&root, &index).unwrap();
    s.rebuild_index().unwrap();
    assert!(s.search(&[old, new], "livevalue", 10).unwrap().is_empty());
    assert!(
        s.core_blocks(&Scope::applicable(&a, None))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn complete_domain_matrix_and_private_cleanup_keep_shared_memory() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let agents = [AgentId("a".into()), AgentId("b".into())];
    let projects = [ProjectId("p".into()), ProjectId("q".into())];
    let mut scopes = vec![Scope::Global];
    scopes.extend(agents.iter().cloned().map(Scope::Agent));
    scopes.extend(projects.iter().cloned().map(Scope::Project));
    for a in &agents {
        for p in &projects {
            scopes.push(Scope::AgentProject(a.clone(), p.clone()));
        }
    }
    for scope in &scopes {
        s.create_item(
            scope,
            ItemKind::Preference,
            &format!("matrix {}", scope.label()),
            None,
            MutationOrigin::User,
        )
        .unwrap();
    }
    for a in &agents {
        for p in [None, Some(&projects[0]), Some(&projects[1])] {
            let applicable = Scope::applicable(a, p);
            let items = s.list_items(&applicable).unwrap();
            assert_eq!(
                items.iter().map(|i| i.scope.clone()).collect::<Vec<_>>(),
                applicable
            );
            assert_eq!(
                s.search(&applicable, "matrix", 20).unwrap().len(),
                applicable.len()
            );
            assert_eq!(s.core_blocks(&applicable).unwrap().len(), applicable.len());
        }
    }
    assert_eq!(s.scopes_on_disk().len(), scopes.len());
    s.delete_private_memory(&agents[0]).unwrap();
    assert!(
        s.list_items(&[Scope::Agent(agents[0].clone())])
            .unwrap()
            .is_empty()
    );
    assert!(
        s.private_scopes(&agents[0])
            .unwrap()
            .iter()
            .all(|scope| s.list_items(&[scope.clone()]).unwrap().is_empty())
    );
    assert_eq!(
        s.list_items(&[
            Scope::Global,
            Scope::Project(projects[0].clone()),
            Scope::Agent(agents[1].clone())
        ])
        .unwrap()
        .len(),
        3
    );
}

#[test]
fn item_documents_and_nested_scope_symlinks_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join(".items")).unwrap();
    assert!(
        s.create_item(
            &Scope::Global,
            ItemKind::Fact,
            "escape",
            None,
            MutationOrigin::User
        )
        .is_err()
    );
    std::fs::remove_file(d.path().join(".items")).unwrap();
    std::fs::create_dir_all(d.path().join("agent-projects")).unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join("agent-projects/a")).unwrap();
    let scope = Scope::AgentProject(AgentId("a".into()), ProjectId("p".into()));
    assert!(s.list(&scope).is_err());
    assert!(
        s.create_item(&scope, ItemKind::Fact, "escape", None, MutationOrigin::User)
            .is_err()
    );
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
}

#[test]
fn proposals_persist_stale_updates_fail_and_changes_have_stable_keys() {
    let d = tempfile::tempdir().unwrap();
    let a = AgentId("a".into());
    let c = ChannelId("source".into());
    let scope = Scope::Agent(a.clone());
    let item;
    let proposal;
    {
        let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
        item = s
            .create_item(
                &scope,
                ItemKind::Decision,
                "decision one",
                Some(&c),
                MutationOrigin::User,
            )
            .unwrap();
        proposal = s
            .propose_update(&a, None, &c, &item.id, item.version, "decision two")
            .unwrap();
    }
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    assert_eq!(s.pending_proposals().unwrap(), vec![proposal.clone()]);
    s.update_item(
        &item.id,
        item.version,
        "user correction",
        MutationOrigin::User,
    )
    .unwrap();
    assert!(s.approve_proposal(&proposal.id).is_err());
    assert_eq!(s.read_item(&item.id).unwrap().content, "user correction");
    let changes = s.memory_changes(&c).unwrap();
    assert_eq!(changes, s.memory_changes(&c).unwrap());
    s.reject_proposal(&proposal.id).unwrap();
    assert!(s.pending_proposals().unwrap().is_empty());
}

#[test]
fn forget_proposals_need_approval_reject_cleanly_and_refuse_stale_delete() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let a = AgentId("a".into());
    let c = ChannelId("original".into());
    let current = ChannelId("current".into());
    let scope = Scope::Agent(a.clone());
    let i = s
        .create_item(
            &scope,
            ItemKind::Fact,
            "forget value",
            Some(&c),
            MutationOrigin::User,
        )
        .unwrap();
    let p = s
        .propose_forget(&a, None, &current, &i.id, i.version)
        .unwrap();
    s.reject_proposal(&p.id).unwrap();
    assert!(!s.read_item(&i.id).unwrap().deleted);
    assert!(s.approve_proposal(&p.id).is_err());
    let p = s
        .propose_forget(&a, None, &current, &i.id, i.version)
        .unwrap();
    let i = s
        .update_item(&i.id, i.version, "new value", MutationOrigin::User)
        .unwrap();
    assert!(s.approve_proposal(&p.id).is_err());
    assert!(!s.read_item(&i.id).unwrap().deleted);
    let p = s
        .propose_forget(&a, None, &current, &i.id, i.version)
        .unwrap();
    s.approve_proposal(&p.id).unwrap();
    assert!(s.read_item(&i.id).unwrap().deleted);
    assert!(
        s.capture_item(&a, None, &current, &scope, ItemKind::Fact, " new value ")
            .is_err()
    );
    assert!(
        s.capture_item(&a, None, &c, &scope, ItemKind::Fact, "new value")
            .is_err()
    );
}

#[test]
fn stale_index_after_authoritative_write_cannot_deliver_forgotten_or_old_content() {
    let d = tempfile::tempdir().unwrap();
    let s = MemoryStore::open_in_memory_index(d.path()).unwrap();
    let i = s
        .create_item(
            &Scope::Global,
            ItemKind::Fact,
            "oldindexedword",
            None,
            MutationOrigin::User,
        )
        .unwrap();
    let path = d.path().join(".items").join(format!("{}.md", i.id));
    let original = std::fs::read_to_string(&path).unwrap();
    // Simulate a committed markdown rename followed by a crash before FTS update.
    std::fs::write(&path, original.replace("oldindexedword", "newbodyword")).unwrap();
    assert!(
        s.search(&[Scope::Global], "oldindexedword", 10)
            .unwrap()
            .is_empty()
    );
    s.reconcile().unwrap();
    assert_eq!(
        s.search(&[Scope::Global], "newbodyword", 10).unwrap().len(),
        1
    );
    let document = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        document.replace("\"deleted\":false", "\"deleted\":true"),
    )
    .unwrap();
    assert!(
        s.search(&[Scope::Global], "newbodyword", 10)
            .unwrap()
            .is_empty()
    );
}
