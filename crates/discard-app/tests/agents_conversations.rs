//! Desktop lifecycle behavior with real Store and memory persistence, without a webview.
use discard_app::desktop::{
    channels::{Channels, Launcher},
    core::{AppCore, CoreInit},
};
use discard_core::{
    agent::{AgentConfig, generated_name},
    capability::ApprovalPolicy,
    model::{Block, ChannelId, Message, MessageId, Role},
    store::{DeleteAgentOptions, Store},
    ui_state::{Command, UiState},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

fn config() -> AgentConfig {
    AgentConfig {
        name: generated_name(),
        avatar: Some("avatar.png".into()),
        provider: "codex".into(),
        model: Some("provider/exact-model:1".into()),
        system_prompt: String::new(),
        policy: ApprovalPolicy::DenyAll,
        plugin_write: false,
    }
}
fn empty_core(data: &Path) -> Arc<AppCore> {
    let (commands, _) = mpsc::unbounded_channel();
    let core = Arc::new(AppCore::new(CoreInit {
        state: UiState::new(ChannelId(String::new()), vec![]),
        commands,
        project: String::new(),
        channel: String::new(),
        agent: String::new(),
        provider: String::new(),
    }));
    core.configure_window(data, Path::new(""), None);
    core
}
fn service(core: Arc<AppCore>, store: Arc<Mutex<Store>>) -> Channels {
    // Keep command receivers alive so sending exercises the real core running/idle guards.
    let receivers = Arc::new(Mutex::new(Vec::<mpsc::UnboundedReceiver<Command>>::new()));
    let launch: Launcher = {
        let store = store.clone();
        Arc::new(move |channel| {
            let store = store.lock().unwrap();
            let cfg = store.load_agent(&channel.agent_id).unwrap();
            let history = store.messages_for_channel(&channel.id).unwrap();
            let (commands, receiver) = mpsc::unbounded_channel();
            receivers.lock().unwrap().push(receiver);
            let (_, updates) = mpsc::unbounded_channel();
            Ok((
                CoreInit {
                    state: UiState::new(channel.id.clone(), history),
                    commands,
                    project: String::new(),
                    channel: channel.name.clone(),
                    agent: cfg.name,
                    provider: cfg.provider,
                },
                updates,
            ))
        })
    };
    Channels::new(core, store, tokio::runtime::Handle::current(), launch)
}
#[tokio::test]
async fn fresh_navigation_direct_chats_fixed_projects_and_saved_selection_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.restore().unwrap();
    assert!(core.snapshot().composer_disabled);
    assert!(core.active_channel_id().is_empty());
    assert!(store.lock().unwrap().all_agents().unwrap().is_empty());
    assert!(store.lock().unwrap().list_projects().unwrap().is_empty());
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let direct = channels.open_agent(&agent).unwrap();
    assert_eq!(core.active_context(), (agent.clone(), None));
    assert!(!core.snapshot().composer_disabled);
    let project_dir = tempfile::tempdir().unwrap();
    let project = store
        .lock()
        .unwrap()
        .import_project_folder(project_dir.path())
        .unwrap();
    channels.activate_project(&project.id).unwrap();
    assert_eq!(core.active_channel_id(), direct);
    assert!(
        store
            .lock()
            .unwrap()
            .channels_for_project(&project.id)
            .unwrap()
            .is_empty()
    );
    let project_one = channels
        .create_conversation(&agent, Some(&project.id), "First project chat")
        .unwrap();
    let project_two = channels
        .create_conversation(&agent, Some(&project.id), "Second project chat")
        .unwrap();
    let direct_two = channels
        .create_conversation(&agent, None, "Another direct chat")
        .unwrap();
    assert_eq!(core.active_context(), (agent.clone(), None));
    assert_eq!(core.current_project().id, project.id);
    channels.select(&project_one).unwrap();
    assert_eq!(core.active_context().1, Some(project.id.clone()));
    channels.select(&direct_two).unwrap();
    assert_ne!(project_one, project_two);
    assert_ne!(direct, direct_two);
    let restored = empty_core(dir.path());
    let reopen = service(restored.clone(), store.clone());
    reopen.restore().unwrap();
    assert_eq!(restored.active_channel_id(), direct_two);
    assert_eq!(restored.active_context(), (agent, None));
    assert_eq!(
        store
            .lock()
            .unwrap()
            .channels_for_project(&project.id)
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
async fn labels_refresh_running_guards_archive_restore_and_deleted_history_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let other = store.lock().unwrap().create_agent(&config()).unwrap();
    let project = store
        .lock()
        .unwrap()
        .import_project_folder(dir.path())
        .unwrap();
    let other_project = store
        .lock()
        .unwrap()
        .create_conversation(&other, Some(&project.id), "Other agent project chat")
        .unwrap();
    store
        .lock()
        .unwrap()
        .create_conversation(&agent, Some(&project.id), "Deleted agent project chat")
        .unwrap();
    let direct = store.lock().unwrap().default_conversation(&agent).unwrap();
    let message = Message {
        id: MessageId::new(),
        channel_id: direct.id.clone(),
        role: Role::Agent,
        blocks: vec![Block::Markdown {
            text: "Saved reply".into(),
        }],
    };
    store.lock().unwrap().insert_message(&message).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.open_agent(&agent).unwrap();
    let second = channels
        .create_conversation(&agent, None, "Second")
        .unwrap();
    core.send_to(Some(&second), "Hello".into()).unwrap();
    assert!(
        core.begin_agent_change(&agent.0)
            .unwrap_err()
            .contains("Stop")
    );
    let mut cfg = store.lock().unwrap().load_agent(&agent).unwrap();
    cfg.name = "River".into();
    cfg.provider = "claude".into();
    store.lock().unwrap().update_agent(&agent, &cfg).unwrap();
    core.refresh_agent(&agent.0, &cfg, false);
    assert!(
        core.snapshot()
            .channels
            .iter()
            .filter(|c| c.agent_id == agent.0)
            .all(|c| c.agent == "River" && c.provider == "claude")
    );
    core.apply_channel(
        &second,
        discard_core::ui_state::Update::TurnFinished(discard_core::driver::TurnOutcome::Completed),
    );
    core.begin_agent_change(&agent.0).unwrap();
    assert!(core.send_to(Some(&second), "Blocked".into()).is_err());
    core.end_agent_change(&agent.0);
    store
        .lock()
        .unwrap()
        .set_agent_archived(&agent, true)
        .unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(core.snapshot().composer_disabled);
    assert!(core.send_to(Some(&second), "Blocked".into()).is_err());
    channels.select(&direct.id.0).unwrap();
    assert_eq!(core.history_snapshot().messages[0].id, message.id.0);
    store
        .lock()
        .unwrap()
        .set_agent_archived(&agent, false)
        .unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(!core.snapshot().composer_disabled);
    store.lock().unwrap().archive_channel(&direct.id).unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(core.snapshot().archived);
    assert!(core.snapshot().composer_disabled);
    assert_eq!(core.history_snapshot().messages[0].id, message.id.0);
    channels.restore_conversation(&direct.id.0).unwrap();
    assert!(!core.snapshot().composer_disabled);
    store
        .lock()
        .unwrap()
        .delete_agent(
            &agent,
            DeleteAgentOptions {
                delete_conversations: false,
                delete_private_memory: false,
            },
        )
        .unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(core.snapshot().composer_disabled);
    assert_eq!(core.snapshot().agent, "Deleted agent");
    assert_eq!(core.history_snapshot().messages[0].id, message.id.0);
    channels.select(&other_project.id.0).unwrap();
    assert!(!core.snapshot().composer_disabled);
    assert_eq!(core.active_context().1, Some(project.id));
    channels.open_agent(&other).unwrap();
    assert!(!core.snapshot().composer_disabled);
    channels.select(&direct.id.0).unwrap();
    assert_eq!(core.history_snapshot().messages[0].id, message.id.0);
}

#[test]
fn all_deletion_choices_preserve_shared_memory_and_include_orphan_private_scopes() {
    use discard_app::desktop::settings::agents::delete_agent_data;
    use discard_core::{
        memory::{MemoryStore, Scope},
        memory_items::{ItemKind, MutationOrigin},
    };
    for delete_conversations in [false, true] {
        for delete_private_memory in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open_in_memory().unwrap();
            let memory =
                MemoryStore::open(&dir.path().join("memory"), &dir.path().join("memory.db"))
                    .unwrap();
            let agent = store.create_agent(&config()).unwrap();
            let other = store.create_agent(&config()).unwrap();
            let project = store.import_project_folder(dir.path()).unwrap();
            let scopes = [
                Scope::Global,
                Scope::Project(project.id.clone()),
                Scope::Agent(agent.clone()),
                Scope::AgentProject(agent.clone(), project.id.clone()),
                Scope::Agent(other.clone()),
            ];
            for (i, scope) in scopes.iter().enumerate() {
                memory
                    .create_item(
                        scope,
                        ItemKind::Fact,
                        &format!("Durable fact number {i}"),
                        None,
                        MutationOrigin::User,
                    )
                    .unwrap();
            }
            let chat = store.default_conversation(&agent).unwrap();
            store
                .insert_message(&Message {
                    id: MessageId::new(),
                    channel_id: chat.id.clone(),
                    role: Role::Agent,
                    blocks: vec![Block::Markdown {
                        text: "Keep me".into(),
                    }],
                })
                .unwrap();
            // No project conversation exists. The private agent-project scope is an orphan.
            assert!(store.channels_for_project(&project.id).unwrap().is_empty());
            delete_agent_data(
                &store,
                &memory,
                &agent,
                DeleteAgentOptions {
                    delete_conversations,
                    delete_private_memory,
                },
            )
            .unwrap();
            assert_eq!(
                memory
                    .list_items(&[
                        Scope::Global,
                        Scope::Project(project.id.clone()),
                        Scope::Agent(other)
                    ])
                    .unwrap()
                    .len(),
                3
            );
            assert_eq!(
                memory
                    .list_items(&[
                        Scope::Agent(agent.clone()),
                        Scope::AgentProject(agent.clone(), project.id)
                    ])
                    .unwrap()
                    .len(),
                if delete_private_memory { 0 } else { 2 }
            );
            assert_eq!(store.load_channel(&chat.id).is_err(), delete_conversations);
            if !delete_conversations {
                assert_eq!(store.messages_for_channel(&chat.id).unwrap().len(), 1);
            }
        }
    }
}
#[test]
fn deletion_preflights_every_owned_checkout_and_preserves_dirty_and_borrowed_worktrees() {
    use discard_app::desktop::settings::agents::clean_agent_worktrees;
    use discard_core::{store::WorktreeRecord, worktree::WorktreeManager};
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["commit", "--allow-empty", "-m", "initial"]);
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&config()).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let manager = WorktreeManager::new(&dir.path().join("worktrees"));
    let mut roots = Vec::new();
    let mut records = Vec::new();
    for name in ["one", "two"] {
        let channel = store
            .create_conversation(&agent, Some(&project.id), name)
            .unwrap();
        let prepared = manager.prepare(&repo, &channel.id.0, name).unwrap();
        let tree = prepared.worktree;
        let record = WorktreeRecord {
            channel_id: channel.id,
            path: tree.root.to_string_lossy().into_owned(),
            repo: tree.repo.to_string_lossy().into_owned(),
            branch: tree.branch,
            base_commit: tree.base_commit,
            owned: true,
        };
        store.save_worktree(&record).unwrap();
        roots.push(tree.root);
        records.push(record);
    }
    std::fs::write(roots[1].join("unsaved.txt"), "Do not delete").unwrap();
    assert!(
        clean_agent_worktrees(&store, &agent)
            .unwrap_err()
            .contains("Save the work")
    );
    assert!(
        roots.iter().all(|root| root.exists()),
        "preflight must finish before removing any checkout"
    );
    assert!(
        store
            .worktree_for(&records[0].channel_id)
            .unwrap()
            .is_some()
    );
    std::fs::remove_file(roots[1].join("unsaved.txt")).unwrap();
    records[1].owned = false;
    store.save_worktree(&records[1]).unwrap();
    assert!(
        clean_agent_worktrees(&store, &agent)
            .unwrap_err()
            .contains("user-owned")
    );
    assert!(roots.iter().all(|root| root.exists()));
    records[1].owned = true;
    store.save_worktree(&records[1]).unwrap();
    clean_agent_worktrees(&store, &agent).unwrap();
    assert!(roots.iter().all(|root| !root.exists()));
}
