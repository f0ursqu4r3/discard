use discard_app::desktop::settings::project_memory;
use discard_core::{
    agent::AgentConfig,
    capability::ApprovalPolicy,
    memory::{MemoryStore, Scope},
    memory_items::{ItemKind, MutationOrigin, SaveOutcome},
    model::{AgentId, ChannelId, Project, ProjectId},
    store::Store,
};

struct Fixture {
    _dir: tempfile::TempDir,
    memory: MemoryStore,
    store: Store,
    project: ProjectId,
    other: ProjectId,
    agents: Vec<AgentId>,
    chats: Vec<ChannelId>,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let memory = MemoryStore::open_in_memory_index(dir.path()).unwrap();
    let store = Store::open_in_memory().unwrap();
    let project = ProjectId::new();
    let other = ProjectId::new();
    for id in [&project, &other] {
        store
            .insert_project(&Project {
                id: id.clone(),
                name: id.0.clone(),
                path: format!("/tmp/{}", id.0),
            })
            .unwrap();
    }
    let mut agents = vec![];
    let mut chats = vec![];
    for name in ["Ada", "Ben"] {
        let agent = store
            .create_agent(&AgentConfig {
                name: name.into(),
                avatar: None,
                provider: "codex".into(),
                model: Some("exact".into()),
                reasoning_effort: None,
                system_prompt: String::new(),
                policy: ApprovalPolicy::DenyAll,
                plugin_write: false,
            })
            .unwrap();
        chats.push(
            store
                .create_conversation(&agent, Some(&project), name)
                .unwrap()
                .id,
        );
        agents.push(agent);
    }
    Fixture {
        _dir: dir,
        memory,
        store,
        project,
        other,
        agents,
        chats,
    }
}

#[test]
fn project_view_includes_all_project_agents_and_global_but_excludes_other_private_scopes() {
    let f = setup();
    let orphan = AgentId::new();
    let included = [
        Scope::Global,
        Scope::Project(f.project.clone()),
        Scope::AgentProject(f.agents[0].clone(), f.project.clone()),
        Scope::AgentProject(f.agents[1].clone(), f.project.clone()),
        Scope::AgentProject(orphan, f.project.clone()),
    ];
    let excluded = [
        Scope::Project(f.other.clone()),
        Scope::Agent(f.agents[0].clone()),
        Scope::AgentProject(f.agents[0].clone(), f.other.clone()),
    ];
    for scope in included.iter().chain(&excluded) {
        f.memory
            .create_item(
                scope,
                ItemKind::Fact,
                &scope.label(),
                None,
                MutationOrigin::User,
            )
            .unwrap();
    }
    let view = project_memory::view(&f.memory, &f.store, &f.project.0).unwrap();
    let items = view["items"].as_array().unwrap();
    assert_eq!(items.len(), included.len());
    for scope in included {
        assert!(items.iter().any(|i| i["scope"] == scope.label()));
    }
    assert_eq!(view["projectId"], f.project.0);
    assert!(
        view["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| !excluded.iter().any(|e| s["scope"] == e.label()))
    );
    assert!(project_memory::view(&f.memory, &f.store, "missing").is_err());
    assert!(project_memory::view(&f.memory, &f.store, "").is_err());
}

#[test]
fn explicit_project_create_and_mutations_keep_versions_and_validate_boundaries() {
    let f = setup();
    let item = project_memory::create(
        &f.memory,
        &f.store,
        &f.project.0,
        &uuid::Uuid::new_v4().to_string(),
        ItemKind::Decision,
        " Use SQLite ",
    )
    .unwrap()
    .0;
    assert_eq!(item.scope, Scope::Project(f.project.clone()));
    assert_eq!(item.content, "Use SQLite");
    assert_eq!(item.origin, MutationOrigin::User);
    assert!(item.source_conversation.is_none());
    assert!(
        project_memory::create(
            &f.memory,
            &f.store,
            &f.project.0,
            &uuid::Uuid::new_v4().to_string(),
            ItemKind::Fact,
            "  "
        )
        .is_err()
    );
    assert!(
        project_memory::create(
            &f.memory,
            &f.store,
            "missing",
            &uuid::Uuid::new_v4().to_string(),
            ItemKind::Fact,
            "test"
        )
        .is_err()
    );
    assert!(
        project_memory::mutate(
            &f.memory, &f.store, &f.other.0, &item.id, 1, "forget", None, None
        )
        .is_err()
    );
    assert!(
        project_memory::mutate(
            &f.memory,
            &f.store,
            &f.project.0,
            &item.id,
            1,
            "move",
            None,
            Some(&Scope::Project(f.other.clone()).label())
        )
        .is_err()
    );
    let updated = project_memory::mutate(
        &f.memory,
        &f.store,
        &f.project.0,
        &item.id,
        1,
        "edit",
        Some("Use local SQLite"),
        None,
    )
    .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(updated.history.len(), 1);
    assert!(
        project_memory::mutate(
            &f.memory,
            &f.store,
            &f.project.0,
            &item.id,
            1,
            "forget",
            None,
            None
        )
        .is_err()
    );
    let shared = project_memory::mutate(
        &f.memory,
        &f.store,
        &f.project.0,
        &item.id,
        2,
        "move",
        None,
        Some("global"),
    )
    .unwrap();
    assert_eq!(shared.scope, Scope::Global);
    project_memory::mutate(
        &f.memory,
        &f.store,
        &f.project.0,
        &item.id,
        3,
        "forget",
        None,
        None,
    )
    .unwrap();
    assert!(
        project_memory::view(&f.memory, &f.store, &f.project.0).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn project_proposals_span_conversations_and_reject_other_projects() {
    let f = setup();
    let mut proposal_ids = vec![];
    for (agent, chat) in f.agents.iter().zip(&f.chats) {
        let SaveOutcome::Proposed(p) = f
            .memory
            .capture_item(
                agent,
                Some(&f.project),
                chat,
                &Scope::Project(f.project.clone()),
                ItemKind::Fact,
                "shared fact",
            )
            .unwrap()
        else {
            panic!()
        };
        proposal_ids.push(p.id);
    }
    let other_chat = f
        .store
        .create_conversation(&f.agents[0], Some(&f.other), "Other")
        .unwrap()
        .id;
    let SaveOutcome::Proposed(other) = f
        .memory
        .capture_item(
            &f.agents[0],
            Some(&f.other),
            &other_chat,
            &Scope::Global,
            ItemKind::Fact,
            "unrelated proposal",
        )
        .unwrap()
    else {
        panic!()
    };
    let view = project_memory::view(&f.memory, &f.store, &f.project.0).unwrap();
    assert_eq!(view["proposals"].as_array().unwrap().len(), 2);
    assert!(project_memory::resolve(&f.memory, &f.store, &f.project.0, &other.id, true).is_err());
    assert!(project_memory::resolve(&f.memory, &f.store, &f.project.0, &other.id, false).is_err());
    project_memory::resolve(&f.memory, &f.store, &f.project.0, &proposal_ids[0], true).unwrap();
    project_memory::resolve(&f.memory, &f.store, &f.project.0, &proposal_ids[1], false).unwrap();
    assert!(
        project_memory::view(&f.memory, &f.store, &f.project.0).unwrap()["proposals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn private_and_moved_proposal_targets_stay_outside_the_project_view() {
    let f = setup();
    let private = f
        .memory
        .create_item(
            &Scope::Agent(f.agents[0].clone()),
            ItemKind::Fact,
            "private original",
            Some(&f.chats[0]),
            MutationOrigin::User,
        )
        .unwrap();
    let update = f
        .memory
        .propose_update(
            &f.agents[0],
            Some(&f.project),
            &f.chats[0],
            &private.id,
            1,
            "private correction",
        )
        .unwrap();
    let moved = f
        .memory
        .create_item(
            &Scope::Project(f.project.clone()),
            ItemKind::Fact,
            "other project secret",
            Some(&f.chats[0]),
            MutationOrigin::User,
        )
        .unwrap();
    let forget = f
        .memory
        .propose_forget(&f.agents[0], Some(&f.project), &f.chats[0], &moved.id, 1)
        .unwrap();
    f.memory
        .move_item(&moved.id, 1, &Scope::Project(f.other.clone()))
        .unwrap();
    let view = project_memory::view(&f.memory, &f.store, &f.project.0).unwrap();
    assert!(view["proposals"].as_array().unwrap().is_empty());
    assert!(!view.to_string().contains("private correction"));
    assert!(!view.to_string().contains("other project secret"));
    for id in [&update.id, &forget.id] {
        assert!(project_memory::resolve(&f.memory, &f.store, &f.project.0, id, true).is_err());
    }
}
