//! Engine tests: UI commands in, UI updates out, history persisted. Uses the fake provider.

use discard_core::agent::AgentConfig;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::{Engine, MEMORY_TOOL_PREFIX, MemoryService, PreparedMemory};
use discard_core::event::{ApprovalId, EventKind, SessionId};
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
    let store = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
    store.insert_project(&project).unwrap();
    let (agent_id, project_id) = (AgentId::new(), project.id.clone());
    let agent = agent_id.clone();
    store
        .insert_agent(
            &agent,
            &AgentConfig { name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy, plugin_write: false },
        )
        .unwrap();
    let channel = Channel {
        id: ChannelId::new(),
        project_id: project.id,
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
        store: Arc::clone(&store),
        agent_id: agent_id.clone(),
        project_id: project_id.clone(),
        memory,
        plugin_write: false,
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
    prepared: Mutex<Vec<(AgentId, ProjectId, Option<String>)>>,
    finished: Mutex<Vec<String>>,
}

impl FakeMemory {
    fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self { fail, prepared: Mutex::default(), finished: Mutex::default() })
    }
}

impl MemoryService for FakeMemory {
    fn prepare(&self, agent: &AgentId, project: &ProjectId, base: Option<&str>, _plugin_write: bool) -> Result<PreparedMemory, String> {
        self.prepared.lock().unwrap().push((agent.clone(), project.clone(), base.map(str::to_string)));
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
