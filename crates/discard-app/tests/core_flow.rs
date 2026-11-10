use discard_app::desktop::{
    core::{AppCore, CoreInit},
    dto::{HistoryDto, SnapshotDto},
    pump::{Emit, pump},
};
use discard_core::agent::AgentConfig;
use discard_core::capability::ApprovalPolicy;
use discard_core::engine::Engine;
use discard_core::event::{Event, EventId, EventKind, SessionId};
use discard_core::model::{AgentId, Channel, Project, ProjectId, WorkspaceMode};
use discard_core::model::{Block, ChannelId};
use discard_core::store::Store;
use discard_core::ui_state::{UiState, Update};
use discard_providers::fake::{FakeProvider, Step};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Default)]
struct RecordingEmit(Mutex<Vec<SnapshotDto>>);
impl Emit for RecordingEmit {
    fn snapshot(&self, value: &SnapshotDto) {
        self.0.lock().unwrap().push(value.clone());
    }
    fn history(&self, _: &HistoryDto) {}
}
fn text() -> Update {
    Update::Event(Box::new(Event {
        id: EventId::new(),
        session_id: SessionId::new(),
        seq: 1,
        kind: EventKind::TextDelta { text: "x".into() },
        raw: None,
    }))
}
fn core() -> (
    Arc<AppCore>,
    mpsc::UnboundedReceiver<discard_core::ui_state::Command>,
) {
    let (commands, receiver) = mpsc::unbounded_channel();
    (
        Arc::new(AppCore::new(CoreInit {
            state: UiState::new(ChannelId::new(), vec![]),
            commands,
            project: "Project".into(),
            channel: "general".into(),
            agent: "Agent".into(),
            provider: "fake".into(),
        })),
        receiver,
    )
}
#[tokio::test]
async fn ten_thousand_deltas_emit_few_snapshots() {
    let (core, _commands) = core();
    let emit = Arc::new(RecordingEmit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    for _ in 0..10_000 {
        tx.send(text()).unwrap();
    }
    drop(tx);
    pump(core, rx, emit.clone(), Duration::from_millis(16)).await;
    let values = emit.0.lock().unwrap();
    assert!(values.len() < 500);
    assert_eq!(
        values.last().unwrap().live_blocks,
        vec![Block::Markdown {
            text: "x".repeat(10_000)
        }]
    );
}
#[tokio::test]
async fn send_streams_then_finishes() {
    let store = Store::open_in_memory().unwrap();
    let project = Project {
        id: ProjectId::new(),
        name: "p".into(),
        path: "/p".into(),
    };
    store.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    let policy = ApprovalPolicy::Ask;
    store
        .insert_agent(
            &agent_id,
            &AgentConfig {
                workspace_write: false, sandbox_autorun: false,
                reasoning_effort: None,
                name: "A".into(),
                avatar: None,
                provider: "fake".into(),
                model: None,
                system_prompt: String::new(),
                policy,
                plugin_write: false,
            },
        )
        .unwrap();
    let channel = Channel {
        id: ChannelId::new(),
        project_id: Some(project.id.clone()),
        name: "c".into(),
        agent_id: agent_id.clone(),
        workspace_mode: WorkspaceMode::MainCheckout,
    };
    store.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    store.create_session(&session, &channel.id).unwrap();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel),
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding, providers: vec![],
        store: Arc::new(Mutex::new(store)),
        agent_id,
        project_id: Some(project.id),
        memory: None,
        plugin_write: false,
        workspace_mode: WorkspaceMode::MainCheckout,
        workspace: None,
        observer: None,
        channel_name: "c".into(),
        provider: Arc::new(FakeProvider::new(vec![
            Step::working(),
            Step::text("Hi "),
            Step::text("there"),
            Step::idle(),
        ])),
        channel_id: channel.id.clone(),
        session_id: session,
        cwd: ".".into(),
        model: None,
        system_prompt: None,
        policy,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();
    let engine_task = tokio::spawn(engine.run(commands_rx, updates_tx));
    let core = Arc::new(AppCore::new(CoreInit {
        state: UiState::new(channel.id, vec![]),
        commands,
        project: "p".into(),
        channel: "c".into(),
        agent: "A".into(),
        provider: "fake".into(),
    }));
    let emit = Arc::new(RecordingEmit::default());
    let pump_task = tokio::spawn(pump(
        core.clone(),
        updates_rx,
        emit.clone(),
        Duration::from_millis(16),
    ));
    core.send("hello".into());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if core.history().len() == 2
                && emit
                    .0
                    .lock()
                    .unwrap()
                    .last()
                    .is_some_and(|s| !s.turn_running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let history = core.history();
    assert_eq!(
        history[0].blocks,
        vec![Block::Markdown {
            text: "hello".into()
        }]
    );
    assert_eq!(
        history[1].blocks,
        vec![Block::Markdown {
            text: "Hi there".into()
        }]
    );
    engine_task.abort();
    pump_task.abort();
}
#[tokio::test]
async fn get_snapshot_after_the_fact_returns_full_state() {
    let (core, _commands) = core();
    let emit = Arc::new(RecordingEmit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(text()).unwrap();
    drop(tx);
    pump(core.clone(), rx, emit.clone(), Duration::from_millis(16)).await;
    assert_eq!(core.snapshot(), *emit.0.lock().unwrap().last().unwrap());
}

#[test]
fn snapshot_includes_loaded_plugin_slots() {
    use discard_lua::keystore::StaticKeyStore;
    use discard_lua::ledger::Ledger;
    use discard_lua::{PluginLimits, PluginManager, PluginService};
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory(Arc::new(StaticKeyStore([5; 32]))).unwrap();
    let mut manager = PluginManager::new(PluginLimits::default());
    manager.load(discard_lua::bundled::default_ui()).unwrap();
    let plugins = Arc::new(
        PluginService::new(dir.path(), ledger, Arc::new(Mutex::new(manager)), false).unwrap(),
    );
    let (core, _commands) = core();
    core.set_plugins(plugins);
    let snapshot = core.snapshot();
    assert_eq!(snapshot.slots["channel_header"].plugin, "default-ui");
    assert_eq!(snapshot.slots["member_list"].plugin, "default-ui");
}

#[test]
fn switching_keeps_background_turns_history_and_commands_in_their_channel() {
    use discard_core::{driver::TurnOutcome, ui_state::Command};
    let dir = tempfile::tempdir().unwrap();
    let (core, mut first_commands) = core();
    core.configure_read_marker(dir.path(), "first");
    core.configure_targets("project".into(), dir.path(), "agent-a".into());
    let second = Channel { id: ChannelId("second".into()), project_id: Some(ProjectId("project".into())), name: "tests".into(), agent_id: AgentId("agent-b".into()), workspace_mode: WorkspaceMode::Worktree };
    let (commands, mut second_commands) = mpsc::unbounded_channel();
    core.add_channel(CoreInit { state: UiState::new(second.id.clone(), vec![]), commands, project: "Project".into(), channel: second.name.clone(), agent: "Tester".into(), provider: "codex".into() }, &second);
    core.send_to(Some("first"), "first task".into()).unwrap();
    assert_eq!(first_commands.try_recv().unwrap(), Command::Send("first task".into()));
    core.switch_channel("second").unwrap();
    assert_eq!(core.snapshot().agent, "Tester");
    assert_eq!(core.snapshot().provider, "codex");
    assert!(core.history().is_empty());
    assert!(core.any_running());
    assert!(!core.snapshot().turn_running);
    assert!(core.send_to(Some("first"), "stale task".into()).is_err());
    core.send_to(Some("second"), "second task".into()).unwrap();
    assert_eq!(second_commands.try_recv().unwrap(), Command::Send("second task".into()));
    core.apply_channel("first", text());
    core.apply_channel("first", Update::TurnFinished(TurnOutcome::Completed));
    assert_eq!(core.history().len(), 1, "background reply must not enter the current transcript");
    assert!(core.snapshot().channels.iter().find(|c| c.id == "first").unwrap().unread);
    assert!(core.snapshot().turn_running, "finishing another channel must not stop this channel");
    let before = core.history_snapshot().rev;
    core.switch_channel("first").unwrap();
    assert_eq!(core.history().len(), 2);
    assert!(core.history_snapshot().rev > before);
    assert_eq!(core.history_snapshot().channel_id, "first");
    core.cancel_in(Some("second"));
    assert!(first_commands.try_recv().is_err(), "a stale stop must not stop the selected channel");
    core.cancel_all();
    assert_eq!(second_commands.try_recv().unwrap(), Command::Cancel);
}

#[test]
fn archive_confirmation_locks_selection_to_its_channel() {
    let (core, _commands) = core();
    let (commands, _receiver) = mpsc::unbounded_channel();
    let channel = Channel { id: ChannelId("other".into()), project_id: Some(ProjectId("p".into())), name: "other".into(), agent_id: AgentId("a".into()), workspace_mode: WorkspaceMode::MainCheckout };
    core.add_channel(CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "p".into(), channel: "other".into(), agent: "a".into(), provider: "fake".into() }, &channel);
    assert!(core.archive_started());
    assert!(core.switch_channel("other").is_err());
    core.archive_finished(discard_app::desktop::plan::ArchiveStep::NeedsConfirm(vec!["uncommitted changes".into()]));
    assert!(core.switch_channel("other").is_err());
    core.cancel_archive();
    core.switch_channel("other").unwrap();
}

#[test]
fn background_completion_releases_the_exit_confirmation() {
    let (core, _receiver) = core();
    let (commands, _receiver2) = mpsc::unbounded_channel();
    let other = Channel { id: ChannelId("other".into()), project_id: Some(ProjectId("p".into())), name: "other".into(), agent_id: AgentId("a".into()), workspace_mode: WorkspaceMode::MainCheckout };
    core.add_channel(CoreInit { state: UiState::new(other.id.clone(), vec![]), commands, project: "p".into(), channel: "other".into(), agent: "a".into(), provider: "fake".into() }, &other);
    core.send("work".into());
    let first = core.active_channel_id();
    core.switch_channel("other").unwrap();
    core.ask(discard_app::desktop::plan::Confirm::Quit);
    assert!(core.can_switch().is_err());
    core.apply_channel(&first, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
    assert!(core.snapshot().banners.confirm.is_none());
    assert!(core.can_switch().is_ok());
    assert!(!core.archive_started_for(Some("stale-channel")));
}

#[tokio::test]
async fn live_pump_keeps_emitting_after_initial_project_engine_closes() {
    let (core, _) = core();
    let (sender, receiver) = mpsc::unbounded_channel();
    let emit = Arc::new(RecordingEmit::default());
    let task = tokio::spawn(discard_app::desktop::pump::pump_live(core.clone(), receiver, emit.clone(), Duration::from_millis(1)));
    drop(sender);
    tokio::task::yield_now().await;
    core.clear_project();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if emit.0.lock().unwrap().iter().any(|snapshot| snapshot.project_id.is_empty()) { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert!(!task.is_finished());
    task.abort();
}
