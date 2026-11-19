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
        workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
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
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
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
async fn the_engine_reports_the_saved_id_of_the_user_message() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("hello".into())).unwrap();
    let updates = rig.until_finished().await;
    let saved: Vec<_> = updates.iter().filter_map(|u| match u { Update::UserMessageSaved(id) => Some(id.clone()), _ => None }).collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0], rig.messages()[0].id, "the update names the stored user message");
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id: approval_id.clone(), decision: Decision::Approve }).unwrap();
    seen.extend(rig.until_finished().await);
    assert!(seen.contains(&Update::ApprovalState { approval_id: approval_id.clone(), state: ApprovalState::Decided(Decision::Approve), decided_by: discard_core::model::DecidedBy::User }));
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
        rig.commands.send(Command::Decide { rule_cut: None, approval_id: approval_id.clone(), decision: d }).unwrap();
    }
    let updates = rig.until_finished().await;
    let states = updates.iter().filter(|u| matches!(u, Update::ApprovalState { .. })).count();
    assert_eq!(states, 1);
    assert_eq!(rig.probe.responses.lock().unwrap().len(), 1);
}

fn saved_approval(rig: &Rig) -> serde_json::Value {
    let messages = rig.messages();
    let agent = messages.iter().rev().find(|m| m.role == Role::Agent).expect("a saved agent reply");
    let block = agent.blocks.iter().find(|b| matches!(b, discard_core::model::Block::Approval { .. })).expect("a saved approval block");
    serde_json::to_value(block).unwrap()
}

#[tokio::test]
async fn a_user_decision_is_saved_with_the_approval_block() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), bash(), Step::text("done"), Step::idle()]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await
            && let EventKind::ApprovalRequest { approval_id, .. } = e.kind
        {
            break approval_id;
        }
    };
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
    let saved = saved_approval(&rig);
    assert_eq!((saved["state"].as_str(), saved["decided_by"].as_str()), (Some("denied"), Some("user")));
    assert!(saved["call_id"].as_str().is_some_and(|id| !id.is_empty()));
}

#[tokio::test]
async fn an_approval_left_pending_at_the_end_of_the_turn_is_saved_as_stale() {
    let mut rig = start(FakeProvider::new(vec![Step::working(), bash(), Step::Hang]), ApprovalPolicy::Ask);
    rig.commands.send(Command::Send("go".into())).unwrap();
    loop {
        if let Update::Event(e) = rig.next().await
            && matches!(e.kind, EventKind::ApprovalRequest { .. })
        {
            break;
        }
    }
    rig.commands.send(Command::Cancel).unwrap();
    rig.until_finished().await;
    let saved = saved_approval(&rig);
    assert_eq!(saved["state"], "stale");
    assert!(saved.get("decided_by").is_none());
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Approve }).unwrap();
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
    assert_eq!(rig.next().await, Update::SessionProfile(discard_core::ui_state::SessionProfile::Coding));
    assert!(matches!(rig.next().await, Update::UserMessageSaved(_)), "the user message is saved before the setup check");
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
            parts: None,
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
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
    store.insert_agent(&agent_id, &AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent_id.clone(), workspace_mode: mode };
    store.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    store.create_session(&session, &channel.id).unwrap();
    let store = Arc::new(Mutex::new(store));
    let probe = provider.probe.clone();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
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
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Deny }).unwrap();
    rig.until_finished().await;
    let names = rec.names();
    assert!(!names.iter().any(|n| n == "approval_requested" || n == "tool_call" || n == "tool_result"), "{names:?}");
    assert!(names.iter().any(|n| n == "agent_status"), "non-tool events still flow: {names:?}");
}

fn workspace_engine(provider: FakeProvider, eligible: bool) -> (tempfile::TempDir, Engine, Probe) { workspace_engine_in(provider, eligible, false) }

/// With `file_db`, the store is a file in the temp directory, so a test can break a row with its own connection.
fn workspace_engine_in(provider: FakeProvider, eligible: bool, file_db: bool) -> (tempfile::TempDir, Engine, Probe) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| { let out = discard_core::process::spawn_std(std::process::Command::new("git").current_dir(&repo).args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"]).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap(); assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr)); };
    git(&["init", "-q", "-b", "main"]);
    git(&["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "initial"]);
    let root = dir.path().join("worktree"); git(&["worktree", "add", "-b", "feature", root.to_str().unwrap()]);
    let store = if file_db { Store::open(&dir.path().join("store.db")).unwrap() } else { Store::open_in_memory().unwrap() };
    let mut config = AgentConfig::from_toml("name='A'\nprovider='fake'\nmodel='test'\npolicy='ask'\n").unwrap(); config.workspace_write = true;
    let agent = store.create_agent(&config).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let channel = Channel { id: ChannelId::new(), name: "feature".into(), project_id: Some(project.id.clone()), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_channel(&channel, eligible.then_some("feature"), eligible.then_some(&discard_core::store::WorktreeRecord { channel_id: channel.id.clone(), path: root.to_string_lossy().into(), repo: repo.to_string_lossy().into(), branch: "feature".into(), base_commit: String::new(), owned: false })).unwrap();
    let session = SessionId::new(); store.create_session(&session, &channel.id).unwrap();
    let probe = provider.probe.clone();
    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(), mode: discard_core::provider::SessionMode::Conversation, providers: vec![], store: Arc::new(Mutex::new(store)), agent_id: agent, project_id: Some(project.id), memory: Some(FakeMemory::new(false)), plugin_write: false, workspace_mode: WorkspaceMode::Worktree, workspace: None, observer: None, channel_name: channel.name, provider: Arc::new(provider), channel_id: channel.id, session_id: session, cwd: repo, model: Some("test".into()), reasoning_effort: None, system_prompt: None, policy: ApprovalPolicy::Ask, write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off };
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

fn opt_in_to_autorun(engine: &Engine) {
    let store = engine.store.lock().unwrap();
    let mut config = store.load_agent(&engine.agent_id).unwrap();
    config.sandbox_autorun = true;
    store.update_agent(&engine.agent_id, &config).unwrap();
}

#[tokio::test]
async fn autorun_agent_starts_session_with_autorun_flag() {
    let provider = FakeProvider::new(vec![Step::working(), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let (_dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); let spec = &specs[0];
    assert_eq!(spec.mode, discard_core::provider::SessionMode::Workspace);
    assert!(spec.sandbox_autorun);
    let prompt = spec.system_prompt.as_ref().unwrap();
    assert!(prompt.contains("Shell commands inside this directory run without asking."));
    assert!(!prompt.contains("Autorun is off"));
    let profile = updates.iter().find_map(|u| match u { Update::SessionProfile(p) => Some(p), _ => None }).unwrap();
    assert!(matches!(profile, discard_core::ui_state::SessionProfile::Workspace { autorun: discard_core::ui_state::AutorunState::On, .. }), "{profile:?}");
    rig.task.abort();
}

#[tokio::test]
async fn autorun_unavailable_starts_session_without_flag() {
    let provider = FakeProvider::new(vec![Step::working(), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Unsupported("not verified".into()));
    let (_dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); let spec = &specs[0];
    assert_eq!(spec.mode, discard_core::provider::SessionMode::Workspace);
    assert!(!spec.sandbox_autorun);
    let prompt = spec.system_prompt.as_ref().unwrap();
    assert!(prompt.contains("Shell commands ask for approval."));
    assert_eq!(prompt.matches("Autorun is off for this response: not verified.").count(), 1);
    let profile = updates.iter().find_map(|u| match u { Update::SessionProfile(p) => Some(p), _ => None }).unwrap();
    assert!(matches!(profile, discard_core::ui_state::SessionProfile::Workspace { autorun: discard_core::ui_state::AutorunState::Unavailable(reason), .. } if reason == "not verified"), "{profile:?}");
    rig.task.abort();
}

#[tokio::test]
async fn agent_without_autorun_reports_off() {
    let provider = FakeProvider::new(vec![Step::working(), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let (_dir, engine, probe) = workspace_engine(provider, true);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(!rig.probe.specs.lock().unwrap()[0].sandbox_autorun);
    let profile = updates.iter().find_map(|u| match u { Update::SessionProfile(p) => Some(p), _ => None }).unwrap();
    assert!(matches!(profile, discard_core::ui_state::SessionProfile::Workspace { autorun: discard_core::ui_state::AutorunState::Off, .. }), "{profile:?}");
    rig.task.abort();
}

#[tokio::test]
async fn sandbox_fallback_turns_autorun_off() {
    let provider = FakeProvider::new(vec![Step::working(), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported).with_workspace_error("test unavailable");
    let (_dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap(); assert_eq!(specs.len(), 2);
    assert!(specs[0].sandbox_autorun);
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Conversation);
    assert!(!specs[1].sandbox_autorun);
    rig.task.abort();
}

#[tokio::test]
async fn autorun_response_records_audit_row() {
    let provider = FakeProvider::new(vec![Step::working(), Step::text("done"), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let (dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let agent = engine.agent_id.clone();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let response = rig.messages().into_iter().rev().find(|m| m.role == Role::Agent).unwrap();
    let rows = rig.store.lock().unwrap().autorun_responses(&rig.channel).unwrap();
    assert_eq!(rows, vec![(response.id, agent, "fake".to_string(), dir.path().join("worktree").canonicalize().unwrap())]);
    rig.task.abort();
}

#[tokio::test]
async fn sandbox_fallback_records_no_autorun_audit() {
    let provider = FakeProvider::new(vec![Step::working(), Step::text("done"), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported).with_workspace_error("test unavailable");
    let (_dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(rig.store.lock().unwrap().autorun_responses(&rig.channel).unwrap().is_empty());
    let profile = updates.iter().rev().find_map(|u| match u { Update::SessionProfile(p) => Some(p), _ => None }).unwrap();
    assert!(matches!(profile, discard_core::ui_state::SessionProfile::ReadOnly { .. }), "{profile:?}");
    rig.task.abort();
}

/// A memory service with the prompt split that native resume needs, and a readable MCP config.
struct ResumableMemory { config: PathBuf }

impl MemoryService for ResumableMemory {
    fn prepare_workspace_turn(&self, agent: &AgentId, project: Option<&ProjectId>, source: &ChannelId, base: Option<&str>, _grant: &discard_core::workspace_session::WorkspaceGrant, _query: &str) -> Result<PreparedMemory, String> {
        self.prepare(agent, project, source, base, false)
    }
    fn prepare(&self, _: &AgentId, _: Option<&ProjectId>, _: &ChannelId, _: Option<&str>, _: bool) -> Result<PreparedMemory, String> {
        let parts = discard_core::engine::ResumableParts { fixed_prompt: "P".into(), session_state: String::new(), core_hash: String::new(), selected: vec![], live_versions: Default::default(), capture_paused: false };
        Ok(PreparedMemory { system_prompt: "P".into(), mcp_config: self.config.clone(), token: "t".into(), parts: Some(parts) })
    }
    fn finish(&self, _: &str) {}
}

#[tokio::test]
async fn a_sandbox_fallback_forgets_the_saved_session_and_reports_a_replay() {
    use discard_core::ui_state::ResumeState;
    let (dir, mut engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::text("one"), Step::idle()]).with_resume_handle("h1"), true);
    let config = dir.path().join("mcp.json");
    std::fs::write(&config, r#"{"mcpServers":{"discard-memory":{}}}"#).unwrap();
    engine.memory = Some(Arc::new(ResumableMemory { config }));
    {
        let store = engine.store.lock().unwrap();
        let mut agent = store.load_agent(&engine.agent_id).unwrap(); agent.native_resume = true;
        store.update_agent(&engine.agent_id, &agent).unwrap();
    }
    let (channel, agent) = (engine.channel_id.clone(), engine.agent_id.clone());
    let fallback = FakeProvider::new(vec![Step::working(), Step::text("two"), Step::idle()])
        .then_session(vec![Step::working(), Step::text("three"), Step::idle()])
        .with_resume_handle("h1").with_workspace_error("test unavailable");
    let fallback_probe = fallback.probe.clone();
    let mut broken = engine.clone();
    broken.provider = Arc::new(fallback);
    // A Workspace response saves a session.
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    assert_eq!(rig.probe.started_resumable.lock().unwrap().as_slice(), [true]);
    let row = rig.store.lock().unwrap().agent_session(&channel, &agent).unwrap().expect("a saved session");
    assert_eq!((row.handle.as_str(), row.dropped_reason.as_deref()), ("h1", None));
    rig.task.abort();
    // The sandbox is gone. The resumed Workspace start fails, and the read-only fallback replays.
    let mut rig = launch_workspace(broken, fallback_probe);
    rig.commands.send(Command::Send("Edit again".into())).unwrap();
    let updates = rig.until_finished().await;
    assert_eq!(rig.probe.started_with.lock().unwrap().as_slice(), [Some("h1".to_string()), None]);
    let read_only = ResumeState::Replay { reason: "the workspace sandbox is unavailable, so this response is read-only".into() };
    let states = |updates: &[Update]| updates.iter().filter_map(|u| match u { Update::ResumeState(state) => Some(state.clone()), _ => None }).collect::<Vec<_>>();
    assert_eq!(states(&updates).last(), Some(&read_only), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().agent_session(&channel, &agent).unwrap(), None);
    // The next fallback response shows the same state, and no "Fresh replay" marker.
    rig.commands.send(Command::Send("Edit once more".into())).unwrap();
    let updates = rig.until_finished().await;
    assert_eq!(states(&updates), vec![read_only]);
    assert!(!rig.messages().iter().any(|m| m.role == Role::System && matches!(&m.blocks[0], Block::Markdown { text } if text.starts_with("Fresh replay"))));
    rig.task.abort();
}

#[tokio::test]
async fn autorun_marks_shell_calls_in_the_saved_response() {
    let provider = FakeProvider::new(vec![
        Step::working(),
        Step::Emit(EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({"command": "ls"}) }),
        Step::Emit(EventKind::ToolCallResult { call_id: "c1".into(), output: "ok".into(), is_error: false }),
        Step::idle(),
    ]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let (_dir, engine, probe) = workspace_engine(provider, true);
    opt_in_to_autorun(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let response = rig.messages().into_iter().rev().find(|m| m.role == Role::Agent).unwrap();
    assert!(matches!(&response.blocks[0], Block::ToolCall { sandboxed: Some(true), .. }), "{:?}", response.blocks);
    rig.task.abort();
}

fn system_texts(rig: &Rig) -> Vec<String> {
    rig.messages().into_iter().filter(|m| m.role == Role::System).map(|m| match &m.blocks[0] { Block::Markdown { text } => text.clone(), other => panic!("{other:?}") }).collect()
}

fn nested_notices(updates: &[Update]) -> Vec<String> {
    updates.iter().filter_map(|u| match u { Update::Notice(text) if text.contains("nested .git") => Some(text.clone()), _ => None }).collect()
}

const SINGULAR: &str = "Warning: this response created a nested .git path: newdir/.git. A hook or config inside it can run commands when someone runs Git there. Review or remove it before you run Git in that directory.";

/// An opted-in autorun engine on a provider that reports support and does not refuse nested paths.
/// The script is built from the worktree root. `setup` runs on the root before the engine starts.
fn autorun_rig_with(script: impl FnOnce(&std::path::Path) -> Vec<Step>, setup: impl FnOnce(&std::path::Path)) -> (tempfile::TempDir, Rig) {
    let (dir, engine, _) = workspace_engine(FakeProvider::new(vec![]), true);
    let root = dir.path().join("worktree");
    setup(&root);
    let provider = FakeProvider::new(script(&root)).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let probe = provider.probe.clone();
    let engine = Engine { provider: Arc::new(provider), ..engine };
    opt_in_to_autorun(&engine);
    (dir, launch_workspace(engine, probe))
}

#[tokio::test]
async fn an_autorun_response_that_creates_a_nested_git_path_saves_a_warning_after_the_response() {
    let (_dir, mut rig) = autorun_rig_with(|root| vec![Step::working(), Step::CreateDir(root.join("newdir/.git")), Step::text("done"), Step::idle()], |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    assert_eq!(system_texts(&rig), vec![SINGULAR.to_string()]);
    let messages = rig.messages();
    assert_eq!(messages.last().unwrap().role, Role::System, "the warning follows the response");
    assert!(messages.iter().rev().nth(1).is_some_and(|m| m.role == Role::Agent));
    assert!(updates.iter().any(|u| matches!(u, Update::ConversationMarker(m) if m.role == Role::System)), "the UI shows it as a conversation marker");
    assert!(nested_notices(&updates).is_empty(), "nothing existed before");
    rig.task.abort();
}

#[tokio::test]
async fn the_warning_uses_the_plural_form_and_cuts_the_list_at_five_paths() {
    let (_dir, mut rig) = autorun_rig_with(|root| vec![Step::working(), Step::CreateDir(root.join("a/.git")), Step::CreateDir(root.join("b/.git")), Step::text("done"), Step::idle()], |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    assert_eq!(system_texts(&rig), vec!["Warning: this response created nested .git paths: a/.git, b/.git. A hook or config inside them can run commands when someone runs Git there. Review or remove them before you run Git in those directories.".to_string()]);
    rig.task.abort();
    let (_dir, mut rig) = autorun_rig_with(|root| [vec![Step::working()], (1..=7).map(|n| Step::CreateDir(root.join(format!("d{n}/.git")))).collect(), vec![Step::idle()]].concat(), |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    let texts = system_texts(&rig);
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("created nested .git paths: d1/.git, d2/.git, d3/.git, d4/.git, d5/.git and 2 more. A hook"), "{}", texts[0]);
    rig.task.abort();
}

#[tokio::test]
async fn a_response_that_creates_none_saves_nothing_and_old_paths_are_not_new() {
    let (_dir, mut rig) = autorun_rig_with(|_| vec![Step::working(), Step::text("done"), Step::idle()], |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    assert!(system_texts(&rig).is_empty());
    rig.task.abort();
    // A path that exists before the turn is not reported. Only the new one is.
    let (_dir, mut rig) = autorun_rig_with(|root| vec![Step::working(), Step::CreateDir(root.join("newdir/.git")), Step::idle()], |root| std::fs::create_dir_all(root.join("old/.git")).unwrap());
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    assert_eq!(system_texts(&rig), vec![SINGULAR.to_string()]);
    rig.task.abort();
}

#[tokio::test]
async fn failed_and_cancelled_turns_still_walk() {
    let (_dir, mut rig) = autorun_rig_with(|root| vec![Step::working(), Step::CreateDir(root.join("newdir/.git")), Step::Crash], |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))), "{:?}", updates.last());
    assert_eq!(system_texts(&rig), vec![SINGULAR.to_string()]);
    rig.task.abort();
    let (_dir, mut rig) = autorun_rig_with(|root| vec![Step::working(), Step::CreateDir(root.join("newdir/.git")), Step::Hang], |_| {});
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    loop { if let Update::Event(e) = rig.next().await && matches!(e.kind, EventKind::StatusChange { .. }) { break; } }
    tokio::time::sleep(Duration::from_millis(100)).await;
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))), "{:?}", updates.last());
    assert_eq!(system_texts(&rig), vec![SINGULAR.to_string()]);
    rig.task.abort();
}

#[tokio::test]
async fn a_session_without_autorun_never_walks() {
    // The script creates a nested path and the root has a pre-existing one. An autorun response would warn and notify.
    let (dir, engine, _) = workspace_engine(FakeProvider::new(vec![]), true);
    let root = dir.path().join("worktree");
    std::fs::create_dir_all(root.join("old/.git")).unwrap();
    let provider = FakeProvider::new(vec![Step::working(), Step::CreateDir(root.join("newdir/.git")), Step::idle()]).with_sandbox_autorun(discard_core::provider::SandboxAutorun::Supported);
    let probe = provider.probe.clone();
    // The agent did not opt in.
    let mut rig = launch_workspace(Engine { provider: Arc::new(provider), ..engine }, probe);
    rig.commands.send(Command::Send("Edit".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(!rig.probe.specs.lock().unwrap()[0].sandbox_autorun);
    assert!(system_texts(&rig).is_empty());
    assert!(nested_notices(&updates).is_empty());
    rig.task.abort();
}

#[tokio::test]
async fn a_walk_that_cannot_finish_saves_one_warning() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut rig) = autorun_rig_with(|_| vec![Step::working(), Step::text("done"), Step::idle()], |root| {
        std::fs::create_dir(root.join("locked")).unwrap();
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
    });
    rig.commands.send(Command::Send("Edit".into())).unwrap(); rig.until_finished().await;
    // Both walks fail. The response gets one warning, not two.
    assert_eq!(system_texts(&rig), vec!["Warning: Discard could not check this worktree for nested .git paths (a directory could not be read). An autorun response can create them with no approval card.".to_string()]);
    std::fs::set_permissions(dir.path().join("worktree/locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    rig.task.abort();
}

#[tokio::test]
async fn an_existing_nested_git_path_gives_one_notice_for_each_set_of_paths() {
    let (dir, mut rig) = autorun_rig_with(|_| vec![Step::working(), Step::idle()], |root| std::fs::create_dir_all(root.join("vendor/.git")).unwrap());
    rig.commands.send(Command::Send("one".into())).unwrap();
    assert_eq!(nested_notices(&rig.until_finished().await), vec!["This worktree has nested .git paths (vendor/.git). An autorun command can write inside them with no approval card.".to_string()]);
    // The same set shows no second notice.
    rig.commands.send(Command::Send("two".into())).unwrap();
    assert!(nested_notices(&rig.until_finished().await).is_empty());
    // A different set shows a new notice.
    std::fs::create_dir_all(dir.path().join("worktree/other/.git")).unwrap();
    rig.commands.send(Command::Send("three".into())).unwrap();
    assert_eq!(nested_notices(&rig.until_finished().await), vec!["This worktree has nested .git paths (other/.git and 1 more). An autorun command can write inside them with no approval card.".to_string()]);
    assert!(system_texts(&rig).is_empty(), "a notice is not saved as a message");
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
async fn always_allow_for_unparseable_command_is_refused() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Approval { tool: "Bash".into(), input: json!({"command":"cargo test; rm x"}) }, Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("Run".into())).unwrap();
    let id = loop { if let Update::Event(e) = rig.next().await && let EventKind::ApprovalRequest { approval_id, .. } = e.kind { break approval_id; } };
    rig.commands.send(Command::Decide { rule_cut: None, approval_id: id.clone(), decision: Decision::AlwaysAllow }).unwrap();
    assert_eq!(rig.next().await, Update::Error("This command uses shell syntax that Always allow cannot match. Approve it once.".into()));
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Decide { rule_cut: None, approval_id: id, decision: Decision::Approve }).unwrap(); rig.until_finished().await; rig.task.abort();
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
        assert!(discard_core::process::spawn_std(std::process::Command::new("git").current_dir(&root).args(["config", key, value])).and_then(|mut child| child.wait()).unwrap().success());
    }
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("commit".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    std::fs::write(root.join("a"), "new").unwrap();
    access.commit(&agent, &rig.channel, &root, &discard_core::workspace_commit::CommitRequest { message: "Change".into(), paths: None }, false).unwrap();
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
        assert!(discard_core::process::spawn_std(std::process::Command::new("git").current_dir(&root).args(["config", key, value])).and_then(|mut child| child.wait()).unwrap().success());
    }
    std::fs::write(root.join(".gitattributes"), "gated.txt filter=delay\n").unwrap();
    let access = Arc::new(discard_core::worktree_access::WorktreeAccess::new(engine.store.clone()).with_locks(engine.workspace_locks.clone()));
    let agent = engine.agent_id.clone(); let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("commit".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    std::fs::write(root.join("gated.txt"), "new\n").unwrap();
    let commit_root = root.clone(); let channel = rig.channel.clone();
    let commit = tokio::task::spawn_blocking(move || access.commit(&agent, &channel, &commit_root, &discard_core::workspace_commit::CommitRequest { message: "Delayed commit".into(), paths: None }, false));
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


async fn next_approval(rig: &mut Rig) -> ApprovalId {
    loop { if let Update::Event(e) = rig.next().await && let EventKind::ApprovalRequest { approval_id, .. } = e.kind { return approval_id; } }
}

fn command_step(command: &str) -> Step {
    Step::Approval { tool: "Bash".into(), input: json!({"command": command}) }
}

fn save_command_rule(engine: &Engine) -> discard_core::always_allow::AlwaysAllowRule {
    engine.store.lock().unwrap().insert_always_allow_rule(&engine.agent_id, engine.project_id.as_ref().unwrap(), &discard_core::always_allow::RuleTarget::Command { words: vec!["cargo".into(), "test".into()] }).unwrap()
}

#[tokio::test]
async fn matching_command_rule_auto_approves_in_workspace_sessions() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![command_step("cargo test -p x"), Step::idle()]), true);
    let rule = save_command_rule(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("run".into())).unwrap();
    rig.until_finished().await;
    let id = rig.probe.responses.lock().unwrap()[0].0.clone();
    let a = rig.store.lock().unwrap().load_approval(&id).unwrap();
    assert_eq!(a.audit[0].decider, discard_core::approval::Decider::Policy(format!("always_allow:{}", rule.id)));
    rig.task.abort();
}

#[tokio::test]
async fn compound_command_never_matches_a_rule() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![command_step("cargo test; rm x"), Step::idle()]), true);
    save_command_rule(&engine);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("run".into())).unwrap(); next_approval(&mut rig).await;
    assert!(rig.probe.responses.lock().unwrap().is_empty());
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await; rig.task.abort();
}

async fn saves_cut(cut: Option<usize>) {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![command_step("cargo test -p x"), Step::idle()]), true);
    let project = engine.project_id.clone().unwrap();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("run".into())).unwrap(); let id = next_approval(&mut rig).await;
    rig.commands.send(Command::Decide { approval_id: id.clone(), decision: Decision::AlwaysAllow, rule_cut: cut }).unwrap();
    rig.until_finished().await;
    assert_eq!(rig.probe.responses.lock().unwrap().as_slice(), [(id, Decision::Approve)]);
    let rules = rig.store.lock().unwrap().always_allow_rules(&project).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].target, discard_core::always_allow::RuleTarget::Command { words: vec!["cargo".into(), "test".into()] });
    rig.task.abort();
}
#[tokio::test]
async fn always_allow_saves_the_chosen_cut_and_approves_once() { saves_cut(Some(2)).await; }
#[tokio::test]
async fn always_allow_without_a_cut_uses_the_default() { saves_cut(None).await; }

#[tokio::test]
async fn revoked_rule_stops_matching_in_a_running_response() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![command_step("cargo test"), command_step("barrier"), command_step("cargo test"), Step::idle()]), true);
    let rule = save_command_rule(&engine);
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("run".into())).unwrap();
    next_approval(&mut rig).await;
    let barrier = next_approval(&mut rig).await;
    rig.store.lock().unwrap().delete_always_allow_rule(&rule.project, &rule.id).unwrap();
    rig.commands.send(Command::Decide { approval_id: barrier, decision: Decision::Approve, rule_cut: None }).unwrap();
    let last = next_approval(&mut rig).await;
    assert_eq!(rig.store.lock().unwrap().load_approval(&last).unwrap().state, ApprovalState::Pending);
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await; rig.task.abort();
}

#[tokio::test]
async fn coding_sessions_keep_provider_session_always_allow() {
    for rule_cut in [None, Some(1)] {
        let mut rig = start(FakeProvider::new(vec![bash(), Step::idle()]), ApprovalPolicy::Ask);
        rig.commands.send(Command::Send("run".into())).unwrap();
        let mut coding_profile = false;
        let id = loop {
            match rig.next().await {
                Update::SessionProfile(discard_core::ui_state::SessionProfile::Coding) => coding_profile = true,
                Update::Event(e) => if let EventKind::ApprovalRequest { approval_id, .. } = e.kind { break approval_id; },
                _ => {}
            }
        };
        assert!(coding_profile);
        rig.commands.send(Command::Decide { approval_id: id.clone(), decision: Decision::AlwaysAllow, rule_cut }).unwrap();
        rig.until_finished().await;
        assert_eq!(rig.probe.responses.lock().unwrap().as_slice(), [(id, Decision::AlwaysAllow)]);
        let store = rig.store.lock().unwrap();
        let project = store.load_channel(&rig.channel).unwrap().project_id.unwrap();
        assert!(store.always_allow_rules(&project).unwrap().is_empty());
        rig.task.abort();
    }
}

#[tokio::test]
async fn matching_host_rule_auto_approves_network_requests() {
    use discard_core::approval::{ApprovalKind, Decider};
    let id = ApprovalId::new();
    let event = EventKind::ApprovalRequest { approval_id: id.clone(), kind: ApprovalKind::NetworkDomain, call_id: "network".into(), tool: "SandboxNetworkAccess".into(), input: json!({"host":"REGISTRY.NPMJS.ORG"}), paths: vec![] };
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Emit(event), Step::Hang]), true);
    let rule = engine.store.lock().unwrap().insert_always_allow_rule(&engine.agent_id, engine.project_id.as_ref().unwrap(), &discard_core::always_allow::RuleTarget::Host { host: "registry.npmjs.org".into() }).unwrap();
    let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("run".into())).unwrap();
    loop { if let Update::ApprovalState { state: ApprovalState::Decided(Decision::Approve), .. } = rig.next().await { break; } }
    rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
    assert_eq!(rig.store.lock().unwrap().load_approval(&id).unwrap().audit[0].decider, Decider::Policy(format!("always_allow:{}", rule.id))); rig.task.abort();
}

#[tokio::test]
async fn rules_never_apply_to_escapes_or_other_agents() {
    for escape in [true, false] {
        let id = ApprovalId::new();
        let event = EventKind::ApprovalRequest { approval_id: id.clone(), kind: if escape { discard_core::approval::ApprovalKind::SandboxEscape } else { Default::default() }, call_id: "shell".into(), tool: "Bash".into(), input: json!({"command":"cargo test"}), paths: vec![] };
        let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::Emit(event), Step::Hang]), true);
        let rule = save_command_rule(&engine);
        if !escape {
            let store = engine.store.lock().unwrap();
            store.delete_always_allow_rule(&rule.project, &rule.id).unwrap();
            let other = store.create_agent(&store.load_agent(&engine.agent_id).unwrap()).unwrap();
            store.insert_always_allow_rule(&other, &rule.project, &rule.target).unwrap();
        }
        let mut rig = launch_workspace(engine, probe); rig.commands.send(Command::Send("run".into())).unwrap(); next_approval(&mut rig).await;
        rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
        assert!(rig.probe.responses.lock().unwrap().is_empty()); rig.task.abort();
    }
}

#[tokio::test]
async fn continue_answers_as_the_given_agent_without_a_new_user_message() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::text("done"), Step::idle()]), true);
    let agent = engine.agent_id.clone();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Continue { agent: agent.clone(), note: "Created worktree.".into() }).unwrap();
    rig.until_finished().await;
    assert!(rig.messages().iter().all(|m| m.role != Role::User));
    assert!(rig.messages().iter().any(|m| m.role == Role::Agent));
    assert!(rig.probe.sent.lock().unwrap()[0].trim_end().ends_with("The user approved your request in the app: Created worktree. Continue the user's latest request in this channel. Do not repeat work that is already done.\n</current_user_request>"));
    assert!(rig.store.lock().unwrap().message_authors(&rig.channel).unwrap().values().all(|a| a.agent_id == agent));
    rig.task.abort();
}

#[tokio::test]
async fn continue_ignores_mentions_in_history() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::text("done"), Step::idle()]), true);
    let agent = engine.agent_id.clone();
    engine.store.lock().unwrap().insert_message(&discard_core::model::Message { id: discard_core::model::MessageId::new(), channel_id: engine.channel_id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "@missing please edit".into() }] }).unwrap();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Continue { agent, note: "@missing approved".into() }).unwrap();
    rig.until_finished().await;
    assert_eq!(rig.probe.specs.lock().unwrap().len(), 1);
    assert_eq!(rig.messages().iter().filter(|m| m.role == Role::User).count(), 1);
    rig.task.abort();
}

#[tokio::test]
async fn continue_uses_current_eligibility() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::text("done"), Step::idle()]), true);
    let agent = engine.agent_id.clone();
    let record = engine.store.lock().unwrap().worktree_for(&engine.channel_id).unwrap().unwrap();
    engine.store.lock().unwrap().delete_worktree(&engine.channel_id).unwrap();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("edit".into())).unwrap(); rig.until_finished().await;
    rig.store.lock().unwrap().save_worktree(&record).unwrap();
    rig.commands.send(Command::Continue { agent, note: "Created worktree.".into() }).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap();
    assert_eq!(specs[0].mode, discard_core::provider::SessionMode::Conversation);
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Workspace);
    rig.task.abort();
}

#[tokio::test]
async fn continue_with_an_archived_agent_fails_with_the_normal_error() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::idle()]), true);
    let agent = engine.agent_id.clone();
    engine.store.lock().unwrap().set_agent_archived(&agent, true).unwrap();
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Continue { agent, note: "Approved.".into() }).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.contains(&Update::Error("This agent is archived. Restore it before sending a message.".into())));
    assert!(rig.probe.specs.lock().unwrap().is_empty()); rig.task.abort();
}

fn main_checkout_engine() -> (tempfile::TempDir, Engine, Probe) {
    let (dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::text("done"), Step::idle()]).with_main_checkout_writes(true), true);
    { let store = engine.store.lock().unwrap(); let mut record = store.worktree_for(&engine.channel_id).unwrap().unwrap(); record.path = dir.path().join("repo").to_string_lossy().into_owned(); record.branch = "main".into(); store.save_worktree(&record).unwrap(); store.rename_project_branch(engine.project_id.as_ref().unwrap(), &engine.channel_id, "main").unwrap(); }
    (dir, engine, probe)
}
#[tokio::test]
async fn granted_response_writes_in_the_main_checkout_once() {
    let (dir, engine, probe) = main_checkout_engine();
    engine.main_checkout_grants.grant(&engine.channel_id, &engine.agent_id);
    let mut rig = launch_workspace(engine, probe);
    for _ in 0..2 { rig.commands.send(Command::Send("edit".into())).unwrap(); rig.until_finished().await; }
    let specs = rig.probe.specs.lock().unwrap();
    assert_eq!(specs[0].mode, discard_core::provider::SessionMode::Workspace);
    assert_eq!(specs[0].cwd, dir.path().join("repo").canonicalize().unwrap());
    assert!(specs[0].system_prompt.as_ref().unwrap().contains("You are writing in the main checkout for this response only. The user may also be editing these files."));
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Conversation);
    assert!(specs[1].system_prompt.as_ref().unwrap().contains("agents do not write in the main checkout")); rig.task.abort();
}
#[tokio::test]
async fn a_mentioned_agent_does_not_use_another_agents_grant() {
    let (_dir, engine, probe) = main_checkout_engine();
    engine.main_checkout_grants.grant(&engine.channel_id, &engine.agent_id);
    { let store = engine.store.lock().unwrap(); let mut config = store.load_agent(&engine.agent_id).unwrap(); config.name = "Other".into(); store.create_agent(&config).unwrap(); }
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("@Other edit".into())).unwrap(); rig.until_finished().await;
    rig.commands.send(Command::Send("edit".into())).unwrap(); rig.until_finished().await;
    let specs = rig.probe.specs.lock().unwrap();
    assert_eq!(specs[0].mode, discard_core::provider::SessionMode::Conversation);
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Workspace); rig.task.abort();
}

#[tokio::test]
async fn failed_continue_does_not_leave_main_checkout_grant_for_later_send() {
    for unavailable_provider in [true, false] {
        let (_dir, engine, probe) = main_checkout_engine();
        let agent = engine.agent_id.clone();
        engine.main_checkout_grants.grant(&engine.channel_id, &agent);
        { let store = engine.store.lock().unwrap(); let mut config = store.load_agent(&agent).unwrap();
            if unavailable_provider { config.provider = "unavailable".into(); } else { config.workspace_write = false; }
            store.update_agent(&agent, &config).unwrap();
        }
        let mut rig = launch_workspace(engine, probe);
        rig.commands.send(Command::Continue { agent: agent.clone(), note: "Approved.".into() }).unwrap(); rig.until_finished().await;
        { let store = rig.store.lock().unwrap(); let mut config = store.load_agent(&agent).unwrap(); config.provider = "fake".into(); config.workspace_write = true; store.update_agent(&agent, &config).unwrap(); }
        rig.commands.send(Command::Send("edit".into())).unwrap(); rig.until_finished().await;
        assert_eq!(rig.probe.specs.lock().unwrap().last().unwrap().mode, discard_core::provider::SessionMode::Conversation);
        rig.task.abort();
    }
}

#[tokio::test]
async fn main_checkout_requests_and_grants_follow_the_responding_provider() {
    use discard_core::provider::{Provider, SessionMode};
    use discard_mcp::{service::MemoryRuntime, tools::McpHandler};
    assert!(discard_providers::claude::ClaudeProvider::new("unused").supports_main_checkout_writes());
    assert!(!discard_providers::codex::CodexProvider::new("unused").supports_main_checkout_writes());
    assert!(!FakeProvider::new(vec![]).supports_main_checkout_writes());
    for supported in [false, true] {
        let (dir, mut engine, _) = main_checkout_engine();
        let provider = Arc::new(FakeProvider::new(vec![Step::working(), Step::Hang]).with_main_checkout_writes(supported));
        let probe = provider.probe.clone();
        // The responding provider comes from the current registry, not the engine's old provider.
        engine.provider = Arc::new(FakeProvider::new(vec![]).with_main_checkout_writes(!supported));
        engine.providers = vec![provider];
        let memory = Arc::new(discard_core::memory::MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
        let requests = Arc::new(discard_core::write_requests::WriteRequests::default());
        let workspace = Arc::new(discard_core::workspace::GitWorkspace::new(&dir.path().join("owned"), engine.store.clone()));
        let access = Arc::new(discard_core::worktree_access::WorktreeAccess::new(engine.store.clone()).with_locks(engine.workspace_locks.clone()));
        let runtime = MemoryRuntime::start_with_write_services(memory.clone(), &dir.path().join("run"), None, Some(access.clone()), Some((requests.clone(), workspace.clone()))).await.unwrap();
        let handler = McpHandler::new(memory).with_worktrees(access).with_write_requests(requests.clone(), workspace);
        engine.memory = Some(runtime.clone());
        let grants = engine.main_checkout_grants.clone(); let agent = engine.agent_id.clone();
        let mut rig = launch_workspace(engine, probe);
        let identity = |spec: &discard_core::provider::SessionSpec| {
            let config: serde_json::Value = serde_json::from_slice(&std::fs::read(spec.mcp_config.as_ref().unwrap()).unwrap()).unwrap();
            let token = config["mcpServers"]["discard-memory"]["headers"]["Authorization"].as_str().unwrap().strip_prefix("Bearer ").unwrap();
            runtime.registry().lookup(token).unwrap()
        };
        rig.commands.send(Command::Send("edit".into())).unwrap();
        loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
        let spec = rig.probe.specs.lock().unwrap().last().unwrap().clone();
        assert_eq!(spec.mode, SessionMode::Conversation);
        let who = identity(&spec);
        assert_eq!(who.main_checkout_requests, supported);
        assert!(!who.main_checkout_grant);
        let tools = handler.handle(&who, &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})).unwrap();
        assert_eq!(tools.to_string().contains("main_checkout_request_write"), supported);
        let prompt = spec.system_prompt.as_deref().unwrap();
        if supported {
            assert!(prompt.contains("This branch is checked out in the main checkout. Call main_checkout_request_write to ask for write access for one response."));
        } else {
            assert!(prompt.contains("This branch is checked out in the main checkout. This agent's provider cannot write there. If the user asks for changes, explain this once."));
            assert!(!prompt.contains("main_checkout_request_write"));
        }
        let reply = handler.handle(&who, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"main_checkout_request_write","arguments":{}}})).unwrap();
        assert_eq!(reply["result"]["isError"].as_bool().unwrap_or(false), !supported);
        assert_eq!(requests.list(&rig.channel).len(), usize::from(supported));
        rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
        // Also reject a stale app approval after the provider loses support.
        grants.grant(&rig.channel, &agent);
        rig.commands.send(Command::Continue { agent: agent.clone(), note: "Approved in the app.".into() }).unwrap();
        loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
        let spec = rig.probe.specs.lock().unwrap().last().unwrap().clone();
        assert_eq!(spec.mode, if supported { SessionMode::Workspace } else { SessionMode::Conversation });
        if supported { assert_eq!(spec.cwd, dir.path().join("repo").canonicalize().unwrap()); }
        assert_eq!(identity(&spec).main_checkout_grant, supported);
        assert!(!grants.take(&rig.channel, &agent));
        rig.commands.send(Command::Cancel).unwrap(); rig.until_finished().await;
        // Old memory entry points must never infer provider support.
        let prepared = runtime.prepare_turn(&agent, who.project.as_ref(), &rig.channel, None, false, "edit").unwrap();
        let legacy = runtime.registry().lookup(&prepared.token).unwrap();
        assert!(!legacy.main_checkout_requests);
        assert!(!handler.handle(&legacy, &json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}})).unwrap().to_string().contains("main_checkout_request_write"));
        runtime.finish(&prepared.token);
        rig.task.abort();
    }
}

#[tokio::test]
async fn provider_notice_is_saved_once_without_blocking_two_turns() {
    let notice = "Verified newer CLI controls.";
    let provider = FakeProvider::new(vec![Step::text("answer"), Step::idle()]).with_notice(notice);
    let mut rig = start(provider, ApprovalPolicy::Ask);
    let mut markers = Vec::new();
    for text in ["first", "second"] {
        rig.commands.send(Command::Send(text.into())).unwrap();
        let updates = rig.until_finished().await;
        assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
        markers.extend(updates.into_iter().filter_map(|u| match u { Update::ConversationMarker(m) => Some(m), _ => None }));
    }
    if markers.is_empty() {
        loop {
            if let Update::ConversationMarker(message) = rig.next().await { markers.push(message); break; }
        }
    }
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].role, Role::System);
    assert_eq!(markers[0].blocks, vec![Block::Markdown { text: notice.into() }]);
    assert_eq!(rig.messages().iter().filter(|m| m.role == Role::System && m.blocks == markers[0].blocks).count(), 1);
    drop(rig.commands);
    rig.task.await.unwrap();
}

struct Threaded {
    _dir: tempfile::TempDir,
    parent: Engine,
    parent_probe: Probe,
    thread: Engine,
    thread_probe: Probe,
    other: AgentId,
    anchor: discard_core::model::MessageId,
}

fn put(store: &Store, channel: &ChannelId, role: Role, text: &str, author: Option<&AgentId>) -> discard_core::model::MessageId {
    let message = discard_core::model::Message { id: discard_core::model::MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: text.into() }] };
    match author {
        Some(agent) => { let config = store.load_agent(agent).unwrap(); store.insert_authored_message(&message, &discard_core::model::MessageAuthor { agent_id: agent.clone(), name: config.name, provider: config.provider, avatar: None }).unwrap() }
        None => store.insert_message(&message).unwrap(),
    }
    message.id
}

/// A worktree parent channel that can write, with a thread on its second message. `mode` is the thread engine's process mode.
fn threaded(mode: discard_core::provider::SessionMode, anchor_by_other: bool) -> Threaded { threaded_in(mode, anchor_by_other, false) }

fn threaded_in(mode: discard_core::provider::SessionMode, anchor_by_other: bool, file_db: bool) -> Threaded {
    let (dir, parent, parent_probe) = workspace_engine_in(FakeProvider::new(vec![Step::working(), Step::text("parent reply"), Step::idle()]), true, file_db);
    let provider = FakeProvider::new(vec![Step::working(), Step::text("thread reply"), Step::idle()]).with_resume_handle("must-not-resume");
    let thread_probe = provider.probe.clone();
    let (other, anchor, thread_channel, session) = {
        let store = parent.store.lock().unwrap();
        let other = store.create_agent(&AgentConfig::from_toml("name='Other'\nprovider='fake'\nmodel='test'\npolicy='ask'\n").unwrap()).unwrap();
        put(&store, &parent.channel_id, Role::User, "one", None);
        let anchor = put(&store, &parent.channel_id, Role::Agent, "anchor reply", Some(if anchor_by_other { &other } else { &parent.agent_id }));
        put(&store, &parent.channel_id, Role::User, "after the anchor", None);
        let channel = store.create_thread(&parent.channel_id, &anchor, &discard_core::store::thread_name("Side question"), &parent.agent_id).unwrap();
        let session = store.latest_session(&channel.id).unwrap().unwrap();
        (other, anchor, channel, session)
    };
    let mut thread = parent.clone();
    thread.channel_id = thread_channel.id; thread.channel_name = thread_channel.name; thread.session_id = session; thread.mode = mode; thread.provider = Arc::new(provider);
    Threaded { _dir: dir, parent, parent_probe, thread, thread_probe, other, anchor }
}

#[tokio::test]
async fn a_thread_runs_as_read_only_conversation_whatever_the_process_mode_and_parent_write_access() {
    use discard_core::provider::SessionMode;
    use discard_core::workspace_session::{ReadOnlyReason, SessionProfile};
    for mode in [SessionMode::Coding, SessionMode::Conversation] {
        let t = threaded(mode, false);
        // The parent has a verified worktree and an opted-in agent: its own turn is a Workspace session.
        let mut parent = launch_workspace(t.parent.clone(), t.parent_probe.clone());
        parent.commands.send(Command::Send("edit".into())).unwrap(); parent.until_finished().await;
        assert_eq!(parent.probe.specs.lock().unwrap()[0].mode, SessionMode::Workspace);
        let mut rig = launch_workspace(t.thread, t.thread_probe);
        rig.commands.send(Command::Send("what does that mean?".into())).unwrap();
        let updates = rig.until_finished().await;
        assert!(updates.iter().any(|u| matches!(u, Update::SessionProfile(SessionProfile::ReadOnly { reason: Some(ReadOnlyReason::Thread) }))), "{mode:?}");
        let specs = rig.probe.specs.lock().unwrap();
        let spec = &specs[0];
        assert_eq!(spec.mode, SessionMode::Conversation, "{mode:?}");
        assert_eq!(spec.resume_handle, None, "{mode:?}");
        assert!(!spec.sandbox_autorun);
        let system = spec.system_prompt.as_deref().unwrap();
        assert!(system.contains("File edits are unavailable in this response: threads are read-only."), "{system}");
        assert!(!system.contains("Workspace writes"), "{system}");
        assert_ne!(spec.cwd, t._dir.path().join("worktree").canonicalize().unwrap());
        assert_eq!(rig.probe.started_with.lock().unwrap().as_slice(), [None]);
        drop(specs);
        assert!(rig.messages().iter().any(|m| m.role == Role::Agent), "the reply is saved in the thread");
        assert!(rig.store.lock().unwrap().resume_handle(&rig.session).unwrap().is_none(), "no resume handle is saved");
        rig.task.abort(); parent.task.abort();
    }
}

#[tokio::test]
async fn a_thread_never_asks_for_main_checkout_writes() {
    #[derive(Default)]
    struct Spy(Mutex<Vec<bool>>);
    impl MemoryService for Spy {
        fn prepare_response(&self, _agent: &AgentId, _project: Option<&ProjectId>, _source: &ChannelId, _base: Option<&str>, _plugin_write: bool, options: discard_core::engine::MemoryOptions<'_>) -> Result<PreparedMemory, String> {
            self.0.lock().unwrap().push(options.main_checkout_requests);
            Ok(PreparedMemory { system_prompt: "P".into(), mcp_config: PathBuf::from("/run/c.json"), token: "t".into(), parts: None })
        }
        fn prepare(&self, _: &AgentId, _: Option<&ProjectId>, _: &ChannelId, _: Option<&str>, _: bool) -> Result<PreparedMemory, String> { unreachable!() }
        fn finish(&self, _: &str) {}
    }
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let spy = Arc::new(Spy::default());
    let (mut parent, mut thread) = (t.parent.clone(), t.thread.clone());
    parent.memory = Some(spy.clone()); parent.provider = Arc::new(FakeProvider::new(vec![Step::idle()]).with_main_checkout_writes(true));
    thread.memory = Some(spy.clone()); thread.provider = Arc::new(FakeProvider::new(vec![Step::idle()]).with_main_checkout_writes(true));
    let mut parent = launch_workspace(parent, t.parent_probe.clone()); parent.commands.send(Command::Send("hi".into())).unwrap(); parent.until_finished().await;
    let mut rig = launch_workspace(thread, t.thread_probe.clone()); rig.commands.send(Command::Send("hi".into())).unwrap(); rig.until_finished().await;
    assert_eq!(spy.0.lock().unwrap().as_slice(), [true, false], "the parent may request main-checkout writes, its thread may not");
    rig.task.abort(); parent.task.abort();
}

#[tokio::test]
async fn a_thread_replays_the_parent_up_to_the_anchor_then_its_own_messages() {
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let parent_id = t.parent.channel_id.clone();
    let mut rig = launch_workspace(t.thread, t.thread_probe);
    rig.commands.send(Command::Send("first side question".into())).unwrap(); rig.until_finished().await;
    rig.commands.send(Command::Send("second side question".into())).unwrap(); rig.until_finished().await;
    let sent = rig.probe.sent.lock().unwrap().clone();
    assert!(sent[0].contains("\"one\"") && sent[0].contains("anchor reply"), "{}", sent[0]);
    assert!(!sent[0].contains("after the anchor"), "later parent messages stay out");
    assert!(sent[0].trim_end().ends_with("first side question\n</current_user_request>"), "{}", sent[0]);
    assert!(sent[1].contains("first side question") && sent[1].contains("thread reply") && sent[1].contains("anchor reply"), "{}", sent[1]);
    assert!(!sent[1].contains("after the anchor"));
    let state = rig.store.lock().unwrap().conversation_context(&rig.channel).unwrap();
    assert_eq!(state.summary, None);
    assert_eq!(rig.store.lock().unwrap().thread_info(&rig.channel).unwrap().unwrap().anchor_message_id, Some(t.anchor.clone()));
    assert_eq!(rig.store.lock().unwrap().messages_for_channel(&parent_id).unwrap().len(), 3, "the parent transcript is unchanged");
    rig.task.abort();
}

#[tokio::test]
async fn the_anchor_author_answers_in_a_thread_and_a_mention_overrides_one_response() {
    for (by_other, expected) in [(true, "other"), (false, "main")] {
        let t = threaded(discard_core::provider::SessionMode::Conversation, by_other);
        let (main, other) = (t.parent.agent_id.clone(), t.other.clone());
        let mut rig = launch_workspace(t.thread, t.thread_probe);
        rig.commands.send(Command::Send("who answers?".into())).unwrap();
        let updates = rig.until_finished().await;
        let responder = updates.iter().find_map(|u| match u { Update::Responding(author) => Some(author.agent_id.clone()), _ => None }).unwrap();
        assert_eq!(responder, if expected == "other" { other.clone() } else { main.clone() });
        rig.commands.send(Command::Send("@Other you then".into())).unwrap();
        let updates = rig.until_finished().await;
        assert!(updates.iter().any(|u| matches!(u, Update::Responding(a) if a.agent_id == other)));
        rig.commands.send(Command::Send("and again".into())).unwrap();
        let updates = rig.until_finished().await;
        let responder = updates.iter().find_map(|u| match u { Update::Responding(author) => Some(author.agent_id.clone()), _ => None }).unwrap();
        assert_eq!(responder, if expected == "other" { other } else { main }, "the mention applied to one response only");
        rig.task.abort();
    }
}

#[tokio::test]
async fn a_user_message_in_a_thread_follows_it_and_continue_does_not() {
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let agent = t.parent.agent_id.clone();
    let thread = t.thread.channel_id.clone();
    t.thread.store.lock().unwrap().set_thread_followed(&thread, false).unwrap();
    let mut rig = launch_workspace(t.thread, t.thread_probe);
    rig.commands.send(Command::Continue { agent, note: "Approved.".into() }).unwrap(); rig.until_finished().await;
    assert!(!rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed);
    rig.commands.send(Command::Send("a reply".into())).unwrap(); rig.until_finished().await;
    assert!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed);
    rig.task.abort();
}

#[tokio::test]
async fn also_post_shows_the_user_message_in_the_parent_and_the_parent_replay_labels_it() {
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let (parent_id, thread_id) = (t.parent.channel_id.clone(), t.thread.channel_id.clone());
    let mut rig = launch_workspace(t.thread, t.thread_probe);
    rig.commands.send(Command::SendInThread { text: "private note".into(), attachments: vec![], also_post: false }).unwrap(); rig.until_finished().await;
    assert!(rig.store.lock().unwrap().posted_sources(&parent_id).unwrap().is_empty(), "off by default");
    rig.commands.send(Command::SendInThread { text: "share this".into(), attachments: vec![], also_post: true }).unwrap();
    let updates = rig.until_finished().await;
    let posted = updates.iter().find_map(|u| match u { Update::MessagePosted { reference, source } => Some((reference.clone(), source.clone())), _ => None }).expect("a MessagePosted update");
    assert_eq!((posted.0.channel_id.clone(), posted.0.role), (parent_id.clone(), Role::User));
    assert_eq!(posted.1.thread_channel_id, thread_id);
    assert_eq!(posted.1.thread_name, "Side question");
    let thread_messages = rig.messages();
    assert_eq!(posted.1.source_message_id, thread_messages.iter().find(|m| matches!(&m.blocks[..], [Block::Markdown { text }] if text == "share this")).unwrap().id, "the source is the thread's saved message");
    {
        let store = rig.store.lock().unwrap();
        let parent_messages = store.messages_for_channel(&parent_id).unwrap();
        assert_eq!(parent_messages.len(), 4, "only the user message was posted, not the agent reply or the first note");
        assert!(matches!(&parent_messages[3].blocks[..], [Block::Markdown { text }] if text == "share this"));
        assert_eq!(store.posted_sources(&parent_id).unwrap().len(), 1);
    }
    // The next parent turn sees the post, labelled, and nothing else from the thread. The post itself started no parent turn.
    assert!(t.parent_probe.specs.lock().unwrap().is_empty());
    let mut parent = launch_workspace(t.parent, t.parent_probe);
    parent.commands.send(Command::Send("back in the channel".into())).unwrap(); parent.until_finished().await;
    let sent = parent.probe.sent.lock().unwrap()[0].clone();
    assert!(sent.contains("[From thread \"Side question\"]"), "{sent}");
    assert!(sent.contains("share this"));
    assert!(!sent.contains("private note") && !sent.contains("thread reply"), "unposted thread messages never enter the parent replay: {sent}");
    rig.task.abort(); parent.task.abort();
}

#[tokio::test]
async fn also_post_is_refused_outside_a_thread() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::idle()]), true);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::SendInThread { text: "x".into(), attachments: vec![], also_post: true }).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("Only a message in a thread"))));
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    assert!(rig.messages().is_empty() && rig.probe.specs.lock().unwrap().is_empty());
    rig.task.abort();
}

#[tokio::test]
async fn a_thread_turn_runs_while_the_parent_turn_is_still_running() {
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let mut parent_engine = t.parent.clone();
    let hold = FakeProvider::new(vec![Step::working(), Step::Hang]);
    let hold_probe = hold.probe.clone();
    parent_engine.provider = Arc::new(hold);
    let mut parent = launch_workspace(parent_engine, hold_probe);
    parent.commands.send(Command::Send("hold".into())).unwrap();
    loop { if matches!(parent.next().await, Update::Event(_)) { break; } }
    let mut rig = launch_workspace(t.thread, t.thread_probe);
    rig.commands.send(Command::Send("quick question".into())).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert!(!updates.iter().any(|u| matches!(u, Update::Notice(n) if n.starts_with("Waiting for"))), "the thread never waits on the parent's worktree");
    assert!(rig.messages().iter().any(|m| m.role == Role::Agent));
    parent.commands.send(Command::Cancel).unwrap();
    parent.until_finished().await;
    rig.task.abort(); parent.task.abort();
}

// ---- Sub-agent threads and delegation ----

/// Records the `delegation` option of every response and puts the base prompt into the session prompt.
#[derive(Default)]
struct DelegationSpy(Mutex<Vec<bool>>);
impl MemoryService for DelegationSpy {
    fn prepare_response(&self, _agent: &AgentId, _project: Option<&ProjectId>, _source: &ChannelId, base: Option<&str>, _plugin_write: bool, options: discard_core::engine::MemoryOptions<'_>) -> Result<PreparedMemory, String> {
        self.0.lock().unwrap().push(options.delegation);
        Ok(PreparedMemory { system_prompt: base.unwrap_or_default().into(), mcp_config: PathBuf::from("/run/c.json"), token: "t".into(), parts: None })
    }
    fn prepare(&self, _: &AgentId, _: Option<&ProjectId>, _: &ChannelId, _: Option<&str>, _: bool) -> Result<PreparedMemory, String> { unreachable!() }
    fn finish(&self, _: &str) {}
}

struct SubAgent { t: Threaded, engine: Engine, probe: Probe, thread: ChannelId, sub_agent: AgentId, parent_author: discard_core::model::MessageAuthor, spy: Arc<DelegationSpy> }

/// A sub-agent thread of a worktree parent. The sub-agent is "Other". `script` is what its provider does.
fn sub_agent(script: Vec<Step>) -> SubAgent { sub_agent_in(script, false) }

fn sub_agent_in(script: Vec<Step>, file_db: bool) -> SubAgent { sub_agent_with(FakeProvider::new(script), file_db) }

/// Like `sub_agent`, with a provider that has a script for each session.
fn sub_agent_with(provider: FakeProvider, file_db: bool) -> SubAgent {
    let t = threaded_in(discard_core::provider::SessionMode::Conversation, false, file_db);
    let probe = provider.probe.clone();
    let spy = Arc::new(DelegationSpy::default());
    let (thread, session, parent_author) = {
        let store = t.parent.store.lock().unwrap();
        let thread = store.create_sub_agent_thread(&t.parent.channel_id, "Find callers", &t.other).unwrap();
        let config = store.load_agent(&t.parent.agent_id).unwrap();
        (thread.id.clone(), store.latest_session(&thread.id).unwrap().unwrap(), discard_core::model::MessageAuthor { agent_id: t.parent.agent_id.clone(), name: config.name, provider: config.provider, avatar: None })
    };
    let mut engine = t.parent.clone();
    engine.channel_id = thread.clone(); engine.channel_name = "Find callers".into(); engine.session_id = session; engine.provider = Arc::new(provider); engine.memory = Some(spy.clone());
    let sub_agent = t.other.clone();
    SubAgent { t, engine, probe, thread, sub_agent, parent_author, spy }
}

fn brief(text: &str, author: &discard_core::model::MessageAuthor) -> Command { Command::SendBrief { text: text.into(), author: author.clone() } }

#[tokio::test]
async fn a_brief_runs_one_turn_in_a_sub_agent_thread_and_reports_the_result() {
    use discard_core::model::ThreadState;
    let s = sub_agent(vec![Step::working(), Step::text("Callers: a, b."), Step::idle()]);
    let (thread, other, author) = (s.thread.clone(), s.sub_agent.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    // The text names the parent agent, but a brief never selects a recipient.
    rig.commands.send(brief("@A please find every caller of foo.", &author)).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Responding(a) if a.agent_id == other)), "the sub-agent answers");
    assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))), "{updates:?}");
    let finished = updates.iter().position(|u| matches!(u, Update::SubAgentFinished { .. })).expect("SubAgentFinished");
    assert_eq!(updates[finished], Update::SubAgentFinished { state: ThreadState::Done, text: "Callers: a, b.".into() });
    assert_eq!(finished + 2, updates.len(), "it comes right before TurnFinished");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    let messages = rig.messages();
    assert_eq!(messages.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent]);
    {
        let store = rig.store.lock().unwrap();
        assert_eq!(store.message_author(&messages[0].id).unwrap().unwrap().agent_id, author.agent_id, "the parent agent wrote the brief");
        assert_eq!(store.message_author(&messages[1].id).unwrap().unwrap().agent_id, other);
        let info = store.thread_info(&thread).unwrap().unwrap();
        assert_eq!((info.state, info.followed, info.anchor_message_id), (Some(ThreadState::Done), false, None), "a brief does not follow the thread");
    }
    assert!(updates.iter().any(|u| matches!(u, Update::UserMessageSaved(id) if *id == messages[0].id)));
    let sent = rig.probe.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("@A please find every caller of foo.") && sent[0].trim_end().ends_with("</current_user_request>"), "{}", sent[0]);
    assert!(!sent[0].contains("anchor reply") && !sent[0].contains("after the anchor"), "no parent transcript: {}", sent[0]);
    let specs = rig.probe.specs.lock().unwrap();
    let system = specs[0].system_prompt.as_deref().unwrap();
    assert_eq!(specs[0].mode, discard_core::provider::SessionMode::Conversation);
    assert!(system.contains("File edits are unavailable in this response: threads are read-only."), "{system}");
    assert!(system.contains("write @you in plain text") && system.contains("sub-agent thread"), "{system}");
    assert_eq!(s.spy.0.lock().unwrap().as_slice(), [false], "a sub-agent gets no delegation tools");
    drop(specs); drop(sent);
    rig.task.abort();
}

#[tokio::test]
async fn a_sub_agent_thread_takes_one_brief_and_refuses_messages_until_it_has_one() {
    let s = sub_agent(vec![Step::working(), Step::text("first"), Step::idle()]);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    // Before the brief the task still waits for its turn, so a message has no turn to steer and no result to follow up.
    for refused in [Command::Send("hello".into()), Command::SendInThread { text: "x".into(), attachments: vec![], also_post: false }] {
        rig.commands.send(refused).unwrap();
        let updates = rig.until_finished().await;
        assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("has not started its brief"))), "{updates:?}");
        assert!(rig.messages().is_empty() && rig.probe.specs.lock().unwrap().is_empty(), "nothing is saved or started");
    }
    // A Continue is an approval of the app, which a sub-agent thread never asks for.
    rig.commands.send(Command::Continue { agent: author.agent_id.clone(), note: "ok".into() }).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("messages from the user only"))), "{updates:?}");
    assert!(rig.messages().is_empty() && rig.probe.specs.lock().unwrap().is_empty());
    rig.commands.send(brief("go", &author)).unwrap(); rig.until_finished().await;
    rig.commands.send(brief("again", &author)).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("already ran"))), "{updates:?}");
    assert_eq!(rig.messages().len(), 2, "the second brief is not saved");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(discard_core::model::ThreadState::Done));
    // A brief needs a sub-agent thread.
    let mut question = launch_workspace(s.t.thread, s.t.thread_probe);
    question.commands.send(brief("go", &author)).unwrap();
    let updates = question.until_finished().await;
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("only a sub-agent thread"))), "{updates:?}");
    rig.task.abort(); question.task.abort();
}

#[tokio::test]
async fn a_message_after_the_result_is_a_follow_up_turn_that_leaves_the_state_alone() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("Callers: a, b."), Step::idle()])
        .then_session(vec![Step::working(), Step::text("It is in foo.rs."), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let (thread, author, other) = (s.thread.clone(), s.parent_author.clone(), s.sub_agent.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("find the callers", &author)).unwrap(); rig.until_finished().await;
    assert!(!rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed);
    rig.commands.send(Command::SendInThread { text: "Where is a defined?".into(), attachments: vec![], also_post: false }).unwrap();
    let updates = rig.until_finished().await;
    assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))), "{updates:?}");
    assert!(updates.iter().any(|u| matches!(u, Update::Responding(a) if a.agent_id == other)));
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    // The state and the result of the task stay. The app sees the same final state again.
    assert!(updates.contains(&Update::SubAgentFinished { state: ThreadState::Done, text: "It is in foo.rs.".into() }), "{updates:?}");
    let messages = rig.messages();
    assert_eq!(messages.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent, Role::User, Role::Agent]);
    let info = rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap();
    assert_eq!((info.state, info.followed), (Some(ThreadState::Done), true), "the message follows the thread");
    assert!(updates.iter().any(|u| matches!(u, Update::UserMessageSaved(id) if *id == messages[2].id)));
    let sent = rig.probe.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    for part in ["find the callers", "Callers: a, b.", "Where is a defined?"] { assert!(sent[1].contains(part), "{part}: {}", sent[1]); }
    assert!(!sent[1].contains("anchor reply"), "no parent transcript: {}", sent[1]);
    let specs = rig.probe.specs.lock().unwrap();
    assert_eq!(specs[1].mode, discard_core::provider::SessionMode::Conversation);
    assert!(specs[1].system_prompt.as_deref().unwrap().contains("File edits are unavailable in this response: threads are read-only."));
    drop(specs); drop(sent);
    rig.task.abort();
}

#[tokio::test]
async fn a_follow_up_turn_that_fails_does_not_change_a_final_state() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("Callers: a, b."), Step::idle()])
        .then_session(vec![Step::working(), Step::Crash]);
    let s = sub_agent_with(provider, false);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("find the callers", &author)).unwrap(); rig.until_finished().await;
    rig.commands.send(Command::SendInThread { text: "again".into(), attachments: vec![], also_post: false }).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::Done), "the first final state wins");
    rig.task.abort();
}

/// Read updates until the sub-agent's provider streamed some text.
async fn until_text(rig: &mut Rig) -> Vec<Update> {
    let mut seen = Vec::new();
    loop { let u = rig.next().await; let done = matches!(&u, Update::Event(e) if matches!(e.kind, EventKind::TextDelta { .. })); seen.push(u); if done { return seen; } }
}

fn steer(text: &str) -> Command { Command::SendInThread { text: text.into(), attachments: vec![], also_post: false } }

#[tokio::test]
async fn a_steer_stops_the_turn_and_runs_a_new_one_inside_the_same_task() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial work"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("Final: only the tests."), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("find the callers", &author)).unwrap();
    let mut updates = until_text(&mut rig).await;
    rig.commands.send(steer("Only look in tests.")).unwrap();
    updates.extend(rig.until_finished().await);
    assert!(!updates.iter().any(|u| matches!(u, Update::Error(_))), "{updates:?}");
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::TurnFinished(_))).count(), 1, "the interrupted turn does not end the task's turn: {updates:?}");
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::SubAgentFinished { .. })).count(), 1, "{updates:?}");
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))));
    assert_eq!(updates[updates.len() - 2], Update::SubAgentFinished { state: ThreadState::Done, text: "Final: only the tests.".into() }, "the result is the last turn's message");
    let messages = rig.messages();
    assert_eq!(messages.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent, Role::User, Role::Agent]);
    // The partial response shows before the steer, which shows by `UserMessageAdded`.
    let partial = updates.iter().position(|u| matches!(u, Update::ResponseCompleted(m) if m.id == messages[1].id)).expect("the partial response is saved");
    let added = updates.iter().position(|u| matches!(u, Update::UserMessageAdded(m) if m.id == messages[2].id)).expect("the steer shows");
    assert!(partial < added);
    assert!(updates[added..].contains(&Update::ThreadFollowed), "the app has no local message to follow from");
    assert!(rig.probe.cancelled.load(std::sync::atomic::Ordering::SeqCst), "the provider turn was stopped");
    let info = rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap();
    assert_eq!((info.state, info.followed), (Some(ThreadState::Done), true), "the steer follows the thread");
    let sent = rig.probe.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    for part in ["find the callers", "partial work", "Only look in tests."] { assert!(sent[1].contains(part), "{part}: {}", sent[1]); }
    drop(sent);
    rig.task.abort();
}

#[tokio::test]
async fn a_steer_during_a_steer_turn_is_a_steer_again() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial one"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("partial two"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("Final."), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let author = s.parent_author.clone();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    let mut updates = until_text(&mut rig).await;
    rig.commands.send(steer("first steer")).unwrap();
    // Wait for the second turn's text before the second steer.
    loop { let u = rig.next().await; let text = matches!(&u, Update::Event(e) if matches!(&e.kind, EventKind::TextDelta { text } if text == "partial two")); updates.push(u); if text { break; } }
    rig.commands.send(steer("second steer")).unwrap();
    updates.extend(rig.until_finished().await);
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::SubAgentFinished { .. })).count(), 1, "{updates:?}");
    assert!(updates.contains(&Update::SubAgentFinished { state: ThreadState::Done, text: "Final.".into() }));
    assert_eq!(rig.messages().iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent, Role::User, Role::Agent, Role::User, Role::Agent]);
    rig.task.abort();
}

#[tokio::test]
async fn a_second_steer_while_one_is_pending_is_refused() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("Final."), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let author = s.parent_author.clone();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    let mut updates = until_text(&mut rig).await;
    rig.commands.send(steer("first")).unwrap();
    rig.commands.send(steer("second")).unwrap();
    updates.extend(rig.until_finished().await);
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("A turn is already running"))), "{updates:?}");
    let messages = rig.messages();
    let texts: Vec<_> = messages.iter().filter(|m| m.role == Role::User).map(|m| format!("{:?}", m.blocks)).collect();
    assert_eq!(texts.len(), 2, "the brief and the first steer: {texts:?}");
    assert!(texts[1].contains("first") && !texts.iter().any(|t| t.contains("second")));
    assert_eq!(rig.store.lock().unwrap().thread_info(&rig.channel).unwrap().unwrap().state, Some(ThreadState::Done));
    rig.task.abort();
}

#[tokio::test]
async fn a_steer_cannot_mention_another_agent() {
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial"), Step::Hang]);
    let s = sub_agent_with(provider, false);
    let author = s.parent_author.clone();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    until_text(&mut rig).await;
    // "A" is the parent agent, not the sub-agent.
    rig.commands.send(steer("@A what do you think?")).unwrap();
    loop { if matches!(rig.next().await, Update::Error(e) if e.contains("mention another agent")) { break; } }
    assert_eq!(rig.probe.specs.lock().unwrap().len(), 1, "the turn goes on");
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))));
    rig.task.abort();
}

#[tokio::test]
async fn a_stop_after_a_steer_drops_the_steer_and_ends_the_task_as_cancelled() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("must not run"), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    until_text(&mut rig).await;
    rig.commands.send(steer("change course")).unwrap();
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))), "{updates:?}");
    assert!(updates.contains(&Update::SubAgentFinished { state: ThreadState::Cancelled, text: "partial".into() }), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::Cancelled));
    assert_eq!(rig.messages().iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::Agent], "the steer is not saved");
    assert_eq!(rig.probe.specs.lock().unwrap().len(), 1, "no steer turn starts");
    rig.task.abort();
}

#[tokio::test]
async fn a_stop_during_the_steer_turn_cancels_that_turn() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("steer partial"), Step::Hang]);
    let s = sub_agent_with(provider, false);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    until_text(&mut rig).await;
    rig.commands.send(steer("change course")).unwrap();
    loop { if matches!(rig.next().await, Update::Event(e) if matches!(&e.kind, EventKind::TextDelta { text } if text == "steer partial")) { break; } }
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.contains(&Update::SubAgentFinished { state: ThreadState::Cancelled, text: "steer partial".into() }), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::Cancelled));
    assert_eq!(rig.messages().len(), 4, "brief, partial, steer, steer partial");
    rig.task.abort();
}

#[tokio::test]
async fn a_timeout_that_was_set_before_a_steer_stays() {
    use discard_core::model::ThreadState;
    let provider = FakeProvider::new(vec![Step::working(), Step::text("partial"), Step::Hang])
        .then_session(vec![Step::working(), Step::text("Final."), Step::idle()]);
    let s = sub_agent_with(provider, false);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    until_text(&mut rig).await;
    rig.store.lock().unwrap().finish_sub_agent_thread(&thread, ThreadState::TimedOut).unwrap();
    rig.commands.send(steer("hurry")).unwrap();
    let updates = rig.until_finished().await;
    assert!(updates.contains(&Update::SubAgentFinished { state: ThreadState::TimedOut, text: "Final.".into() }), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::TimedOut));
    rig.task.abort();
}

#[tokio::test]
async fn a_question_thread_still_refuses_a_message_while_its_turn_runs() {
    let mut t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let hold = FakeProvider::new(vec![Step::working(), Step::text("thinking"), Step::Hang]);
    let probe = hold.probe.clone();
    t.thread.provider = Arc::new(hold);
    let mut rig = launch_workspace(t.thread, probe);
    rig.commands.send(Command::SendInThread { text: "first".into(), attachments: vec![], also_post: false }).unwrap();
    until_text(&mut rig).await;
    rig.commands.send(steer("second")).unwrap();
    loop { if matches!(rig.next().await, Update::Error(e) if e.contains("A turn is already running")) { break; } }
    assert_eq!(rig.probe.specs.lock().unwrap().len(), 1, "the turn goes on");
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))));
    assert_eq!(rig.messages().iter().filter(|m| m.role == Role::User).count(), 1, "the second message is not saved");
    rig.task.abort();
}

#[tokio::test]
async fn a_message_during_a_follow_up_turn_is_not_a_steer() {
    let provider = FakeProvider::new(vec![Step::working(), Step::text("done"), Step::idle()])
        .then_session(vec![Step::working(), Step::text("answering"), Step::Hang]);
    let s = sub_agent_with(provider, false);
    let author = s.parent_author.clone();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap(); rig.until_finished().await;
    rig.commands.send(steer("follow up")).unwrap();
    until_text(&mut rig).await;
    rig.commands.send(steer("again")).unwrap();
    loop { if matches!(rig.next().await, Update::Error(e) if e.contains("A turn is already running")) { break; } }
    rig.commands.send(Command::Cancel).unwrap();
    rig.until_finished().await;
    assert_eq!(rig.messages().iter().filter(|m| m.role == Role::User).count(), 2, "the brief and the follow-up");
    rig.task.abort();
}

#[tokio::test]
async fn stopping_a_sub_agent_cancels_it_and_a_timeout_that_was_set_first_stays() {
    use discard_core::model::ThreadState;
    for (preset, expected) in [(None, ThreadState::Cancelled), (Some(ThreadState::TimedOut), ThreadState::TimedOut)] {
        let s = sub_agent(vec![Step::working(), Step::text("partial"), Step::Hang]);
        let (thread, author) = (s.thread.clone(), s.parent_author.clone());
        let mut rig = launch_workspace(s.engine, s.probe);
        rig.commands.send(brief("go", &author)).unwrap();
        loop { if matches!(rig.next().await, Update::Event(e) if matches!(e.kind, EventKind::TextDelta { .. })) { break; } }
        if let Some(state) = preset { rig.store.lock().unwrap().finish_sub_agent_thread(&thread, state).unwrap(); }
        rig.commands.send(Command::Cancel).unwrap();
        let updates = rig.until_finished().await;
        assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Cancelled))));
        assert!(updates.contains(&Update::SubAgentFinished { state: expected, text: "partial".into() }), "{expected:?}: {updates:?}");
        assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(expected));
        assert_eq!(rig.messages().len(), 2, "the partial transcript stays");
        rig.task.abort();
    }
}

#[tokio::test]
async fn a_sub_agent_that_cannot_start_ends_failed() {
    use discard_core::model::ThreadState;
    let s = sub_agent(vec![Step::idle()]);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut engine = s.engine;
    engine.provider = Arc::new(FakeProvider::new(vec![Step::idle()]).with_setup(SetupStatus::NotInstalled { install_hint: "Install it.".into() }));
    let mut rig = launch_workspace(engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    let mut updates = Vec::new();
    loop { let u = rig.next().await; let done = matches!(u, Update::SubAgentFinished { .. }); updates.push(u); if done { break; } }
    assert!(updates.iter().any(|u| matches!(u, Update::SetupNeeded(_))));
    assert_eq!(updates.last(), Some(&Update::SubAgentFinished { state: ThreadState::Failed, text: String::new() }));
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::Failed));
    rig.task.abort();
}

#[tokio::test]
async fn a_thread_prompt_describes_the_parents_worktree_and_the_session_stays_read_only() {
    use discard_core::provider::SessionMode;
    use discard_core::workspace_session::{ReadOnlyReason, SessionProfile};
    for kind in ["question", "sub_agent"] {
        let (dir, mut rig, command, probe) = if kind == "question" {
            let t = threaded(SessionMode::Conversation, false);
            let probe = t.thread_probe.clone();
            (t._dir, launch_workspace(t.thread, t.thread_probe), Command::Send("what does the README say?".into()), probe)
        } else {
            let s = sub_agent(vec![Step::working(), Step::text("answer"), Step::idle()]);
            let command = brief("what does the README say?", &s.parent_author);
            let probe = s.probe.clone();
            (s.t._dir, launch_workspace(s.engine, s.probe), command, probe)
        };
        // The parent is bound to `worktree`, which has an edit that is not in the project folder `repo`.
        std::fs::write(dir.path().join("worktree/README.md"), "Worktree edit that is not committed").unwrap();
        std::fs::write(dir.path().join("repo/README.md"), "Main checkout text").unwrap();
        rig.commands.send(command).unwrap();
        let updates = rig.until_finished().await;
        assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Completed))), "{kind}: {updates:?}");
        let sent = probe.sent.lock().unwrap();
        assert!(sent[0].contains("Worktree edit that is not committed") && !sent[0].contains("Main checkout text"), "{kind}: the thread reads the parent's worktree: {}", sent[0]);
        assert!(sent[0].contains("worktree of the parent channel"), "{kind}: {}", sent[0]);
        // Still read-only: Conversation mode, no workspace grant, and the profile says why.
        assert!(updates.iter().any(|u| matches!(u, Update::SessionProfile(SessionProfile::ReadOnly { reason: Some(ReadOnlyReason::Thread) }))), "{kind}");
        assert!(!updates.iter().any(|u| matches!(u, Update::SessionProfile(SessionProfile::Workspace { .. }))), "{kind}");
        let specs = probe.specs.lock().unwrap();
        assert_eq!(specs[0].mode, SessionMode::Conversation, "{kind}");
        assert!(!specs[0].sandbox_autorun && specs[0].resume_handle.is_none(), "{kind}");
        assert!(rig.store.lock().unwrap().worktree_for(&rig.channel).unwrap().is_none(), "{kind}: the thread has no binding of its own");
        drop(specs); drop(sent);
        rig.task.abort();
    }
}

#[tokio::test]
async fn the_result_is_every_text_block_of_the_final_message_not_only_the_tail() {
    use discard_core::model::ThreadState;
    // The sub-agent writes its findings, saves a memory, and then writes "Saved.".
    let s = sub_agent(vec![
        Step::working(), Step::text("Callers: a, b."),
        Step::Emit(EventKind::ToolCallStart { call_id: "m1".into(), name: "memory_save".into(), input: serde_json::json!({"text": "x"}) }),
        Step::Emit(EventKind::ToolCallResult { call_id: "m1".into(), output: "ok".into(), is_error: false }),
        Step::text("Saved."), Step::idle(),
    ]);
    let author = s.parent_author.clone();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("find the callers", &author)).unwrap();
    let updates = rig.until_finished().await;
    let finished = updates.iter().find_map(|u| match u { Update::SubAgentFinished { state, text } => Some((*state, text.clone())), _ => None }).expect("SubAgentFinished");
    assert_eq!(finished.0, ThreadState::Done);
    assert!(finished.1.contains("Callers: a, b.") && finished.1.contains("Saved."), "{:?}", finished.1);
    rig.task.abort();
}

/// Break the kind of a thread row with a second connection, so that the store cannot read the thread any more.
fn corrupt_thread(dir: &std::path::Path, thread: &ChannelId) {
    let conn = rusqlite::Connection::open(dir.join("store.db")).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=ON").unwrap();
    conn.execute("UPDATE channels SET thread_kind='broken' WHERE id=?1", [&thread.0]).unwrap();
}

#[tokio::test]
async fn a_sub_agent_whose_thread_row_cannot_be_read_still_ends_failed_with_the_error() {
    use discard_core::model::ThreadState;
    // The row is broken before the brief arrives: the turn cannot even start.
    let s = sub_agent_in(vec![Step::working(), Step::text("never"), Step::idle()], true);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    corrupt_thread(s.t._dir.path(), &thread);
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    let updates = rig.until_finished().await;
    let finished = updates.iter().find_map(|u| match u { Update::SubAgentFinished { state, text } => Some((*state, text.clone())), _ => None });
    assert!(matches!(&finished, Some((ThreadState::Failed, text)) if text.contains("thread_kind")), "the task is told it failed, with the error: {updates:?}");
    assert!(updates.iter().any(|u| matches!(u, Update::Error(e) if e.contains("thread_kind"))));
    assert!(matches!(updates.last(), Some(Update::TurnFinished(TurnOutcome::Failed))));
    rig.task.abort();

    // The row breaks during the run: the end of the run cannot read it either.
    let s = sub_agent_in(vec![Step::working(), Step::text("partial"), Step::Hang], true);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let dir = s.t._dir.path().to_path_buf();
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    loop { if matches!(rig.next().await, Update::Event(e) if matches!(e.kind, EventKind::TextDelta { .. })) { break; } }
    corrupt_thread(&dir, &thread);
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    let finished = updates.iter().find_map(|u| match u { Update::SubAgentFinished { state, text } => Some((*state, text.clone())), _ => None });
    assert!(matches!(&finished, Some((ThreadState::Failed, text)) if text.contains("thread_kind")), "{updates:?}");
    rig.task.abort();
}

#[tokio::test]
async fn delegation_tools_go_to_conversation_and_workspace_responses_of_agents_with_the_setting() {
    use discard_core::provider::SessionMode;
    let t = threaded(SessionMode::Conversation, false);
    let spy = Arc::new(DelegationSpy::default());
    let enable = |engine: &Engine, on: bool| { let store = engine.store.lock().unwrap(); let mut config = store.load_agent(&engine.agent_id).unwrap(); config.delegation = on; store.update_agent(&engine.agent_id, &config).unwrap(); };
    let respond = |mut engine: Engine, mode: SessionMode| { engine.memory = Some(spy.clone()); engine.mode = mode; engine.workspace_mode = WorkspaceMode::MainCheckout; engine.provider = Arc::new(FakeProvider::new(vec![Step::working(), Step::idle()])); engine };
    let run = |engine: Engine, command: Command| async move { let probe = Probe::default(); let mut rig = launch_workspace(engine, probe); rig.commands.send(command).unwrap(); rig.until_finished().await; rig.task.abort(); };
    run(respond(t.parent.clone(), SessionMode::Conversation), Command::Send("a".into())).await;
    enable(&t.parent, true);
    run(respond(t.parent.clone(), SessionMode::Conversation), Command::Send("b".into())).await;
    run(respond(t.parent.clone(), SessionMode::Coding), Command::Send("c".into())).await;
    // The agent has the setting, and the thread still gets no tools: a question thread, whatever the process mode.
    let question = respond(t.thread.clone(), SessionMode::Conversation);
    enable(&question, true);
    run(question, Command::Send("d".into())).await;
    assert_eq!(spy.0.lock().unwrap().as_slice(), [false, true, false, false], "off, on, Coding, question thread");
}

#[tokio::test]
async fn the_parent_response_anchors_the_sub_agent_threads_of_its_turn() {
    use discard_core::model::ThreadKind;
    // A read-only parent saves its reply; the thread hangs under it.
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::text("answer"), Step::Hang]), false);
    let (parent, agent) = (engine.channel_id.clone(), engine.agent_id.clone());
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("go".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(e) if matches!(e.kind, EventKind::TextDelta { .. })) { break; } }
    let thread = rig.store.lock().unwrap().create_sub_agent_thread(&parent, "work", &agent).unwrap().id;
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    let response = rig.messages().into_iter().find(|m| m.role == Role::Agent).expect("the reply is saved").id;
    let anchored = updates.iter().find_map(|u| match u { Update::ThreadsAnchored(threads) => Some(threads.clone()), _ => None }).expect("ThreadsAnchored");
    assert_eq!(anchored.len(), 1);
    assert_eq!((anchored[0].channel_id.clone(), anchored[0].kind, anchored[0].anchor_message_id.clone()), (thread.clone(), ThreadKind::SubAgent, Some(response)));
    let at = updates.iter().position(|u| matches!(u, Update::ThreadsAnchored(_))).unwrap();
    assert!(updates[..at].iter().any(|u| matches!(u, Update::ResponseCompleted(_))), "the anchor follows the saved response");
    rig.task.abort();
}

#[tokio::test]
async fn a_turn_with_no_response_anchors_its_sub_agent_threads_to_the_user_message() {
    let (_dir, engine, probe) = workspace_engine(FakeProvider::new(vec![Step::working(), Step::Hang]), false);
    let (parent, agent) = (engine.channel_id.clone(), engine.agent_id.clone());
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Send("go".into())).unwrap();
    loop { if matches!(rig.next().await, Update::Event(_)) { break; } }
    let thread = rig.store.lock().unwrap().create_sub_agent_thread(&parent, "work", &agent).unwrap().id;
    rig.commands.send(Command::Cancel).unwrap();
    let updates = rig.until_finished().await;
    let messages = rig.messages();
    assert_eq!(messages.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User], "no response was saved");
    assert!(updates.iter().any(|u| matches!(u, Update::ThreadsAnchored(t) if t.len() == 1 && t[0].anchor_message_id.as_ref() == Some(&messages[0].id))), "{updates:?}");
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().anchor_message_id, Some(messages[0].id.clone()));
    rig.task.abort();
}

#[tokio::test]
async fn a_pending_approval_follows_a_thread() {
    let t = threaded(discard_core::provider::SessionMode::Conversation, false);
    let (agent, thread) = (t.parent.agent_id.clone(), t.thread.channel_id.clone());
    t.thread.store.lock().unwrap().set_thread_followed(&thread, false).unwrap();
    let mut engine = t.thread;
    let provider = FakeProvider::new(vec![Step::working(), bash(), Step::text("done"), Step::idle()]);
    let probe = provider.probe.clone();
    engine.provider = Arc::new(provider);
    let mut rig = launch_workspace(engine, probe);
    rig.commands.send(Command::Continue { agent, note: "Approved.".into() }).unwrap();
    let mut seen = Vec::new();
    let approval_id = loop {
        let u = rig.next().await;
        seen.push(u.clone());
        if let Update::Event(e) = u && let EventKind::ApprovalRequest { approval_id, .. } = e.kind { break approval_id; }
    };
    assert!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed, "followed as soon as the approval is pending");
    assert!(seen.iter().any(|u| matches!(u, Update::ThreadFollowed)));
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Approve }).unwrap();
    rig.until_finished().await;
    rig.task.abort();
}

#[tokio::test]
async fn an_agent_message_with_at_you_in_plain_text_follows_a_thread_of_either_kind() {
    for (text, followed) in [("Done. @you decide.", true), ("Ping @YOU.", true), ("`@you` in code", false), ("> @you quoted", false), ("```\n@you\n```", false), ("mail@you.com", false), ("no mention", false)] {
        // A question thread; Continue never follows by itself.
        let t = threaded(discard_core::provider::SessionMode::Conversation, false);
        let (agent, thread) = (t.parent.agent_id.clone(), t.thread.channel_id.clone());
        t.thread.store.lock().unwrap().set_thread_followed(&thread, false).unwrap();
        let mut engine = t.thread;
        let provider = FakeProvider::new(vec![Step::working(), Step::text(text), Step::idle()]);
        let probe = provider.probe.clone();
        engine.provider = Arc::new(provider);
        let mut rig = launch_workspace(engine, probe);
        rig.commands.send(Command::Continue { agent, note: "Approved.".into() }).unwrap();
        let updates = rig.until_finished().await;
        assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed, followed, "question thread: {text:?}");
        assert_eq!(updates.iter().any(|u| matches!(u, Update::ThreadFollowed)), followed, "{text:?}");
        rig.task.abort();
        // A sub-agent thread.
        let s = sub_agent(vec![Step::working(), Step::text(text), Step::idle()]);
        let (thread, author) = (s.thread.clone(), s.parent_author.clone());
        let mut rig = launch_workspace(s.engine, s.probe);
        rig.commands.send(brief("go", &author)).unwrap();
        rig.until_finished().await;
        assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().followed, followed, "sub-agent thread: {text:?}");
        rig.task.abort();
    }
}

#[tokio::test]
async fn a_sub_agent_thread_that_asks_for_approval_is_followed_and_still_running() {
    use discard_core::model::ThreadState;
    let s = sub_agent(vec![Step::working(), bash(), Step::text("done"), Step::idle()]);
    let (thread, author) = (s.thread.clone(), s.parent_author.clone());
    let mut rig = launch_workspace(s.engine, s.probe);
    rig.commands.send(brief("go", &author)).unwrap();
    let approval_id = loop {
        if let Update::Event(e) = rig.next().await && let EventKind::ApprovalRequest { approval_id, .. } = e.kind { break approval_id; }
    };
    let info = rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap();
    assert_eq!((info.followed, info.state), (true, Some(ThreadState::Running)));
    rig.commands.send(Command::Decide { rule_cut: None, approval_id, decision: Decision::Approve }).unwrap();
    rig.until_finished().await;
    assert_eq!(rig.store.lock().unwrap().thread_info(&thread).unwrap().unwrap().state, Some(ThreadState::Done));
    rig.task.abort();
}
