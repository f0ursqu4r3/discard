//! Shared headless test harness: a store, a sequencer, and a UI stand-in.
#![allow(dead_code)]

use discard_core::agent::AgentConfig;
use discard_core::approval::{ApprovalState, Decider, Decision};
use discard_core::capability::{ApprovalPolicy, EnforcementMode};
use discard_core::driver::{DecisionMsg, TurnContext, TurnInputs, TurnOutcome, run_turn};
use discard_core::event::{ApprovalId, Event, EventKind, Sequencer, SessionId};
use discard_core::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use discard_core::provider::{Provider, SessionSpec};
use discard_core::store::Store;
use discard_core::turn::{Turn, TurnState};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::mpsc;

pub struct Harness {
    pub store: Mutex<Store>,
    pub sequencer: Sequencer,
    pub session_id: SessionId,
}

pub struct Ui {
    pub updates: mpsc::UnboundedReceiver<Event>,
    pub decisions: mpsc::UnboundedSender<DecisionMsg>,
    pub cancel: mpsc::UnboundedSender<()>,
}

impl Harness {
    pub fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store
            .insert_agent(
                &agent,
                &AgentConfig {
                    workspace_write: false, sandbox_autorun: false,
                    reasoning_effort: None,
                    name: "A".into(),
                    avatar: None,
                    provider: "fake".into(),
                    model: None,
                    system_prompt: String::new(),
                    policy: ApprovalPolicy::Ask, plugin_write: false,
                },
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
        let session_id = SessionId::new();
        store.create_session(&session_id, &channel.id).unwrap();
        Self { store: Mutex::new(store), sequencer: Sequencer::new(), session_id }
    }

    pub fn turn_state(&self, t: &Turn) -> TurnState {
        self.store.lock().unwrap().turn_state(&t.id).unwrap()
    }

    pub fn approval_state(&self, id: &ApprovalId) -> ApprovalState {
        self.store.lock().unwrap().load_approval(id).unwrap().state
    }

    pub fn audit_len(&self, id: &ApprovalId) -> usize {
        self.store.lock().unwrap().load_approval(id).unwrap().audit.len()
    }
}

pub async fn drive<F, Fut>(h: &mut Harness, p: &dyn Provider, ui: F) -> (Turn, TurnOutcome)
where
    F: FnOnce(Ui) -> Fut,
    Fut: Future<Output = ()>,
{
    let spec = SessionSpec {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding,
        cwd: PathBuf::from("."),
        model: None,
        system_prompt: None,
        enforcement: EnforcementMode::HostPrompts,
        resume_handle: None,
        mcp_config: None,
    };
    drive_with(h, p, spec, "hello", ui).await
}

pub async fn drive_with<F, Fut>(
    h: &mut Harness,
    p: &dyn Provider,
    spec: SessionSpec,
    text: &str,
    ui: F,
) -> (Turn, TurnOutcome)
where
    F: FnOnce(Ui) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut session = p.start_session(spec).await.unwrap();
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();
    let (dec_tx, dec_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
    let ctx = TurnContext {
        workspace_root: None,
        store: &h.store,
        sequencer: &mut h.sequencer,
        session_id: &h.session_id,
        policy_name: "ask",
        updates: updates_tx,
    };
    let ui_fut = ui(Ui { updates: updates_rx, decisions: dec_tx, cancel: cancel_tx });
    let (res, ()) = tokio::join!(
        run_turn(ctx, &mut *session, text, TurnInputs { decisions: dec_rx, cancel: cancel_rx }),
        ui_fut
    );
    res.unwrap()
}

/// Collect every update. Optionally react to approval requests.
pub async fn collect(mut ui: Ui, mut on_approval: impl FnMut(&ApprovalId, &Ui)) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(ev) = ui.updates.recv().await {
        if let EventKind::ApprovalRequest { approval_id, .. } = &ev.kind {
            on_approval(approval_id, &ui);
        }
        seen.push(ev);
    }
    seen
}

pub fn decide(ui: &Ui, id: &ApprovalId, decision: Decision) {
    ui.decisions
        .send(DecisionMsg { approval_id: id.clone(), decision, decider: Decider::User })
        .unwrap();
}

pub fn approval_id_of(events: &[Event]) -> ApprovalId {
    events
        .iter()
        .find_map(|e| match &e.kind {
            EventKind::ApprovalRequest { approval_id, .. } => Some(approval_id.clone()),
            _ => None,
        })
        .expect("an approval request")
}


/// Setup timeouts must kill their child. Clean up a regression before failing.
pub async fn assert_setup_process_terminated(path: &std::path::Path) {
    let pid: i32 = std::fs::read_to_string(path).expect("setup process wrote its PID").trim().parse().unwrap();
    let alive = || nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok();
    for _ in 0..50 {
        if !alive() { return; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
    panic!("timed-out setup process {pid} remained alive");
}
