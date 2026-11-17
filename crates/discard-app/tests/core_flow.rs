use discard_app::desktop::{
    view::ViewId,
    core::{AppCore, CoreInit},
    dto::{HistoryDto, SnapshotDto, ThreadHistoryDto},
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
    fn snapshot(&self, _: &ViewId, value: &SnapshotDto) {
        self.0.lock().unwrap().push(value.clone());
    }
    fn history(&self, _: &ViewId, _: &HistoryDto) {}
}
/// Keeps what each view received.
#[derive(Default)]
struct ViewEmit {
    snapshots: Mutex<Vec<(ViewId, SnapshotDto)>>,
    histories: Mutex<Vec<(ViewId, HistoryDto)>>,
    thread_histories: Mutex<Vec<(ViewId, ThreadHistoryDto)>>,
}
impl Emit for ViewEmit {
    fn snapshot(&self, view: &ViewId, value: &SnapshotDto) {
        self.snapshots.lock().unwrap().push((view.clone(), value.clone()));
    }
    fn history(&self, view: &ViewId, value: &HistoryDto) {
        self.histories.lock().unwrap().push((view.clone(), value.clone()));
    }
    fn thread_history(&self, view: &ViewId, value: &ThreadHistoryDto) {
        self.thread_histories.lock().unwrap().push((view.clone(), value.clone()));
    }
}
impl ViewEmit {
    fn snapshots_of(&self, view: &ViewId) -> Vec<SnapshotDto> {
        self.snapshots.lock().unwrap().iter().filter(|(v, _)| v == view).map(|(_, s)| s.clone()).collect()
    }
    fn histories_of(&self, view: &ViewId) -> Vec<HistoryDto> {
        self.histories.lock().unwrap().iter().filter(|(v, _)| v == view).map(|(_, h)| h.clone()).collect()
    }
    fn thread_histories_of(&self, view: &ViewId) -> Vec<ThreadHistoryDto> {
        self.thread_histories.lock().unwrap().iter().filter(|(v, _)| v == view).map(|(_, h)| h.clone()).collect()
    }
}
/// A core whose main view selects "a" and whose project view selects "b". Both channels belong to project "p".
fn two_views() -> (Arc<AppCore>, ViewId) {
    let init = |id: &str| {
        let (commands, _) = mpsc::unbounded_channel();
        CoreInit { state: UiState::new(ChannelId(id.into()), vec![]), commands, project: "p".into(), channel: id.into(), agent: "A".into(), provider: "fake".into() }
    };
    let core = Arc::new(AppCore::new(init("a")));
    let b = Channel { id: ChannelId("b".into()), project_id: Some(ProjectId("p".into())), name: "b".into(), agent_id: AgentId("agent".into()), workspace_mode: WorkspaceMode::MainCheckout };
    core.add_channel(init("b"), &b);
    let other = ViewId::project(&ProjectId("p".into()));
    assert!(core.open_view(other.clone(), &Project { id: ProjectId("p".into()), name: "p".into(), path: "/p".into() }));
    core.switch_channel(&other, "b").unwrap();
    (core, other)
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
                workspace_write: false, sandbox_autorun: false, delegation: false,
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
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
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
    core.send(&ViewId::main(), "hello".into());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if core.history(&ViewId::main()).len() == 2
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
    let history = core.history(&ViewId::main());
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
    assert_eq!(core.snapshot(&ViewId::main()), *emit.0.lock().unwrap().last().unwrap());
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
    let snapshot = core.snapshot(&ViewId::main());
    assert_eq!(snapshot.slots["channel_header"].plugin, "default-ui");
    assert_eq!(snapshot.slots["member_list"].plugin, "default-ui");
}

#[test]
fn switching_keeps_background_turns_history_and_commands_in_their_channel() {
    use discard_core::{driver::TurnOutcome, ui_state::Command};
    let dir = tempfile::tempdir().unwrap();
    let (core, mut first_commands) = core();
    core.configure_read_marker(&ViewId::main(), dir.path(), "first");
    core.configure_targets(&ViewId::main(), "project".into(), dir.path(), "agent-a".into());
    let second = Channel { id: ChannelId("second".into()), project_id: Some(ProjectId("project".into())), name: "tests".into(), agent_id: AgentId("agent-b".into()), workspace_mode: WorkspaceMode::Worktree };
    let (commands, mut second_commands) = mpsc::unbounded_channel();
    core.add_channel(CoreInit { state: UiState::new(second.id.clone(), vec![]), commands, project: "Project".into(), channel: second.name.clone(), agent: "Tester".into(), provider: "codex".into() }, &second);
    core.send_to(Some("first"), "first task".into()).unwrap();
    assert_eq!(first_commands.try_recv().unwrap(), Command::Send("first task".into()));
    core.switch_channel(&ViewId::main(), "second").unwrap();
    assert_eq!(core.snapshot(&ViewId::main()).agent, "Tester");
    assert_eq!(core.snapshot(&ViewId::main()).provider, "codex");
    assert!(core.history(&ViewId::main()).is_empty());
    assert!(core.any_running());
    assert!(!core.snapshot(&ViewId::main()).turn_running);
    assert!(core.send_to(Some("first"), "stale task".into()).is_err());
    core.send_to(Some("second"), "second task".into()).unwrap();
    assert_eq!(second_commands.try_recv().unwrap(), Command::Send("second task".into()));
    core.apply_channel("first", text());
    core.apply_channel("first", Update::TurnFinished(TurnOutcome::Completed));
    assert_eq!(core.history(&ViewId::main()).len(), 1, "background reply must not enter the current transcript");
    assert!(core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == "first").unwrap().unread);
    assert!(core.snapshot(&ViewId::main()).turn_running, "finishing another channel must not stop this channel");
    let before = core.history_snapshot(&ViewId::main()).rev;
    core.switch_channel(&ViewId::main(), "first").unwrap();
    assert_eq!(core.history(&ViewId::main()).len(), 2);
    assert!(core.history_snapshot(&ViewId::main()).rev > before);
    assert_eq!(core.history_snapshot(&ViewId::main()).channel_id, "first");
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
    assert!(core.archive_started(&ViewId::main()));
    assert!(core.switch_channel(&ViewId::main(), "other").is_err());
    core.archive_finished(&ViewId::main(), discard_app::desktop::plan::ArchiveStep::NeedsConfirm(vec!["uncommitted changes".into()]));
    assert!(core.switch_channel(&ViewId::main(), "other").is_err());
    core.cancel_archive(&ViewId::main());
    core.switch_channel(&ViewId::main(), "other").unwrap();
}

#[test]
fn background_completion_releases_the_exit_confirmation() {
    let (core, _receiver) = core();
    let (commands, _receiver2) = mpsc::unbounded_channel();
    let other = Channel { id: ChannelId("other".into()), project_id: Some(ProjectId("p".into())), name: "other".into(), agent_id: AgentId("a".into()), workspace_mode: WorkspaceMode::MainCheckout };
    core.add_channel(CoreInit { state: UiState::new(other.id.clone(), vec![]), commands, project: "p".into(), channel: "other".into(), agent: "a".into(), provider: "fake".into() }, &other);
    core.send(&ViewId::main(), "work".into());
    let first = core.active_channel_id(&ViewId::main());
    core.switch_channel(&ViewId::main(), "other").unwrap();
    core.ask(discard_app::desktop::plan::Confirm::Quit);
    assert!(core.can_switch(&ViewId::main()).is_err());
    core.apply_channel(&first, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
    assert!(core.snapshot(&ViewId::main()).banners.confirm.is_none());
    assert!(core.can_switch(&ViewId::main()).is_ok());
    assert!(!core.archive_started_for(&ViewId::main(), Some("stale-channel")));
}

#[tokio::test]
async fn each_view_receives_its_own_snapshot_and_history() {
    let (core, other) = two_views();
    let emit = Arc::new(ViewEmit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(text()).unwrap();
    drop(tx);
    pump(core.clone(), rx, emit.clone(), Duration::from_millis(16)).await;
    let main = ViewId::main();
    let (main_snapshots, other_snapshots) = (emit.snapshots_of(&main), emit.snapshots_of(&other));
    assert!(!main_snapshots.is_empty() && !other_snapshots.is_empty(), "every view gets a snapshot");
    assert!(main_snapshots.iter().all(|s| s.channel_id == "a"));
    assert!(other_snapshots.iter().all(|s| s.channel_id == "b"));
    for view in [&main, &other] {
        let histories = emit.histories_of(view);
        assert!(!histories.is_empty(), "{view:?} gets a history");
        assert!(histories.iter().all(|h| h.channel_id == core.active_channel_id(view)), "{view:?}");
    }
    // The streamed text belongs to channel "a". The view of "b" does not show it.
    assert_eq!(main_snapshots.last().unwrap().live_blocks, vec![Block::Markdown { text: "x".into() }]);
    assert!(other_snapshots.last().unwrap().live_blocks.is_empty());
}
#[tokio::test]
async fn the_thread_history_goes_only_to_the_view_that_has_the_thread_open() {
    let (core, other) = two_views();
    // Both views show "a". Only the main view opens the thread of "a".
    core.switch_channel(&other, "a").unwrap();
    let (commands, _) = mpsc::unbounded_channel();
    let thread = Channel { id: ChannelId("t".into()), project_id: Some(ProjectId("p".into())), name: "t".into(), agent_id: AgentId("agent".into()), workspace_mode: WorkspaceMode::MainCheckout };
    let info = discard_core::model::ThreadInfo {
        channel_id: thread.id.clone(), parent_channel_id: ChannelId("a".into()), anchor_message_id: None, kind: discard_core::model::ThreadKind::SubAgent,
        followed: false, name: "t".into(), state: None, agent_id: AgentId("agent".into()), agent_name: "A".into(),
    };
    core.add_thread(CoreInit { state: UiState::new(thread.id.clone(), vec![]), commands, project: "p".into(), channel: "t".into(), agent: "A".into(), provider: "fake".into() }, &thread, &info);
    core.open_thread(&ViewId::main(), "t").unwrap();
    let emit = Arc::new(ViewEmit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(text()).unwrap();
    drop(tx);
    pump(core.clone(), rx, emit.clone(), Duration::from_millis(16)).await;
    let opened = emit.thread_histories_of(&ViewId::main());
    assert!(!opened.is_empty() && opened.iter().all(|h| h.channel_id == "t"), "the main view gets the transcript of its open thread");
    assert!(emit.thread_histories_of(&other).is_empty(), "the other view has no thread open and gets no thread transcript");
    assert!(emit.snapshots_of(&other).iter().all(|s| s.open_thread.is_none()));
}
#[tokio::test]
async fn a_dropped_view_receives_nothing_more_and_a_new_view_receives_from_its_start() {
    let (core, other) = two_views();
    let emit = Arc::new(ViewEmit::default());
    let (sender, receiver) = mpsc::unbounded_channel();
    let task = tokio::spawn(discard_app::desktop::pump::pump_live(core.clone(), receiver, emit.clone(), Duration::from_millis(1)));
    let wait = |done: Box<dyn Fn() -> bool>| async move {
        tokio::time::timeout(Duration::from_secs(2), async { while !done() { tokio::time::sleep(Duration::from_millis(5)).await; } }).await.unwrap();
    };
    sender.send(text()).unwrap();
    let probe = emit.clone();
    let main = ViewId::main();
    let (watched, watched_main) = (other.clone(), main.clone());
    wait(Box::new(move || !probe.snapshots_of(&watched).is_empty() && !probe.snapshots_of(&watched_main).is_empty())).await;
    core.drop_view(&other);
    // Let a pump pass that follows the drop finish, then count.
    core.notice(&main, "after the drop".into());
    let probe = emit.clone();
    wait(Box::new(move || probe.snapshots_of(&ViewId::main()).last().is_some_and(|s| s.notice.as_deref() == Some("after the drop")))).await;
    let count = emit.snapshots_of(&other).len();
    let histories = emit.histories_of(&other).len();
    core.notice(&main, "again".into());
    let probe = emit.clone();
    wait(Box::new(move || probe.snapshots_of(&ViewId::main()).last().is_some_and(|s| s.notice.as_deref() == Some("again")))).await;
    assert_eq!(emit.snapshots_of(&other).len(), count, "a dropped view gets no more snapshots");
    // The same label opens again and gets its own channel from the start.
    assert!(core.open_view(other.clone(), &Project { id: ProjectId("p".into()), name: "p".into(), path: "/p".into() }));
    core.switch_channel(&other, "b").unwrap();
    let probe = emit.clone();
    let reopened = other.clone();
    wait(Box::new(move || probe.snapshots_of(&reopened).len() > count)).await;
    assert_eq!(emit.snapshots_of(&other).last().unwrap().channel_id, "b");
    assert!(emit.histories_of(&other).len() > histories, "the reopened view gets a history of its own");
    assert!(!task.is_finished());
    drop(sender);
    task.abort();
}
#[tokio::test]
async fn live_pump_keeps_emitting_after_initial_project_engine_closes() {
    let (core, _) = core();
    let (sender, receiver) = mpsc::unbounded_channel();
    let emit = Arc::new(RecordingEmit::default());
    let task = tokio::spawn(discard_app::desktop::pump::pump_live(core.clone(), receiver, emit.clone(), Duration::from_millis(1)));
    drop(sender);
    tokio::task::yield_now().await;
    core.clear_project(&ViewId::main());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if emit.0.lock().unwrap().iter().any(|snapshot| snapshot.project_id.is_empty()) { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert!(!task.is_finished());
    task.abort();
}
