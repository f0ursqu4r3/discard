//! Engine tests: UI commands in, UI updates out, history persisted. Uses the fake provider.

use discard_core::agent::AgentConfig;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::{Engine, MEMORY_TOOL_PREFIX, MemoryService, PreparedMemory, PreparedWorkspace, WorkspaceService};
use discard_core::event::{ApprovalId, EventKind, SessionId};
use discard_core::observer::EventObserver;
use discard_core::model::{AgentId, Block, Channel, ChannelId, Project, ProjectId, Role, WorkspaceMode};
use discard_core::provider::SetupStatus;
use discard_core::store::Store;
use discard_core::ui_state::{Command, Update};
use discard_providers::fake::{FakeProvider, Probe, Step};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct Rig {
    store: Arc<Mutex<Store>>,
    channel: ChannelId,
    session: SessionId,
    probe: Probe,
    commands: mpsc::UnboundedSender<Command>,
    updates: mpsc::UnboundedReceiver<Update>,
    task: tokio::task::JoinHandle<()>,
}

fn start(provider: FakeProvider, policy: ApprovalPolicy) -> Rig {
    start_with(provider, policy, None)
}

fn start_with(provider: FakeProvider, policy: ApprovalPolicy, memory: Option<Arc<dyn MemoryService>>) -> Rig {
    start_full(provider, policy, memory, None)
}

fn start_full(
    provider: FakeProvider,
    policy: ApprovalPolicy,
    memory: Option<Arc<dyn MemoryService>>,
    observer: Option<Arc<dyn EventObserver>>,
) -> Rig {
    let store = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
    store.insert_project(&project).unwrap();
    let (agent_id, project_id) = (AgentId::new(), project.id.clone());
    let agent = agent_id.clone();
    store
        .insert_agent(
            &agent,
            &AgentConfig {
        reasoning_effort: None, name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy, plugin_write: false },
        )
        .unwrap();
    let channel = Channel {
        id: ChannelId::new(),
        project_id: Some(project.id),
        name: "c".into(),
        agent_id: agent,
        workspace_mode: WorkspaceMode::MainCheckout,
    };
    store.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    store.create_session(&session, &channel.id).unwrap();
    let store = Arc::new(Mutex::new(store));
    let probe = provider.probe.clone();
    let engine = Engine {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding, providers: vec![],
        store: Arc::clone(&store),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        memory,
        plugin_write: false,
        workspace_mode: discard_core::model::WorkspaceMode::MainCheckout,
        workspace: None,
        observer,
        channel_name: "c".into(),
        provider: Arc::new(provider),
        channel_id: channel.id.clone(),
        session_id: session.clone(),
        cwd: PathBuf::from("."),
        model: None,
        system_prompt: None,
        policy,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, updates) = mpsc::unbounded_channel();
    let task = tokio::spawn(engine.run(commands_rx, updates_tx));
    Rig { store, channel: channel.id, session, probe, commands, updates, task }
}

impl Rig {
    async fn next(&mut self) -> Update {
        tokio::time::timeout(Duration::from_secs(5), self.updates.recv())
            .await
            .expect("an update within 5 seconds")
            .expect("the engine is still running")
    }

    /// Collect updates up to and including the next `TurnFinished`.
    async fn until_finished(&mut self) -> Vec<Update> {
        let mut out = Vec::new();
        loop {
            let u = self.next().await;
            let done = matches!(u, Update::TurnFinished(_));
            out.push(u);
            if done {
                return out;
            }
        }
    }

    fn messages(&self) -> Vec<discard_core::model::Message> {
        self.store.lock().unwrap().messages_for_channel(&self.channel).unwrap()
    }
}

fn bash() -> Step {
    Step::Approval { tool: "Bash".into(), input: json!({"command": "touch x"}) }
}

#[tokio::test]
async fn a_turn_streams_updates_and_persists_both_messages() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), Step::text("Hi "), Step::text("there"), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("hello".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::Event(_))).count(), 4);
    let msgs = rig.messages();
    assert_eq!(msgs.len(), 2);
    assert_eq!((msgs[0].role, &msgs[0].blocks[..]), (Role::User, &[Block::Markdown { text: "hello".into() }][..]));
    assert_eq!((msgs[1].role, &msgs[1].blocks[..]), (Role::Agent, &[Block::Markdown { text: "Hi there".into() }][..]));
}

#[tokio::test]
async fn attachments_are_persisted_and_sent_without_requiring_message_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "The release date is Friday.").unwrap();
    let attachments = discard_core::attachments::import_files(dir.path(), &[path]).unwrap();
    let mut rig = start(FakeProvider::new(vec![Step::text("Read it"), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::SendAttachments { text: String::new(), attachments: attachments.clone() }).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert_eq!(rig.messages()[0].blocks, discard_core::attachments::blocks("", &attachments));
    assert!(rig.probe.sent.lock().unwrap()[0].contains("The release date is Friday."));
}

#[tokio::test]
async fn builder_turns_reload_saved_reasoning_and_can_clear_it() {
    let mut rig = start(FakeProvider::new(vec![Step::text("done"), Step::idle()]), ApprovalPolicy::Ask);
    for effort in [Some("high"), Some("low"), None] {
        {
            let store = rig.store.lock().unwrap();
            let agent = store.load_channel(&rig.channel).unwrap().agent_id;
            let mut config = store.load_agent(&agent).unwrap();
            config.model = Some("model".into());
            config.reasoning_effort = effort.map(str::to_string);
            store.update_agent(&agent, &config).unwrap();
        }
        rig.commands.send(Command::Send("hello".into())).unwrap();
        rig.until_finished().await;
        assert_eq!(rig.probe.specs.lock().unwrap().last().unwrap().reasoning_effort.as_deref(), effort);
    }
}

#[tokio::test]
async fn approvals_flow_through_commands_and_report_the_decision() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), bash(), Step::text("done"), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    let mut seen = Vec::new();
    let approval_id = loop {
        let u = rig.next().await;
        seen.push(u.clone());
        if let Update::Event(e) = u
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    rig.commands.send(Command::Decide { approval_id: approval_id.clone(), decision: Decision::Approve }).unwrap();
    seen.extend(rig.until_finished().await);
    assert!(seen.contains(&Update::ApprovalState { approval_id: approval_id.clone(), state: ApprovalState::Decided(Decision::Approve) }));
    assert!(matches!(seen.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert_eq!(rig.probe.responses.lock().unwrap().as_slice(), [(approval_id, Decision::Approve)]);
}

#[tokio::test]
async fn a_second_decision_for_the_same_approval_is_not_reported_again() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), bash(), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    for d in [Decision::Approve, Decision::Deny] {
        rig.commands.send(Command::Decide { approval_id: approval_id.clone(), decision: d }).unwrap();
    }
    let updates = rig.until_finished().await;
    let states = updates.iter().filter(|u| matches!(u, Update::ApprovalState { .. })).count();
    assert_eq!(states, 1);
    assert_eq!(rig.probe.responses.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancel_ends_the_turn_as_cancelled() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), Step::Hang]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    loop {
        if matches!(rig.next().await, Update::Event(_)) {
            break;
        }
    }
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))));
    assert!(rig.probe.cancelled.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn sending_during_a_turn_is_refused_without_disturbing_it() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), bash(), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("first".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    rig.commands.send(Command::Send("second".into())).unwrap();
    let busy = rig.next().await;
    assert!(matches!(&busy, Update::Error(m) if m.contains("already running")), "{busy:?}");
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Approve }).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert_eq!(rig.messages().iter().filter(|m| m.role == Role::User).count(), 1, "the refused message is not stored");
}

#[tokio::test]
async fn missing_login_tells_the_user_the_command_to_run() {
    let provider = FakeProvider::new(vec![Step::idle()])
        .with_setup(SetupStatus::NotLoggedIn { login_command: "claude auth login".into() });
    let mut rig = start(provider, ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("hello".into())).unwrap();
    let Update::SetupNeeded(text) = rig.next().await else { panic!("expected SetupNeeded") };
    assert!(text.contains("claude auth login"), "{text}");
    assert!(rig.probe.started_with.lock().unwrap().is_empty(), "no session may start");
}

#[tokio::test]
async fn a_policy_the_provider_cannot_enforce_fails_closed() {
    let provider = FakeProvider::with_capabilities(vec![Step::idle()], false);
    let mut rig = start(provider, ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("hello".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(m) if m.contains("cannot enforce"))), "{updates:?}");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    assert!(rig.probe.started_with.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_resume_handle_is_saved_and_used_for_the_next_turn() {
    let mut rig = start(
        FakeProvider::new(vec![Step::working(), Step::idle()]).with_resume_handle("sess-1"),
        ApprovalPolicy::Ask,
    );
    for text in ["one", "two"] {
        rig.commands.send(Command::Send(text.into())).unwrap();
        rig.until_finished().await;
    }
    assert_eq!(rig.store.lock().unwrap().resume_handle(&rig.session).unwrap().as_deref(), Some("sess-1"));
    assert_eq!(rig.probe.started_with.lock().unwrap().as_slice(), [None, Some("sess-1".to_string())]);
}

#[tokio::test]
async fn the_engine_stops_when_the_ui_goes_away() {
    let rig = start(FakeProvider::new(vec![]), ApprovalPolicy::Ask);
    drop(rig.commands);
    tokio::time::timeout(Duration::from_secs(2), rig.task).await.expect("engine exits").unwrap();
}

#[allow(dead_code)]
fn _uses(_: ApprovalId) {}

// ---- memory integration ----

struct FakeMemory {
    fail: bool,
    prepared: Mutex<Vec<(AgentId, Option<ProjectId>, Option<String>)>>,
    finished: Mutex<Vec<String>>,
}

impl FakeMemory {
    fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self { fail, prepared: Mutex::default(), finished: Mutex::default() })
    }
}

impl MemoryService for FakeMemory {
    fn prepare(&self, agent: &AgentId, project: Option<&ProjectId>, _source: &ChannelId, base: Option<&str>, _plugin_write: bool) -> Result<PreparedMemory, String> {
        self.prepared.lock().unwrap().push((agent.clone(), project.cloned(), base.map(str::to_string)));
        if self.fail {
            return Err("memory is unavailable".into());
        }
        Ok(PreparedMemory {
            system_prompt: "PROMPT WITH CORE BLOCKS".into(),
            mcp_config: PathBuf::from("/run/discard/session.json"),
            token: "tok-1".into(),
        })
    }
    fn finish(&self, token: &str) {
        self.finished.lock().unwrap().push(token.to_string());
    }
}

fn memory_tool() -> Step {
    Step::Approval { tool: format!("{MEMORY_TOOL_PREFIX}memory_append"), input: json!({"scope": "agent"}) }
}

#[tokio::test]
async fn a_session_gets_the_memory_prompt_and_mcp_config() {
    let mem = FakeMemory::new(false);
    let mut rig = start_with(FakeProvider::new(vec![Step::working(), Step::idle()]), ApprovalPolicy::Ask, Some(mem.clone()));
    rig.commands.send(Command::Send("hi".into())).unwrap();
    rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap().clone();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].system_prompt.as_deref(), Some("PROMPT WITH CORE BLOCKS"));
    assert_eq!(specs[0].mcp_config, Some(PathBuf::from("/run/discard/session.json")));
    assert_eq!(mem.prepared.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_session_token_is_revoked_after_every_kind_of_turn_end() {
    for script in [vec![Step::working(), Step::idle()], vec![Step::working(), Step::Crash]] {
        let mem = FakeMemory::new(false);
        let mut rig = start_with(FakeProvider::new(script), ApprovalPolicy::Ask, Some(mem.clone()));
        rig.commands.send(Command::Send("hi".into())).unwrap();
        rig.until_finished().await;
        assert_eq!(mem.finished.lock().unwrap().as_slice(), ["tok-1"]);
    }
}

#[tokio::test]
async fn the_token_is_revoked_after_a_cancelled_turn() {
    let mem = FakeMemory::new(false);
    let mut rig = start_with(FakeProvider::new(vec![Step::working(), Step::Hang]), ApprovalPolicy::Ask, Some(mem.clone()));
    rig.commands.send(Command::Send("hi".into())).unwrap();
    loop {
        if matches!(rig.next().await, Update::Event(_)) {
            break;
        }
    }
    rig.commands.send(Command::Cancel).unwrap();
    rig.until_finished().await;
    assert_eq!(mem.finished.lock().unwrap().as_slice(), ["tok-1"]);
}

#[tokio::test]
async fn memory_tool_calls_are_approved_by_policy_and_audited_as_such() {
    let mem = FakeMemory::new(false);
    let mut rig = start_with(FakeProvider::new(vec![Step::working(), memory_tool(), Step::idle()]), ApprovalPolicy::Ask, Some(mem));
    rig.commands.send(Command::Send("remember".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))), "no user action was needed");
    let responses = rig.probe.responses.lock().unwrap().clone();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].1, Decision::Approve);
    let approval = rig.store.lock().unwrap().load_approval(&responses[0].0).unwrap();
    assert_eq!(approval.state, ApprovalState::Decided(Decision::Approve));
    assert_eq!(approval.audit[0].decider, discard_core::approval::Decider::Policy("builtin-discard-tools".into()));
}

#[tokio::test]
async fn other_tools_still_wait_for_the_user_even_with_memory_on() {
    let mem = FakeMemory::new(false);
    let mut rig = start_with(FakeProvider::new(vec![Step::working(), bash(), Step::idle()]), ApprovalPolicy::Ask, Some(mem));
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.probe.responses.lock().unwrap().is_empty(), "a shell command must not be auto-approved");
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
}

#[tokio::test]
async fn a_lookalike_tool_from_another_server_is_not_auto_approved() {
    let mem = FakeMemory::new(false);
    let lookalike = Step::Approval { tool: "mcp__other-server__memory_append".into(), input: json!({}) };
    let mut rig = start_with(FakeProvider::new(vec![Step::working(), lookalike, Step::idle()]), ApprovalPolicy::Ask, Some(mem));
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
}

#[tokio::test]
async fn without_a_memory_service_even_a_memory_named_tool_waits_for_the_user() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), memory_tool(), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
}

#[tokio::test]
async fn if_memory_cannot_be_prepared_the_turn_fails_visibly_and_no_session_starts() {
    let mem = FakeMemory::new(true);
    let mut rig = start_with(FakeProvider::new(vec![Step::idle()]), ApprovalPolicy::Ask, Some(mem.clone()));
    rig.commands.send(Command::Send("hi".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(m) if m.contains("memory is unavailable"))), "{updates:?}");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    assert!(rig.probe.started_with.lock().unwrap().is_empty());
    assert!(mem.finished.lock().unwrap().is_empty(), "nothing was issued, so nothing is revoked");
}

#[tokio::test]
async fn the_agents_own_system_prompt_is_passed_to_the_memory_service() {
    let mem = FakeMemory::new(false);
    let mut rig = start_with(FakeProvider::new(vec![Step::idle()]), ApprovalPolicy::Ask, Some(mem.clone()));
    // `start_with` builds the engine with system_prompt None, so the base is None here.
    rig.commands.send(Command::Send("hi".into())).unwrap();
    rig.until_finished().await;
    assert_eq!(mem.prepared.lock().unwrap()[0].2, None);
}

// ---- workspace (worktree) integration ----

struct FakeWorkspace {
    fail: Option<String>,
    notes: Vec<String>,
    calls: Mutex<Vec<(PathBuf, ChannelId, String)>>,
}

impl FakeWorkspace {
    fn ok(notes: &[&str]) -> Arc<Self> {
        Arc::new(Self { fail: None, notes: notes.iter().map(|s| s.to_string()).collect(), calls: Mutex::default() })
    }
    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self { fail: Some(message.into()), notes: vec![], calls: Mutex::default() })
    }
}

impl WorkspaceService for FakeWorkspace {
    fn prepare(&self, project_dir: &std::path::Path, channel: &ChannelId, name: &str) -> Result<PreparedWorkspace, String> {
        self.calls.lock().unwrap().push((project_dir.to_path_buf(), channel.clone(), name.to_string()));
        match &self.fail {
            Some(m) => Err(m.clone()),
            None => Ok(PreparedWorkspace { cwd: PathBuf::from("/worktrees/chan-1"), notes: self.notes.clone() }),
        }
    }
}

fn start_workspace(
    provider: FakeProvider,
    mode: discard_core::model::WorkspaceMode,
    workspace: Option<Arc<dyn WorkspaceService>>,
    memory: Option<Arc<dyn MemoryService>>,
) -> Rig {
    let store = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
    store.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    store.insert_agent(&agent_id, &AgentConfig { reasoning_effort: None, name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent_id.clone(), workspace_mode: mode };
    store.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    store.create_session(&session, &channel.id).unwrap();
    let store = Arc::new(Mutex::new(store));
    let probe = provider.probe.clone();
    let engine = Engine {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding, providers: vec![],
        store: Arc::clone(&store),
        agent_id,
        project_id: Some(project.id),
        memory,
        plugin_write: false,
        workspace_mode: mode,
        workspace,
        observer: None,
        channel_name: "general".into(),
        provider: Arc::new(provider),
        channel_id: channel.id.clone(),
        session_id: session.clone(),
        cwd: PathBuf::from("/projects/app"),
        model: None,
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, updates) = mpsc::unbounded_channel();
    let task = tokio::spawn(engine.run(commands_rx, updates_tx));
    Rig { store, channel: channel.id, session, probe, commands, updates, task }
}

#[tokio::test]
async fn a_worktree_channel_gets_its_workspace_before_the_session_and_runs_there() {
    use discard_core::model::WorkspaceMode::Worktree;
    let ws = FakeWorkspace::ok(&[]);
    let mut rig = start_workspace(FakeProvider::new(vec![Step::working(), Step::idle()]), Worktree, Some(ws.clone()), None);
    rig.commands.send(Command::Send("hi".into())).unwrap();
    rig.until_finished().await;
    let calls = ws.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, PathBuf::from("/projects/app"));
    assert_eq!((calls[0].1.clone(), calls[0].2.as_str()), (rig.channel.clone(), "general"));
    let specs = rig.probe.specs.lock().unwrap().clone();
    assert_eq!(specs[0].cwd, PathBuf::from("/worktrees/chan-1"), "the session runs in the worktree");
}

#[tokio::test]
async fn a_main_checkout_channel_never_touches_the_workspace_service() {
    use discard_core::model::WorkspaceMode::MainCheckout;
    let ws = FakeWorkspace::ok(&[]);
    let mut rig = start_workspace(FakeProvider::new(vec![Step::working(), Step::idle()]), MainCheckout, Some(ws.clone()), None);
    rig.commands.send(Command::Send("hi".into())).unwrap();
    rig.until_finished().await;
    assert!(ws.calls.lock().unwrap().is_empty());
    assert_eq!(rig.probe.specs.lock().unwrap()[0].cwd, PathBuf::from("/projects/app"));
}

#[tokio::test]
async fn a_worktree_channel_with_no_workspace_service_fails_closed_instead_of_using_the_main_checkout() {
    use discard_core::model::WorkspaceMode::Worktree;
    let mut rig = start_workspace(FakeProvider::new(vec![Step::idle()]), Worktree, None, None);
    rig.commands.send(Command::Send("hi".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(m) if m.contains("worktree"))), "{updates:?}");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    assert!(rig.probe.started_with.lock().unwrap().is_empty(), "no session may start in the wrong directory");
}

#[tokio::test]
async fn if_the_workspace_cannot_be_created_the_turn_does_not_start() {
    use discard_core::model::WorkspaceMode::Worktree;
    let mem = FakeMemory::new(false);
    let mut rig = start_workspace(FakeProvider::new(vec![Step::idle()]), Worktree, Some(FakeWorkspace::failing("the repository has no commits yet")), Some(mem.clone()));
    rig.commands.send(Command::Send("hi".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(m) if m.contains("no commits"))), "{updates:?}");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    assert!(rig.probe.started_with.lock().unwrap().is_empty());
    assert!(mem.prepared.lock().unwrap().is_empty(), "no memory token may be issued for a turn that cannot start");
}

#[tokio::test]
async fn workspace_notes_reach_the_user_as_notices_before_the_reply() {
    use discard_core::model::WorkspaceMode::Worktree;
    let ws = FakeWorkspace::ok(&["Uncommitted changes were not copied: a.txt"]);
    let mut rig = start_workspace(FakeProvider::new(vec![Step::working(), Step::idle()]), Worktree, Some(ws), None);
    rig.commands.send(Command::Send("hi".into())).unwrap();
    let updates = rig.until_finished().await;
    let notice_at = updates.iter().position(|u| matches!(u, Update::Notice(m) if m.contains("not copied"))).expect("a notice");
    let first_event = updates.iter().position(|u| matches!(u, Update::Event(_))).unwrap();
    assert!(notice_at < first_event);
}

#[tokio::test]
async fn the_workspace_is_requested_again_on_every_turn() {
    use discard_core::model::WorkspaceMode::Worktree;
    let ws = FakeWorkspace::ok(&[]);
    let mut rig = start_workspace(FakeProvider::new(vec![Step::working(), Step::idle()]), Worktree, Some(ws.clone()), None);
    for text in ["one", "two"] {
        rig.commands.send(Command::Send(text.into())).unwrap();
        rig.until_finished().await;
    }
    assert_eq!(ws.calls.lock().unwrap().len(), 2, "prepare is idempotent, so the engine asks every turn");
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);

impl EventObserver for Recorder {
    fn observe(&self, event: &str, payload: &serde_json::Value) {
        self.0.lock().unwrap().push((event.to_string(), payload.clone()));
    }
}

impl Recorder {
    fn names(&self) -> Vec<String> {
        self.0.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }
}

#[tokio::test]
async fn an_observer_sees_the_message_status_changes_and_the_finished_turn_in_order() {
    let rec = Arc::new(Recorder::default());
    let mut rig = start_full(FakeProvider::new(vec![Step::working(), Step::text("hi"), Step::idle()]), ApprovalPolicy::Ask, None, Some(rec.clone()));
    rig.commands.send(Command::Send("hello".into())).unwrap();
    rig.until_finished().await;
    assert_eq!(rec.names(), ["message_sent", "agent_status", "agent_status", "turn_finished"]);
    let seen = rec.0.lock().unwrap().clone();
    assert_eq!(seen[0].1, json!({ "channel_id": rig.channel.0, "text": "hello" }));
    assert_eq!(seen[1].1, json!({ "status": "working" }));
    assert_eq!(seen[3].1, json!({ "outcome": "completed" }));
}

#[tokio::test]
async fn an_observer_sees_the_approval_request_without_its_input() {
    let rec = Arc::new(Recorder::default());
    let mut rig = start_full(FakeProvider::new(vec![Step::working(), bash(), Step::idle()]), ApprovalPolicy::Ask, None, Some(rec.clone()));
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
    let seen = rec.0.lock().unwrap().clone();
    let (_, payload) = seen.iter().find(|(n, _)| n == "approval_requested").expect("an approval_requested event");
    assert_eq!(payload["tool"], "Bash");
    assert!(!payload.to_string().contains("touch x"), "the tool input must not reach plugins: {payload}");
}

#[tokio::test]
async fn a_turn_whose_stream_ends_early_is_reported_as_failed() {
    let rec = Arc::new(Recorder::default());
    let mut rig = start_full(FakeProvider::new(vec![]), ApprovalPolicy::Ask, None, Some(rec.clone()));
    rig.commands.send(Command::Send("x".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    let last = rec.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!((last.0.as_str(), last.1), ("turn_finished", json!({ "outcome": "failed" })));
}

#[tokio::test]
async fn tool_events_are_not_sent_when_the_provider_cannot_show_tool_calls() {
    let rec = Arc::new(Recorder::default());
    let provider = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]).with_tool_call_visibility(false);
    let mut rig = start_full(provider, ApprovalPolicy::Ask, None, Some(rec.clone()));
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    rig.commands.send(Command::Decide { approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
    let names = rec.names();
    assert!(!names.iter().any(|n| n == "approval_requested" || n == "tool_call" || n == "tool_result"), "{names:?}");
    assert!(names.iter().any(|n| n == "agent_status"), "non-tool events still flow: {names:?}");
}
