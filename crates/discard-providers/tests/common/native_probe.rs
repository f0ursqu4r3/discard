//! Shared body of the opt-in native-resume live probes (Claude and Codex).
//!
//! Three turns in one channel with an agent that has `native_resume` on. A new non-mandatory memory
//! item arrives between turns 2 and 3. Turn 3 resumes the provider session, and the answer must use
//! the new item.

use discard_core::agent::AgentConfig;
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::{Engine, MemoryService};
use discard_core::event::{EventKind, SessionId};
use discard_core::memory::{MemoryStore, Scope};
use discard_core::memory_items::{ItemKind, MutationOrigin};
use discard_core::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use discard_core::provider::Provider;
use discard_core::store::{AgentSession, Store};
use discard_core::ui_state::{Command, ResumeState, Update};
use discard_mcp::MemoryRuntime;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const SECRET: &str = "4417";

/// What one turn produced.
pub struct ProbeTurn {
    pub outcome: TurnOutcome,
    pub reply: String,
    pub notices: Vec<String>,
    pub resume_states: Vec<ResumeState>,
}

pub struct ProbeResult {
    pub turns: Vec<ProbeTurn>,
    /// The saved row after each turn.
    pub rows: Vec<Option<AgentSession>>,
}

async fn turn(commands: &mpsc::UnboundedSender<Command>, updates: &mut mpsc::UnboundedReceiver<Update>, text: &str) -> ProbeTurn {
    commands.send(Command::Send(text.into())).unwrap();
    let (mut reply, mut notices, mut resume_states) = (String::new(), Vec::new(), Vec::new());
    loop {
        let update = tokio::time::timeout(Duration::from_secs(180), updates.recv()).await.expect("an update within 180s").expect("engine alive");
        match update {
            Update::Event(e) => {
                if let EventKind::TextDelta { text } = &e.kind {
                    reply.push_str(text);
                }
            }
            Update::Notice(n) => notices.push(n),
            Update::ResumeState(s) => resume_states.push(s),
            Update::TurnFinished(outcome) => return ProbeTurn { outcome, reply, notices, resume_states },
            Update::Error(m) | Update::ResponseSaveFailed(m) | Update::SetupNeeded(m) => panic!("engine problem: {m}"),
            _ => {}
        }
    }
}

/// Runs the three-turn probe. `provider_name` is the agent's provider label; `model` is the explicit model.
pub async fn run(provider: Arc<dyn Provider>, provider_name: &str, model: &str) -> ProbeResult {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();

    let db = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: project_dir.to_string_lossy().into() };
    db.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    db.insert_agent(
        &agent_id,
        &AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: true, delegation: false, reasoning_effort: None, name: "Scout".into(), avatar: None, provider: provider_name.into(), model: Some(model.into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false },
    )
    .unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "c".into(), agent_id: agent_id.clone(), workspace_mode: WorkspaceMode::MainCheckout };
    db.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    db.create_session(&session, &channel.id).unwrap();

    let memory_store = Arc::new(MemoryStore::open(&dir.path().join("memory"), &dir.path().join("index.db")).unwrap());
    let runtime = MemoryRuntime::start(Arc::clone(&memory_store), &dir.path().join("run")).await.unwrap();
    let memory: Arc<dyn MemoryService> = runtime.clone();
    let store = Arc::new(Mutex::new(db));

    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel),
        autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Conversation,
        providers: vec![provider.clone()],
        store: store.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project.id.clone()),
        memory: Some(memory),
        plugin_write: false,
        workspace_mode: WorkspaceMode::MainCheckout,
        workspace: None,
        observer: None,
        channel_name: "c".into(),
        provider,
        channel_id: channel.id.clone(),
        session_id: session,
        cwd: project_dir,
        model: Some(model.into()),
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, mut updates) = mpsc::unbounded_channel();
    tokio::spawn(engine.run(commands_rx, updates_tx));

    let row = || store.lock().unwrap().agent_session(&channel.id, &agent_id).unwrap();
    let mut turns = Vec::new();
    let mut rows = Vec::new();

    turns.push(turn(&commands, &mut updates, "Do not use any tools. Reply with the single word one.").await);
    rows.push(row());
    turns.push(turn(&commands, &mut updates, "Do not use any tools. Reply with the single word two.").await);
    rows.push(row());

    // A new fact. It is not mandatory, so it reaches the resumed session in the context update.
    memory_store
        .create_item(&Scope::Agent(agent_id.clone()), ItemKind::Fact, &format!("The user's lucky number is {SECRET}."), None, MutationOrigin::User)
        .unwrap();
    turns.push(turn(&commands, &mut updates, "Do not use any tools. What is the user's lucky number? Answer with the number only.").await);
    rows.push(row());

    drop(commands);
    runtime.shutdown().await;
    ProbeResult { turns, rows }
}

/// Assertions shared by both providers.
pub fn assert_resumed(result: &ProbeResult) {
    let t = &result.turns;
    for (i, t) in t.iter().enumerate() {
        assert_eq!(t.outcome, TurnOutcome::Completed, "turn {} reply: {:?}", i + 1, t.reply);
        assert!(!t.notices.iter().any(|n| n.starts_with("Fresh replay")), "turn {} showed a fresh replay: {:?}", i + 1, t.notices);
    }
    assert_eq!(t[0].resume_states, vec![], "turn 1 starts a new session");
    assert_eq!(t[1].resume_states, vec![ResumeState::Resumed], "turn 2 must resume the saved session");
    assert_eq!(t[2].resume_states, vec![ResumeState::Resumed], "turn 3 must resume the saved session");
    let first = result.rows[0].as_ref().expect("a saved row after turn 1");
    let last = result.rows[2].as_ref().expect("a saved row after turn 3");
    assert!(!first.handle.is_empty() && !last.handle.is_empty());
    assert_eq!(last.dropped_reason, None);
    println!("handle after turn 1 == handle after turn 3: {}", first.handle == last.handle);
    assert!(t[2].reply.contains(SECRET), "turn 3 must use the new memory item: {:?}", t[2].reply);
    assert!(last.delivered_memory.len() == 1, "the new item is recorded as delivered: {:?}", last.delivered_memory);
    println!("replies: {:?}", t.iter().map(|t| t.reply.as_str()).collect::<Vec<_>>());
}
