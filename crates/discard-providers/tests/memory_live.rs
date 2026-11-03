//! Live check: the real `claude` CLI uses Discard's memory tools over the real MCP server.
//! Run with: cargo test -p discard-providers --test memory_live -- --ignored --nocapture

use discard_core::agent::AgentConfig;
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::{Engine, MemoryService};
use discard_core::event::{EventKind, SessionId};
use discard_core::memory::{MemoryStore, Scope};
use discard_core::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use discard_core::store::Store;
use discard_core::ui_state::{Command, Update};
use discard_mcp::MemoryRuntime;
use discard_providers::claude::ClaudeProvider;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

async fn turn(commands: &mpsc::UnboundedSender<Command>, updates: &mut mpsc::UnboundedReceiver<Update>, text: &str) -> (TurnOutcome, String, Vec<EventKind>) {
    commands.send(Command::Send(text.into())).unwrap();
    let (mut reply, mut kinds) = (String::new(), Vec::new());
    loop {
        let update = tokio::time::timeout(Duration::from_secs(120), updates.recv()).await.expect("an update within 120s").expect("engine alive");
        match update {
            Update::Event(e) => {
                if let EventKind::TextDelta { text } = &e.kind {
                    reply.push_str(text);
                }
                kinds.push(e.kind);
            }
            Update::TurnFinished(outcome) => return (outcome, reply, kinds),
            Update::Error(m) | Update::SetupNeeded(m) => panic!("engine problem: {m}"),
            Update::ApprovalState { .. } => {}
        }
    }
}

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_saves_and_recalls_memory_through_discard() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();

    let db = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: project_dir.to_string_lossy().into() };
    db.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    db.insert_agent(&agent_id, &AgentConfig { name: "Scout".into(), avatar: None, provider: "claude".into(), model: Some("haiku".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: project.id.clone(), name: "c".into(), agent_id: agent_id.clone(), workspace_mode: WorkspaceMode::MainCheckout };
    db.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    db.create_session(&session, &channel.id).unwrap();

    let memory_store = Arc::new(MemoryStore::open(&dir.path().join("memory"), &dir.path().join("index.db")).unwrap());
    let runtime = MemoryRuntime::start(Arc::clone(&memory_store), &dir.path().join("run")).await.unwrap();
    let memory: Arc<dyn MemoryService> = runtime.clone();

    let engine = Engine {
        store: Arc::new(Mutex::new(db)),
        agent_id: agent_id.clone(),
        project_id: project.id.clone(),
        memory: Some(memory),
        provider: Arc::new(ClaudeProvider::new("claude")),
        channel_id: channel.id.clone(),
        session_id: session,
        cwd: project_dir,
        model: Some("haiku".into()),
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, mut updates) = mpsc::unbounded_channel();
    tokio::spawn(engine.run(commands_rx, updates_tx));

    // Turn 1: save a core memory with the tool. No user approval is given: Discard approves its own tools.
    let (outcome, _, kinds) = turn(
        &commands,
        &mut updates,
        "Use the memory_append tool exactly once. scope: agent. path: core/human.md. text: The user's favorite drink is oolong tea. Then reply with the single word done.",
    )
    .await;
    assert_eq!(outcome, TurnOutcome::Completed, "{kinds:#?}");
    assert!(
        kinds.iter().any(|k| matches!(k, EventKind::ToolCallStart { name, .. } if name.contains("memory_append"))),
        "Claude must have called the memory tool: {kinds:#?}"
    );
    let saved = memory_store.read(&Scope::Agent(agent_id.clone()), "core/human.md").expect("the memory file exists").text;
    assert!(saved.to_lowercase().contains("oolong"), "{saved}");

    // Turn 2: the same conversation resumes. The core block must be visible without any tool call.
    let (outcome, reply, kinds) = turn(
        &commands,
        &mut updates,
        "Do not use any tools. Based only on what you already know from your memory blocks, what is the user's favorite drink? Answer in one short sentence.",
    )
    .await;
    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(reply.to_lowercase().contains("oolong"), "reply was: {reply:?}\n{kinds:#?}");
    runtime.shutdown().await;
}
