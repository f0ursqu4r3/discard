//! Engine tests: UI commands in, UI updates out, history persisted. Uses the fake provider.

use discard_core::agent::AgentConfig;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::Engine;
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
    let store = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
    store.insert_project(&project).unwrap();
    let agent = AgentId::new();
    store
        .insert_agent(
            &agent,
            &AgentConfig { name: "A".into(), avatar: None, provider: "fake".into(), model: None, system_prompt: String::new(), policy },
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
