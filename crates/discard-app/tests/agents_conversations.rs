//! Desktop lifecycle behavior with real Store and memory persistence, without a webview.
use discard_app::desktop::{
    view::ViewId,
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
        workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
        reasoning_effort: None,
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
    core.configure_window(&ViewId::main(), data, Path::new(""), None);
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
            if store.channel_archived(&channel.id).unwrap()
                || store.agent_archived(&channel.agent_id).unwrap_or(true)
            {
                drop(receiver);
            } else {
                receivers.lock().unwrap().push(receiver);
            }
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
    channels.restore(&ViewId::main()).unwrap();
    assert!(core.snapshot(&ViewId::main()).composer_disabled);
    assert!(core.active_channel_id(&ViewId::main()).is_empty());
    assert!(store.lock().unwrap().all_agents().unwrap().is_empty());
    assert!(store.lock().unwrap().list_projects().unwrap().is_empty());
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    assert!(store.lock().unwrap().direct_message_agents().unwrap().is_empty());
    let direct = channels.open_agent(&ViewId::main(), &agent).unwrap();
    assert_eq!(store.lock().unwrap().direct_message_agents().unwrap(), vec![agent.clone()]);
    assert_eq!(core.active_context(&ViewId::main()), (agent.clone(), None));
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
    let project_dir = tempfile::tempdir().unwrap();
    let project = store
        .lock()
        .unwrap()
        .import_project_folder(project_dir.path())
        .unwrap();
    let project_one = channels.activate_project(&ViewId::main(), &project.id).unwrap().1;
    assert_ne!(core.active_channel_id(&ViewId::main()), direct);
    assert_eq!(core.snapshot(&ViewId::main()).channel, "general");
    assert_eq!(store.lock().unwrap().channels_for_project(&project.id).unwrap().len(), 1);
    assert!(channels.create_conversation(&ViewId::main(), &agent, Some(&project.id), "Second project chat").unwrap_err().contains("general channel"));
    let direct_two = channels
        .create_conversation(&ViewId::main(), &agent, None, "Another direct chat")
        .unwrap();
    assert_eq!(core.active_context(&ViewId::main()), (agent.clone(), None));
    assert_eq!(core.current_project(&ViewId::main()).id, project.id);
    channels.select(&ViewId::main(), &project_one).unwrap();
    assert_eq!(core.active_context(&ViewId::main()).1, Some(project.id.clone()));
    channels.select(&ViewId::main(), &direct_two).unwrap();
    assert_eq!(store.lock().unwrap().last_project_channel(&project.id).unwrap().unwrap().id.0, project_one, "direct selection leaves saved project navigation unchanged");
    assert_ne!(direct, direct_two);
    let restored = empty_core(dir.path());
    let reopen = service(restored.clone(), store.clone());
    reopen.restore(&ViewId::main()).unwrap();
    assert_eq!(restored.active_channel_id(&ViewId::main()), direct_two);
    assert_eq!(restored.active_context(&ViewId::main()), (agent, None));
    assert_eq!(
        store
            .lock()
            .unwrap()
            .channels_for_project(&project.id)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn project_navigation_keeps_active_and_background_responses_running() {
    use discard_core::{driver::TurnOutcome, ui_state::Update};
    let dir = tempfile::tempdir().unwrap();
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().create_agent(&config()).unwrap();
    let first = store.lock().unwrap().import_project_folder(first_dir.path()).unwrap();
    let second = store.lock().unwrap().import_project_folder(second_dir.path()).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    let first_channel = channels.activate_project(&ViewId::main(), &first.id).unwrap().1;
    core.send_to(Some(&first_channel), "First response".into()).unwrap();
    let first_generation = core.channel_runtime_generation(&first_channel).unwrap();

    assert!(core.begin_project_change().is_err(), "folder changes remain blocked");
    core.begin_project_navigation(&ViewId::main()).unwrap();
    assert!(core.begin_project_navigation(&ViewId::main()).is_err(), "navigation remains serialized");
    let second_channel = channels.activate_project(&ViewId::main(), &second.id).unwrap().1;
    core.end_project_navigation(&ViewId::main());
    assert_eq!(core.current_project(&ViewId::main()).id, second.id);
    assert_eq!(core.channel_runtime_generation(&first_channel), Some(first_generation));
    assert!(!core.channel_session_closed(&first_channel));
    assert!(core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == first_channel).unwrap().running);
    core.send_to(Some(&second_channel), "Second response".into()).unwrap();

    let reply = Message {
        id: MessageId::new(), channel_id: ChannelId(first_channel.clone()),
        role: Role::Agent, blocks: vec![Block::Markdown { text: "Completed in the background".into() }],
    };
    store.lock().unwrap().insert_message(&reply).unwrap();
    core.apply_runtime(&first_channel, Some(first_generation), Update::ResponseCompleted(reply.clone()));
    core.apply_runtime(&first_channel, Some(first_generation), Update::TurnFinished(TurnOutcome::Completed));
    assert!(core.snapshot(&ViewId::main()).turn_running, "background completion must not finish the current response");
    assert!(!core.history(&ViewId::main()).iter().any(|message| message.id == reply.id.0));

    core.begin_project_navigation(&ViewId::main()).unwrap();
    assert_eq!(channels.activate_project(&ViewId::main(), &first.id).unwrap().1, first_channel);
    core.end_project_navigation(&ViewId::main());
    assert!(core.history(&ViewId::main()).iter().any(|message| message.id == reply.id.0));
    assert!(core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == second_channel).unwrap().running);
    assert!(!core.channel_session_closed(&second_channel));
    assert!(core.begin_project_change().is_err(), "a parked response still blocks destructive changes");
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
    channels.open_agent(&ViewId::main(), &agent).unwrap();
    let second = channels
        .create_conversation(&ViewId::main(), &agent, None, "Second")
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
        core.snapshot(&ViewId::main())
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
    assert!(core.snapshot(&ViewId::main()).composer_disabled);
    assert!(core.send_to(Some(&second), "Blocked".into()).is_err());
    channels.select(&ViewId::main(), &direct.id.0).unwrap();
    assert_eq!(core.history_snapshot(&ViewId::main()).messages[0].id, message.id.0);
    store
        .lock()
        .unwrap()
        .set_agent_archived(&agent, false)
        .unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
    store.lock().unwrap().archive_channel(&direct.id).unwrap();
    channels.reload_agent(&agent.0).unwrap();
    assert!(core.snapshot(&ViewId::main()).archived);
    assert!(core.snapshot(&ViewId::main()).composer_disabled);
    assert_eq!(core.history_snapshot(&ViewId::main()).messages[0].id, message.id.0);
    channels.restore_conversation(&direct.id.0).unwrap();
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
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
    assert!(core.snapshot(&ViewId::main()).composer_disabled);
    assert_eq!(core.snapshot(&ViewId::main()).agent, "Deleted agent");
    assert_eq!(core.history_snapshot(&ViewId::main()).messages[0].id, message.id.0);
    channels.select(&ViewId::main(), &other_project.id.0).unwrap();
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
    assert_eq!(core.active_context(&ViewId::main()).1, Some(project.id));
    channels.open_agent(&ViewId::main(), &other).unwrap();
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
    channels.select(&ViewId::main(), &direct.id.0).unwrap();
    assert_eq!(core.history_snapshot(&ViewId::main()).messages[0].id, message.id.0);
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
        let output = discard_core::process::spawn_std(std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&repo).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output())
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

#[tokio::test]
async fn restoring_agent_rebuilds_only_closed_history_and_keeps_usable_unsaved_reply() {
    use discard_core::{
        driver::TurnOutcome,
        event::{Event, EventId, EventKind, SessionId},
        ui_state::Update,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let active = store.lock().unwrap().default_conversation(&agent).unwrap();
    let history = store
        .lock()
        .unwrap()
        .create_conversation(&agent, None, "Archived history")
        .unwrap();
    store.lock().unwrap().archive_channel(&history.id).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.select(&ViewId::main(), &history.id.0).unwrap();
    channels.select(&ViewId::main(), &active.id.0).unwrap();
    core.send_to(Some(&active.id.0), "Question".into()).unwrap();
    core.apply_channel(
        &active.id.0,
        Update::Event(Box::new(Event {
            id: EventId::new(),
            session_id: SessionId::new(),
            seq: 1,
            kind: EventKind::TextDelta {
                text: "Keep this unsaved reply".into(),
            },
            raw: None,
        })),
    );
    core.apply_channel(
        &active.id.0,
        Update::ResponseSaveFailed("Copy the reply before sending again.".into()),
    );
    core.apply_channel(&active.id.0, Update::TurnFinished(TurnOutcome::Failed));
    let before = core.snapshot(&ViewId::main());
    let before_history = core.history_snapshot(&ViewId::main());
    assert!(core.agent_sessions_closed(&agent.0));
    store
        .lock()
        .unwrap()
        .set_agent_archived(&agent, true)
        .unwrap();
    core.refresh_agent(&agent.0, &config(), true);
    store
        .lock()
        .unwrap()
        .set_agent_archived(&agent, false)
        .unwrap();
    channels.reload_agent(&agent.0).unwrap();
    let after = core.snapshot(&ViewId::main());
    assert_eq!(after.channel_id, before.channel_id);
    assert_eq!(
        after.live_blocks, before.live_blocks,
        "restoring parked history must not discard a usable conversation's reply"
    );
    assert_eq!(after.notice, before.notice);
    assert_eq!(core.history_snapshot(&ViewId::main()).messages, before_history.messages);
    assert!(
        core.has_channel(&history.id.0),
        "the parked history must remain cached"
    );
    channels.select(&ViewId::main(), &history.id.0).unwrap();
    assert!(core.snapshot(&ViewId::main()).composer_disabled);
    channels.restore_conversation(&history.id.0).unwrap();
    assert!(!core.snapshot(&ViewId::main()).composer_disabled);
    channels.select(&ViewId::main(), &active.id.0).unwrap();
    assert_eq!(core.snapshot(&ViewId::main()).live_blocks, before.live_blocks);
    assert_eq!(core.snapshot(&ViewId::main()).notice, before.notice);
}

#[tokio::test]
async fn folder_reload_preserves_unsaved_replies_and_only_restarts_affected_transports() {
    use discard_core::{
        driver::TurnOutcome,
        event::{Event, EventId, EventKind, SessionId},
        ui_state::Update,
    };
    let dir = tempfile::tempdir().unwrap();
    let replacement = tempfile::tempdir().unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let project = store
        .lock()
        .unwrap()
        .import_project_folder(dir.path())
        .unwrap();
    let other = store
        .lock()
        .unwrap()
        .import_project_folder(other_dir.path())
        .unwrap();
    let direct = store.lock().unwrap().default_conversation(&agent).unwrap();
    let affected = store
        .lock()
        .unwrap()
        .create_conversation(&agent, Some(&project.id), "Affected")
        .unwrap();
    let parked = store
        .lock()
        .unwrap()
        .create_conversation(&agent, Some(&project.id), "Parked affected")
        .unwrap();
    let unrelated = store
        .lock()
        .unwrap()
        .create_conversation(&agent, Some(&other.id), "Other project")
        .unwrap();
    let fail_launch = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let launches = Arc::new(Mutex::new(Vec::new()));
    let receivers = Arc::new(Mutex::new(Vec::new()));
    let core = empty_core(dir.path());
    let launch: Launcher = {
        let fail_launch = fail_launch.clone();
        let launches = launches.clone();
        let store = store.clone();
        let receivers = receivers.clone();
        Arc::new(move |channel| {
            if fail_launch.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("test replacement launch failure".into());
            }
            let store = store.lock().unwrap();
            let folder = channel.project_id.as_ref().map(|id| {
                store
                    .list_projects()
                    .unwrap()
                    .into_iter()
                    .find(|p| p.id == *id)
                    .unwrap()
                    .path
            });
            launches.lock().unwrap().push((channel.id.clone(), folder));
            let (commands, receiver) = mpsc::unbounded_channel();
            receivers.lock().unwrap().push(receiver);
            let (_, updates) = mpsc::unbounded_channel();
            Ok((
                CoreInit {
                    state: UiState::new(
                        channel.id.clone(),
                        store.messages_for_channel(&channel.id).unwrap(),
                    ),
                    commands,
                    project: String::new(),
                    channel: channel.name.clone(),
                    agent: "Agent".into(),
                    provider: "codex".into(),
                },
                updates,
            ))
        })
    };
    let channels = Channels::new(
        core.clone(),
        store.clone(),
        tokio::runtime::Handle::current(),
        launch,
    );
    let mut retained = Vec::new();
    for channel in [&direct, &unrelated, &parked, &affected] {
        store
            .lock()
            .unwrap()
            .insert_message(&Message {
                id: MessageId::new(),
                channel_id: channel.id.clone(),
                role: Role::Agent,
                blocks: vec![Block::Markdown {
                    text: format!("Saved {}", channel.name),
                }],
            })
            .unwrap();
        channels.select(&ViewId::main(), &channel.id.0).unwrap();
        core.send_to(Some(&channel.id.0), "Question".into())
            .unwrap();
        assert!(core.begin_project_change().is_err());
        core.apply(&ViewId::main(), Update::Event(Box::new(Event {
            id: EventId::new(),
            session_id: SessionId::new(),
            seq: 1,
            kind: EventKind::TextDelta {
                text: format!("Unsaved {}", channel.name),
            },
            raw: None,
        })));
        core.apply(&ViewId::main(), Update::ResponseSaveFailed(
            "Copy the reply before sending again.".into(),
        ));
        core.apply(&ViewId::main(), Update::TurnFinished(TurnOutcome::Failed));
        retained.push((
            channel.id.0.clone(),
            core.snapshot(&ViewId::main()),
            core.history_snapshot(&ViewId::main()),
            core.runtime_generation(&ViewId::main()),
        ));
    }
    assert!(
        channels.reload_project(&project.id).is_err(),
        "reload needs the lifecycle guard"
    );
    core.begin_project_change().unwrap();
    assert!(
        core.send_to(Some(&affected.id.0), "Blocked".into())
            .is_err()
    );
    store
        .lock()
        .unwrap()
        .change_project_folder(&project.id, replacement.path())
        .unwrap();
    channels.activate_project(&ViewId::main(), &project.id).unwrap();
    channels.reload_project(&project.id).unwrap();
    assert_eq!(core.active_channel_id(&ViewId::main()), affected.id.0);
    core.end_project_change();
    for (id, before, history, old_generation) in retained {
        channels.select(&ViewId::main(), &id).unwrap();
        assert_eq!(
            core.snapshot(&ViewId::main()).live_blocks,
            before.live_blocks,
            "retained reply for {id}"
        );
        assert_eq!(core.snapshot(&ViewId::main()).notice, before.notice);
        assert_eq!(core.history_snapshot(&ViewId::main()).messages, history.messages);
        let is_affected = id == affected.id.0 || id == parked.id.0;
        assert_eq!(core.runtime_generation(&ViewId::main()) == old_generation, !is_affected);
        core.apply_runtime(
            &id,
            Some(old_generation),
            Update::ResponseSaveFailed("late old transport".into()),
        );
        assert_eq!(
            core.snapshot(&ViewId::main()).notice.as_deref() == Some("late old transport"),
            !is_affected
        );
    }
    let launches = launches.lock().unwrap();
    assert_eq!(launches.len(), 6, "only two affected transports restart");
    for (id, path) in &launches[4..] {
        assert!(id == &affected.id || id == &parked.id);
        assert_eq!(
            std::path::Path::new(path.as_ref().unwrap())
                .canonicalize()
                .unwrap(),
            replacement.path().canonicalize().unwrap()
        );
    }
    drop(launches);
    // A launch failure closes only affected transports and leaves their cached replies reachable.
    channels.select(&ViewId::main(), &direct.id.0).unwrap();
    fail_launch.store(true, std::sync::atomic::Ordering::SeqCst);
    core.begin_project_change().unwrap();
    assert!(
        channels
            .reload_project(&project.id)
            .unwrap_err()
            .contains("test replacement")
    );
    core.end_project_change();
    assert_eq!(core.active_channel_id(&ViewId::main()), direct.id.0);
    assert!(!core.channel_session_closed(&direct.id.0));
    assert!(core.channel_session_closed(&parked.id.0));
    let direct_notice = core.snapshot(&ViewId::main()).notice;
    assert!(channels.select(&ViewId::main(), "missing-history").is_err());
    assert_eq!(
        core.active_channel_id(&ViewId::main()),
        direct.id.0,
        "failed history selection must not open another conversation"
    );
    assert_eq!(core.snapshot(&ViewId::main()).notice, direct_notice);
    channels.select(&ViewId::main(), &parked.id.0).unwrap();
    assert_eq!(
        core.active_channel_id(&ViewId::main()),
        parked.id.0,
        "failed retry must still open retained history"
    );
    assert!(!core.snapshot(&ViewId::main()).live_blocks.is_empty());
    assert_eq!(
        core.history_snapshot(&ViewId::main()).messages.len(),
        2,
        "saved reply and current question stay visible"
    );
    let retained = core.snapshot(&ViewId::main());
    let notice = retained.notice.as_deref().unwrap();
    assert!(notice.contains("Copy the reply before sending again."));
    assert!(notice.contains("test replacement launch failure"));
    assert!(notice.contains("Conversation history is open"));
    channels.select(&ViewId::main(), &parked.id.0).unwrap();
    assert_eq!(
        core.snapshot(&ViewId::main()).notice,
        retained.notice,
        "repeated restart failure must not duplicate the notice"
    );
    fail_launch.store(false, std::sync::atomic::Ordering::SeqCst);
    channels.select(&ViewId::main(), &parked.id.0).unwrap();
    assert!(!core.channel_session_closed(&parked.id.0));
    assert_eq!(core.snapshot(&ViewId::main()).live_blocks, retained.live_blocks);
    assert_eq!(core.snapshot(&ViewId::main()).notice, retained.notice);
}

fn matrix(provider: &str) -> discard_core::capability::CapabilityMatrix {
    use discard_core::capability::{Capabilities, CapabilityMatrix};
    CapabilityMatrix { provider: provider.into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } }
}
fn session_row(channel: &ChannelId, agent: &discard_core::model::AgentId) -> discard_core::store::AgentSession {
    discard_core::store::AgentSession {
        channel_id: channel.clone(), agent_id: agent.clone(), handle: "h".into(), fingerprint: "{}".into(), delivered_through: None,
        delivered_memory: Default::default(), delivered_topic_version: None, delivered_capture_paused: false, bytes_in_session: 1, cwd: "/".into(), dropped_reason: None,
    }
}

#[tokio::test]
async fn clearing_a_channel_marks_the_timeline_drops_sessions_and_replaces_the_idle_transport() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let chat = store.lock().unwrap().default_conversation(&agent).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.open_agent(&ViewId::main(), &agent).unwrap();
    store.lock().unwrap().put_agent_session(&session_row(&chat.id, &agent)).unwrap();
    let before = core.channel_runtime_generation(&chat.id.0).unwrap();

    channels.clear_context(&chat.id.0).unwrap();

    assert!(store.lock().unwrap().agent_session(&chat.id, &agent).unwrap().is_none());
    let saved = store.lock().unwrap().messages_for_channel(&chat.id).unwrap();
    let marker = saved.last().unwrap();
    assert_eq!(marker.role, Role::System);
    assert_eq!(store.lock().unwrap().conversation_context(&chat.id).unwrap().reset_from.as_ref(), Some(&marker.id));
    let shown = core.history_snapshot(&ViewId::main()).messages;
    assert_eq!(shown.last().map(|m| m.id.as_str()), Some(marker.id.0.as_str()), "the timeline shows the marker");
    assert!(core.channel_runtime_generation(&chat.id.0).unwrap() > before, "the idle transport was replaced");
    assert!(!core.channel_session_closed(&chat.id.0), "and the new one is open");
}

#[tokio::test]
async fn clearing_is_refused_while_a_turn_runs_and_in_a_thread() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    let chat = store.lock().unwrap().default_conversation(&agent).unwrap();
    let anchor = Message { id: MessageId::new(), channel_id: chat.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "q".into() }] };
    store.lock().unwrap().insert_message(&anchor).unwrap();
    let thread = store.lock().unwrap().create_thread(&chat.id, &anchor.id, "T", &agent).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.open_agent(&ViewId::main(), &agent).unwrap();
    store.lock().unwrap().put_agent_session(&session_row(&chat.id, &agent)).unwrap();
    assert_eq!(channels.clear_context(&thread.id.0).unwrap_err(), "/clear does not apply to a thread.");
    core.send_to(Some(&chat.id.0), "Hello".into()).unwrap();
    assert_eq!(channels.clear_context(&chat.id.0).unwrap_err(), "Stop the turn before clearing the session.");
    assert!(store.lock().unwrap().agent_session(&chat.id, &agent).unwrap().is_some(), "a refused clear changes nothing");
    assert_eq!(store.lock().unwrap().messages_for_channel(&chat.id).unwrap().len(), 1);
}

#[tokio::test]
async fn a_model_change_saves_through_the_editor_path_for_every_channel_and_waits_for_idle() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    store.lock().unwrap().default_conversation(&agent).unwrap();
    let second = store.lock().unwrap().create_conversation(&agent, None, "Second").unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    channels.open_agent(&ViewId::main(), &agent).unwrap();
    channels.select(&ViewId::main(), &second.id.0).unwrap();
    // A running turn blocks the change, and nothing is saved.
    core.send_to(Some(&second.id.0), "Hello".into()).unwrap();
    let blocked = channels.change_agent_model(&agent, "new-model", &matrix("codex")).unwrap_err();
    assert!(blocked.starts_with("Stop the turn of ") && blocked.ends_with(" before changing its model."), "{blocked}");
    assert_eq!(store.lock().unwrap().load_agent(&agent).unwrap().model.as_deref(), Some("provider/exact-model:1"));
    core.apply_channel(&second.id.0, discard_core::ui_state::Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
    // Idle: the model changes and only the model.
    let before = store.lock().unwrap().load_agent(&agent).unwrap();
    let saved = channels.change_agent_model(&agent, "new-model", &matrix("codex")).unwrap();
    assert_eq!(saved, discard_core::agent::AgentConfig { model: Some("new-model".into()), ..before });
    assert_eq!(store.lock().unwrap().load_agent(&agent).unwrap(), saved, "the one saved config serves every channel of the agent");
    assert!(channels.change_agent_model(&agent, "new-model", &matrix("codex")).unwrap_err().ends_with("already uses new-model."));
    assert!(channels.change_agent_model(&agent, "x", &matrix("other")).is_err(), "the provider check of the editor applies");
}

#[tokio::test]
async fn a_review_request_posts_into_the_open_channel_and_a_running_turn_refuses_it() {
    use discard_app::desktop::review::{decide, request_summary, SUMMARY_REQUEST};
    use discard_core::review_state::ReviewState;
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = discard_core::process::spawn_std(std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .current_dir(repo.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()))
            .and_then(|child| child.wait_with_output())
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "--allow-empty", "-qm", "initial"]);
    git(&["checkout", "-q", "-b", "feature"]);
    git(&["commit", "--allow-empty", "-qm", "work"]);
    let head = git(&["rev-parse", "refs/heads/feature"]);
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().create_agent(&config()).unwrap();
    let project = store.lock().unwrap().import_project_folder(repo.path()).unwrap();
    let core = empty_core(dir.path());
    let channels = service(core.clone(), store.clone());
    let channel = ChannelId(channels.activate_project(&ViewId::main(), &project.id).unwrap().1);
    store.lock().unwrap().bind_branch(&channel, "feature").unwrap();
    let send = |text: String| core.send_to_agent(Some(&channel.0), text, None);
    let s = store.lock().unwrap();
    let outcome = decide(&s, &channel, repo.path(), core.channel_busy_reason(&channel.0), "changes_requested", &head, Some("Rename it."), send).unwrap();
    assert_eq!(outcome.post_error, None);
    assert_eq!(outcome.dto.state, ReviewState::ChangesRequested);
    let expected = format!("Review of {}: Rename it.", &head[..7]);
    let history = core.history_snapshot(&ViewId::main());
    let texts = format!("{:?}", history.messages);
    assert!(texts.contains(&expected), "the note is a message of the channel: {texts}");
    // The posted message starts a turn. The channel is busy until it ends.
    let busy = core.channel_busy_reason(&channel.0);
    assert!(busy.is_some());
    let refused = request_summary(&s, &channel, busy, |text| core.send_to_agent(Some(&channel.0), text, None));
    assert_eq!(refused.err().as_deref(), busy);
    core.apply(&ViewId::main(), discard_core::ui_state::Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
    request_summary(&s, &channel, core.channel_busy_reason(&channel.0), |text| core.send_to_agent(Some(&channel.0), text, None)).unwrap();
    assert!(format!("{:?}", core.history_snapshot(&ViewId::main()).messages).contains(SUMMARY_REQUEST));
}
