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
        workspace_write: false,
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
    channels.restore().unwrap();
    assert!(core.snapshot().composer_disabled);
    assert!(core.active_channel_id().is_empty());
    assert!(store.lock().unwrap().all_agents().unwrap().is_empty());
    assert!(store.lock().unwrap().list_projects().unwrap().is_empty());
    let agent = store.lock().unwrap().create_agent(&config()).unwrap();
    assert!(store.lock().unwrap().direct_message_agents().unwrap().is_empty());
    let direct = channels.open_agent(&agent).unwrap();
    assert_eq!(store.lock().unwrap().direct_message_agents().unwrap(), vec![agent.clone()]);
    assert_eq!(core.active_context(), (agent.clone(), None));
    assert!(!core.snapshot().composer_disabled);
    let project_dir = tempfile::tempdir().unwrap();
    let project = store
        .lock()
        .unwrap()
        .import_project_folder(project_dir.path())
        .unwrap();
    let project_one = channels.activate_project(&project.id).unwrap();
    assert_ne!(core.active_channel_id(), direct);
    assert_eq!(core.snapshot().channel, "general");
    assert_eq!(store.lock().unwrap().channels_for_project(&project.id).unwrap().len(), 1);
    assert!(channels.create_conversation(&agent, Some(&project.id), "Second project chat").unwrap_err().contains("general channel"));
    let direct_two = channels
        .create_conversation(&agent, None, "Another direct chat")
        .unwrap();
    assert_eq!(core.active_context(), (agent.clone(), None));
    assert_eq!(core.current_project().id, project.id);
    channels.select(&project_one).unwrap();
    assert_eq!(core.active_context().1, Some(project.id.clone()));
    channels.select(&direct_two).unwrap();
    assert_eq!(store.lock().unwrap().last_project_channel(&project.id).unwrap().unwrap().id.0, project_one, "direct selection leaves saved project navigation unchanged");
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
    let first_channel = channels.activate_project(&first.id).unwrap();
    core.send_to(Some(&first_channel), "First response".into()).unwrap();
    let first_generation = core.channel_runtime_generation(&first_channel).unwrap();

    assert!(core.begin_project_change().is_err(), "folder changes remain blocked");
    core.begin_project_navigation().unwrap();
    assert!(core.begin_project_navigation().is_err(), "navigation remains serialized");
    let second_channel = channels.activate_project(&second.id).unwrap();
    core.end_project_change();
    assert_eq!(core.current_project().id, second.id);
    assert_eq!(core.channel_runtime_generation(&first_channel), Some(first_generation));
    assert!(!core.channel_session_closed(&first_channel));
    assert!(core.snapshot().channels.iter().find(|c| c.id == first_channel).unwrap().running);
    core.send_to(Some(&second_channel), "Second response".into()).unwrap();

    let reply = Message {
        id: MessageId::new(), channel_id: ChannelId(first_channel.clone()),
        role: Role::Agent, blocks: vec![Block::Markdown { text: "Completed in the background".into() }],
    };
    store.lock().unwrap().insert_message(&reply).unwrap();
    core.apply_runtime(&first_channel, Some(first_generation), Update::ResponseCompleted(reply.clone()));
    core.apply_runtime(&first_channel, Some(first_generation), Update::TurnFinished(TurnOutcome::Completed));
    assert!(core.snapshot().turn_running, "background completion must not finish the current response");
    assert!(!core.history().iter().any(|message| message.id == reply.id.0));

    core.begin_project_navigation().unwrap();
    assert_eq!(channels.activate_project(&first.id).unwrap(), first_channel);
    core.end_project_change();
    assert!(core.history().iter().any(|message| message.id == reply.id.0));
    assert!(core.snapshot().channels.iter().find(|c| c.id == second_channel).unwrap().running);
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
    channels.select(&history.id.0).unwrap();
    channels.select(&active.id.0).unwrap();
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
    let before = core.snapshot();
    let before_history = core.history_snapshot();
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
    let after = core.snapshot();
    assert_eq!(after.channel_id, before.channel_id);
    assert_eq!(
        after.live_blocks, before.live_blocks,
        "restoring parked history must not discard a usable conversation's reply"
    );
    assert_eq!(after.notice, before.notice);
    assert_eq!(core.history_snapshot().messages, before_history.messages);
    assert!(
        core.has_channel(&history.id.0),
        "the parked history must remain cached"
    );
    channels.select(&history.id.0).unwrap();
    assert!(core.snapshot().composer_disabled);
    channels.restore_conversation(&history.id.0).unwrap();
    assert!(!core.snapshot().composer_disabled);
    channels.select(&active.id.0).unwrap();
    assert_eq!(core.snapshot().live_blocks, before.live_blocks);
    assert_eq!(core.snapshot().notice, before.notice);
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
        channels.select(&channel.id.0).unwrap();
        core.send_to(Some(&channel.id.0), "Question".into())
            .unwrap();
        assert!(core.begin_project_change().is_err());
        core.apply(Update::Event(Box::new(Event {
            id: EventId::new(),
            session_id: SessionId::new(),
            seq: 1,
            kind: EventKind::TextDelta {
                text: format!("Unsaved {}", channel.name),
            },
            raw: None,
        })));
        core.apply(Update::ResponseSaveFailed(
            "Copy the reply before sending again.".into(),
        ));
        core.apply(Update::TurnFinished(TurnOutcome::Failed));
        retained.push((
            channel.id.0.clone(),
            core.snapshot(),
            core.history_snapshot(),
            core.runtime_generation(),
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
    channels.activate_project(&project.id).unwrap();
    channels.reload_project(&project.id).unwrap();
    assert_eq!(core.active_channel_id(), affected.id.0);
    core.end_project_change();
    for (id, before, history, old_generation) in retained {
        channels.select(&id).unwrap();
        assert_eq!(
            core.snapshot().live_blocks,
            before.live_blocks,
            "retained reply for {id}"
        );
        assert_eq!(core.snapshot().notice, before.notice);
        assert_eq!(core.history_snapshot().messages, history.messages);
        let is_affected = id == affected.id.0 || id == parked.id.0;
        assert_eq!(core.runtime_generation() == old_generation, !is_affected);
        core.apply_runtime(
            &id,
            Some(old_generation),
            Update::ResponseSaveFailed("late old transport".into()),
        );
        assert_eq!(
            core.snapshot().notice.as_deref() == Some("late old transport"),
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
    channels.select(&direct.id.0).unwrap();
    fail_launch.store(true, std::sync::atomic::Ordering::SeqCst);
    core.begin_project_change().unwrap();
    assert!(
        channels
            .reload_project(&project.id)
            .unwrap_err()
            .contains("test replacement")
    );
    core.end_project_change();
    assert_eq!(core.active_channel_id(), direct.id.0);
    assert!(!core.channel_session_closed(&direct.id.0));
    assert!(core.channel_session_closed(&parked.id.0));
    let direct_notice = core.snapshot().notice;
    assert!(channels.select("missing-history").is_err());
    assert_eq!(
        core.active_channel_id(),
        direct.id.0,
        "failed history selection must not open another conversation"
    );
    assert_eq!(core.snapshot().notice, direct_notice);
    channels.select(&parked.id.0).unwrap();
    assert_eq!(
        core.active_channel_id(),
        parked.id.0,
        "failed retry must still open retained history"
    );
    assert!(!core.snapshot().live_blocks.is_empty());
    assert_eq!(
        core.history_snapshot().messages.len(),
        2,
        "saved reply and current question stay visible"
    );
    let retained = core.snapshot();
    let notice = retained.notice.as_deref().unwrap();
    assert!(notice.contains("Copy the reply before sending again."));
    assert!(notice.contains("test replacement launch failure"));
    assert!(notice.contains("Conversation history is open"));
    channels.select(&parked.id.0).unwrap();
    assert_eq!(
        core.snapshot().notice,
        retained.notice,
        "repeated restart failure must not duplicate the notice"
    );
    fail_launch.store(false, std::sync::atomic::Ordering::SeqCst);
    channels.select(&parked.id.0).unwrap();
    assert!(!core.channel_session_closed(&parked.id.0));
    assert_eq!(core.snapshot().live_blocks, retained.live_blocks);
    assert_eq!(core.snapshot().notice, retained.notice);
}
