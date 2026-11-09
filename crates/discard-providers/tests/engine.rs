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
        workspace_write: false,
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
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel),
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
    assert!(matches!(rig.next().await, Update::Responding(_)));
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

type PreparedCalls = Vec<(AgentId, Option<ProjectId>, Option<String>)>;

struct FakeMemory {
    fail: bool,
    prepared: Mutex<PreparedCalls>,
    finished: Mutex<Vec<String>>,
}

impl FakeMemory {
    fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self { fail, prepared: Mutex::default(), finished: Mutex::default() })
    }
}

impl MemoryService for FakeMemory {
    fn prepare_workspace_turn(&self, agent: &AgentId, project: Option<&ProjectId>, source: &ChannelId, base: Option<&str>, _grant: &discard_core::workspace_session::WorkspaceGrant, _query: &str) -> Result<PreparedMemory, String> {
        let mut prepared = self.prepare(agent, project, source, base, false)?;
        prepared.token = "workspace-token".into(); prepared.mcp_config = PathBuf::from("/run/discard/workspace.json"); Ok(prepared)
    }
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
    store.insert_agent(&agent_id, &AgentConfig { workspace_write: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent_id.clone(), workspace_mode: mode };
    store.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    store.create_session(&session, &channel.id).unwrap();
    let store = Arc::new(Mutex::new(store));
    let probe = provider.probe.clone();
    let engine = Engine {
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel),
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

fn workspace_engine(provider: FakeProvider, eligible: bool) -> (tempfile::TempDir, Engine, Probe) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| { let out = std::process::Command::new("git").current_dir(&repo).args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"]).args(args).output().unwrap(); assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr)); };
    git(&["init", "-q", "-b", "main"]);
    git(&["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "initial"]);
    let root = dir.path().join("worktree"); git(&["worktree", "add", "-b", "feature", root.to_str().unwrap()]);
    let store = Store::open_in_memory().unwrap();
    let mut config = AgentConfig::from_toml("name='A'\nprovider='fake'\nmodel='test'\npolicy='ask'\n").unwrap(); config.workspace_write = true;
    let agent = store.create_agent(&config).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let channel = Channel { id: ChannelId::new(), name: "feature".into(), project_id: Some(project.id.clone()), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_channel(&channel, eligible.then_some("feature"), eligible.then_some(&discard_core::store::WorktreeRecord { channel_id: channel.id.clone(), path: root.to_string_lossy().into(), repo: repo.to_string_lossy().into(), branch: "feature".into(), base_commit: String::new(), owned: false })).unwrap();
    let session = SessionId::new(); store.create_session(&session, &channel.id).unwrap();
    let probe = provider.probe.clone();
    let engine = Engine { workspace_locks: Arc::default(), mode: discard_core::provider::SessionMode::Conversation, providers: vec![], store: Arc::new(Mutex::new(store)), agent_id: agent, project_id: Some(project.id), memory: Some(FakeMemory::new(false)), plugin_write: false, workspace_mode: WorkspaceMode::Worktree, workspace: None, observer: None, channel_name: channel.name, provider: Arc::new(provider), channel_id: channel.id, session_id: session, cwd: repo, model: Some("test".into()), reasoning_effort: None, system_prompt: None, policy: ApprovalPolicy::Ask, write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel) };
    (dir, engine, probe)
}

fn launch_workspace(engine: Engine, probe: Probe) -> Rig {
    let (commands, rx) = mpsc::unbounded_channel(); let (tx, updates) = mpsc::unbounded_channel();
    let store = engine.store.clone(); let channel = engine.channel_id.clone(); let session = engine.session_id.clone();
    let task = tokio::spawn(engine.run(rx, tx));
    Rig { store, channel, session, probe, commands, updates, task }
}

#[tokio::test]
async fn eligible_branch_channel_starts_a_workspace_session() {
    let (dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::text("done"), Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); let spec = &specs[0];
    assert_eq!(spec.mode, discard_core::provider::SessionMode::Workspace);
    assert_eq!(spec.cwd, dir.path().join("worktree").canonicalize().unwrap());
    assert_eq!(spec.resume_handle, None);
    assert!(spec.system_prompt.as_ref().unwrap().contains("Commit through worktree_commit"));
    rig.task.abort();
}

#[tokio::test]
async fn ineligible_channel_states_the_reason() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::idle()]), false);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); let spec = &specs[0];
    assert_eq!(spec.mode, discard_core::provider::SessionMode::Conversation);
    assert!(spec.system_prompt.as_ref().unwrap().contains("File edits are unavailable in this response: this is not a project branch channel."));
    rig.task.abort();
}

#[tokio::test]
async fn in_root_edit_is_auto_approved_in_workspace_sessions() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Approval { tool: "Edit".into(), input: json!({"file_path":"new.txt"}) }, Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    let id = updates.iter().find_map(|u| match u { Update::Event(e) => match &e.kind { EventKind::ApprovalRequest { approval_id, .. } => Some(approval_id), _ => None }, _ => None }).unwrap();
    let a = rig.store.lock().unwrap().load_approval(id).unwrap();
    assert_eq!(a.audit[0].decider, discard_core::approval::Decider::Policy("workspace_edit".into()));
    rig.task.abort();
}

#[tokio::test]
async fn out_of_root_edit_waits_for_the_user() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Approval { tool: "Edit".into(), input: json!({"file_path":"../outside.txt"}) }, Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Edit".into())).unwrap();
    loop { if let Update::Event(e) = rig.next().await && matches!(e.kind, EventKind::ApprovalRequest { .. }) { break; } }
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await; rig.task.abort();
}

#[tokio::test]
async fn always_allow_in_a_workspace_session_is_refused_and_not_forwarded() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Approval { tool: "Bash".into(), input: json!({"command":"test"}) }, Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Run".into())).unwrap();
    let id = loop { if let Update::Event(e) = rig.next().await && let EventKind::ApprovalRequest { approval_id, .. } = e.kind { break approval_id; } };
    rig.commands.send(Command::Decide { approval_id: id.clone(), decision: Decision::AlwaysAllow }).unwrap();
    assert_eq!(rig.next().await, Update::Error("Always allow is not available in write sessions yet. Approve this request once.".into()));
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Decide { approval_id: id, decision: Decision::Approve }).unwrap(); rig.until_finished().await; rig.task.abort();
}

#[tokio::test]
async fn sandbox_fallback_drops_the_lock_and_the_commit_tool() {
    let (dir, mut engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::idle()]).with_workspace_error("test unavailable"), true);
    let memory = FakeMemory::new(false); engine.memory = Some(memory.clone()); let locks = engine.workspace_locks.clone();
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); assert_eq!(specs.len(), 2);
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Conversation);
    assert_eq!(specs[1].mcp_config, Some(PathBuf::from("/run/discard/session.json")));
    assert!(specs[1].system_prompt.as_ref().unwrap().contains("the provider sandbox is unavailable (test unavailable)"));
    assert_eq!(locks.holder(&dir.path().join("worktree")), None);
    assert!(memory.finished.lock().unwrap().contains(&"workspace-token".into())); rig.task.abort();
}

#[tokio::test]
async fn lock_is_released_after_cancel_provider_crash_and_setup_failure() {
    for case in ["cancel", "crash", "setup"] {
        let script = if case == "crash" { vec![Step::working(), Step::Crash] } else { vec![Step::working(), Step::Hang] };
        let (dir, mut engine, probe) = workspace_engine(FakeProvider::new(script), true);
        let locks = engine.workspace_locks.clone();
        if case == "setup" { engine.memory = Some(FakeMemory::new(true)); }
        let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Edit".into())).unwrap();
        if case == "cancel" {
            loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
            assert_eq!(locks.holder(&dir.path().join("worktree")), Some(rig.channel.clone()));
            rig.commands.send(Command::Cancel).unwrap();
        }
        rig.until_finished().await; assert_eq!(locks.holder(&dir.path().join("worktree")), None, "{case}"); rig.task.abort();
    }
}

#[tokio::test]
async fn second_channel_on_the_same_path_waits_and_then_runs() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::Hang]), true);
    let mut second = engine.clone(); let second_provider = FakeProvider::new(vec![Step::working(), Step::idle()]); let second_probe = second_provider.probe.clone(); second.provider = Arc::new(second_provider);
    {
        let store = engine.store.lock().unwrap(); let project = Project { id: ProjectId::new(), name: "Other".into(), path: engine.cwd.to_string_lossy().into() }; store.insert_project(&project).unwrap();
        let channel = Channel { id: ChannelId::new(), name: "second".into(), agent_id: engine.agent_id.clone(), project_id: Some(project.id.clone()), workspace_mode: WorkspaceMode::Worktree };
        let mut record = store.worktree_for(&engine.channel_id).unwrap().unwrap(); record.channel_id = channel.id.clone();
        store.create_named_channel(&channel, Some("feature"), Some(&record)).unwrap();
        second.channel_id = channel.id; second.project_id = Some(project.id); second.channel_name = channel.name; second.session_id = SessionId::new(); store.create_session(&second.session_id, &second.channel_id).unwrap();
    }
    let mut first = launch_workspace(engine, probe); first.commands.send(Command::Send("hold".into())).unwrap();
    loop { if matches!(first.next().await, Update::Event(_)) { break; } }
    let mut second = launch_workspace(second, second_probe); second.commands.send(Command::Send("wait".into())).unwrap();
    loop { if let Update::Notice(text) = second.next().await && text.starts_with("Waiting for #feature") { break; } }
    assert!(second.probe.specs.lock().unwrap().is_empty());
    first.commands.send(Command::Cancel).unwrap(); first.until_finished().await; second.until_finished().await;
    assert_eq!(second.probe.specs.lock().unwrap().len(), 1); first.task.abort(); second.task.abort();
}

#[tokio::test]
async fn commits_made_during_a_response_link_to_that_response() {
    let (dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::text("done"), Step::Hang]), true);
    let access = discard_core::worktree_access::WorktreeAccess::new(engine.store.clone()).with_locks(engine.workspace_locks.clone()); let agent = engine.agent_id.clone();
    let root = dir.path().join("worktree");
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false")] {
        assert!(std::process::Command::new("git").current_dir(&root).args(["config", key, value]).status().unwrap().success());
    }
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("commit".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    std::fs::write(root.join("a"), "new").unwrap();
    access.commit(&agent, &rig.channel, &root, &discard_core::workspace_commit::CommitRequest { message: "Change".into(), paths: None }).unwrap();
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
    let store = rig.store.lock().unwrap(); let messages = store.messages_for_channel(&rig.channel).unwrap();
    let response = messages.iter().find(|m| m.role == Role::Agent).unwrap();
    assert_eq!(store.link_agent_commits(&rig.channel, &response.id).unwrap(), 0); rig.task.abort();
}

#[tokio::test]
async fn workspace_responses_always_save_a_changes_card() {
    for (script, expected) in [(vec![Step::working(), Step::idle()], discard_core::workspace_changes::ChangesState::Completed), (vec![Step::working(), Step::Crash], discard_core::workspace_changes::ChangesState::Failed)] {
        let (_dir, engine, probe) = workspace_engine(FakeProvider::new(script), true);
        let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("check".into())).unwrap(); rig.until_finished().await;
        let store = rig.store.lock().unwrap(); let messages = store.messages_for_channel(&rig.channel).unwrap();
        assert!(messages.iter().any(|m| m.role == Role::Agent && m.blocks.iter().any(|b| matches!(b, Block::Changes { state, .. } if *state == expected)))); rig.task.abort();
    }
}

#[tokio::test]
async fn changes_card_failure_still_saves_a_notice_and_releases_the_lock() {
    let (dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::Hang]), true);
    let locks = engine.workspace_locks.clone(); let root = dir.path().join("worktree");
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("check".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    std::fs::remove_file(root.join(".git")).unwrap();
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
    assert_eq!(locks.holder(&root), None);
    assert!(rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap().iter().any(|m| m.blocks.iter().any(|b| matches!(b, Block::Markdown { text } if text.starts_with("Discard could not list this response's changes:")))));
    rig.task.abort();
}

#[tokio::test]
async fn escaped_edits_use_full_access_cards_and_user_only_audit() {
    use discard_core::approval::{ApprovalKind, Decider};
    for path in ["../outside.txt", ".git", ".git/config", ".GIT/config"] {
        let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Approval { tool: "Edit".into(), input: json!({"file_path":path}) }, Step::idle()]), true);
        let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Edit".into())).unwrap();
        let id = loop { if let Update::Event(e) = rig.next().await && let EventKind::ApprovalRequest { approval_id, kind, .. } = e.kind { assert_eq!(kind, ApprovalKind::SandboxEscape, "{path}"); break approval_id; } };
        let mut approval = rig.store.lock().unwrap().load_approval(&id).unwrap();
        assert_eq!(approval.kind, ApprovalKind::SandboxEscape);
        assert!(approval.decide(Decision::Approve, Decider::Plugin("plugin".into()), "ask").is_err());
        assert!(rig.probe.responses.lock().unwrap().is_empty());
        rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await; rig.task.abort();
    }
}

#[tokio::test]
async fn workspace_setup_errors_still_save_failed_changes_cards() {
    for failure in ["memory", "context"] {
        let (_dir, mut engine, probe) = workspace_engine(FakeProvider::new(vec![Step::idle()]), true);
        if failure == "memory" { engine.memory = Some(FakeMemory::new(true)); }
        let mut rig = launch_workspace(engine, probe);
        rig.commands.send(Command::Send(if failure == "context" { "x".repeat(200_000) } else { "edit".into() })).unwrap(); rig.until_finished().await;
        assert!(rig.store.lock().unwrap().messages_for_channel(&rig.channel).unwrap().iter().any(|m| m.blocks.iter().any(|b| matches!(b, Block::Changes { state: discard_core::workspace_changes::ChangesState::Failed, .. }))), "{failure}");
        rig.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopped_response_waits_for_active_commit_before_saving_changes() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::Hang]), true);
    let root = dir.path().join("worktree");
    let script = dir.path().join("filter.sh"); let entered = dir.path().join("entered"); let release = dir.path().join("release");
    std::fs::write(&script, format!("#!/bin/sh\nif [ ! -e '{}' ]; then\n touch '{}'\n while [ ! -e '{}' ]; do sleep 0.01; done\nfi\ncat\n", entered.display(), entered.display(), release.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("filter.delay.clean", script.to_str().unwrap())] {
        assert!(std::process::Command::new("git").current_dir(&root).args(["config", key, value]).status().unwrap().success());
    }
    std::fs::write(root.join(".gitattributes"), "gated.txt filter=delay\n").unwrap();
    let access = Arc::new(discard_core::worktree_access::WorktreeAccess::new(engine.store.clone()).with_locks(engine.workspace_locks.clone()));
    let agent = engine.agent_id.clone(); let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("commit".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    std::fs::write(root.join("gated.txt"), "new\n").unwrap();
    let commit_root = root.clone(); let channel = rig.channel.clone();
    let commit = tokio::task::spawn_blocking(move || access.commit(&agent, &channel, &commit_root, &discard_core::workspace_commit::CommitRequest { message: "Delayed commit".into(), paths: None }));
    tokio::time::timeout(Duration::from_secs(5), async { while !entered.exists() { tokio::time::sleep(Duration::from_millis(10)).await; } }).await.unwrap();
    let gate = release.clone(); std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(500)); std::fs::write(gate, "release").unwrap(); });
    rig.commands.send(Command::Cancel).unwrap();
    let response = loop { if let Update::ResponseCompleted(message) = rig.next().await { break message; } };
    assert!(release.exists(), "saved changes before the commit completed");
    let committed = commit.await.unwrap().unwrap();
    assert!(response.blocks.iter().any(|b| matches!(b, Block::Changes { commits, state: discard_core::workspace_changes::ChangesState::Stopped, .. } if commits.iter().any(|c| c.id == committed.commit))));
    assert_eq!(rig.store.lock().unwrap().link_agent_commits(&rig.channel, &response.id).unwrap(), 0);
    rig.until_finished().await; rig.task.abort();
}
