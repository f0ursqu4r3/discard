//! Live check: a real agent writes a plugin through Discard's MCP tools, the user approves it, and it goes live.
//! Run with: cargo test -p discard-providers --test plugin_live -- --ignored --nocapture

use discard_core::agent::AgentConfig;
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::{Engine, MemoryService};
use discard_core::event::{EventKind, SessionId};
use discard_core::memory::MemoryStore;
use discard_core::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use discard_core::provider::Provider;
use discard_core::store::Store;
use discard_core::ui_state::{Command, Update};
use discard_lua::keystore::StaticKeyStore;
use discard_lua::ledger::Ledger;
use discard_lua::{PluginLimits, PluginManager, PluginService};
use discard_mcp::MemoryRuntime;
use discard_providers::claude::ClaudeProvider;
use discard_providers::codex::CodexProvider;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const REQUEST: &str = "Use the Discard plugin tools to build a plugin named `tea-header`. \
It must replace the `channel_header` slot with a row that shows the text `Tea time` in bold, followed by the channel name. \
Steps: call plugin_api_docs once, then plugin_create, then plugin_test, then plugin_reload to submit it for review. \
Do not use any other tools. When it is submitted, reply with the single word submitted.";

async fn build_plugin(provider: Arc<dyn Provider>, model: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();

    let db = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "p".into(), path: project_dir.to_string_lossy().into() };
    db.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    db.insert_agent(&agent_id, &AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Builder".into(), avatar: None, provider: "any".into(), model: model.map(str::to_string), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: true }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "builder".into(), agent_id: agent_id.clone(), workspace_mode: WorkspaceMode::MainCheckout };
    db.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    db.create_session(&session, &channel.id).unwrap();

    let memory_store = Arc::new(MemoryStore::open(&dir.path().join("memory"), &dir.path().join("index.db")).unwrap());
    let ledger = Ledger::open(&dir.path().join("plugins.db"), Arc::new(StaticKeyStore([4; 32]))).unwrap();
    let manager = Arc::new(Mutex::new(PluginManager::new(PluginLimits::default())));
    let plugins = Arc::new(PluginService::new(dir.path(), ledger, Arc::clone(&manager), false).unwrap());
    let runtime = MemoryRuntime::start_with_plugins(Arc::clone(&memory_store), &dir.path().join("run"), Some(Arc::clone(&plugins))).await.unwrap();
    let memory: Arc<dyn MemoryService> = runtime.clone();

    let engine = Engine {
        main_checkout_grants: Arc::default(),
        workspace_locks: Arc::default(),
        write_access: Err(discard_core::workspace_session::ReadOnlyReason::NotProjectChannel), autorun: discard_core::workspace_session::AutorunState::Off,
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding, providers: vec![],
        store: Arc::new(Mutex::new(db)),
        agent_id,
        project_id: Some(project.id.clone()),
        memory: Some(memory),
        plugin_write: true,
        workspace_mode: discard_core::model::WorkspaceMode::MainCheckout,
        workspace: None,
        observer: None,
        channel_name: "c".into(),
        provider,
        channel_id: channel.id.clone(),
        session_id: session,
        cwd: project_dir,
        model: model.map(str::to_string),
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, mut updates) = mpsc::unbounded_channel();
    tokio::spawn(engine.run(commands_rx, updates_tx));
    commands.send(Command::Send(REQUEST.into())).unwrap();

    let mut tools = Vec::new();
    let outcome = loop {
        let update = tokio::time::timeout(Duration::from_secs(240), updates.recv()).await.expect("an update within 240s").expect("engine alive");
        match update {
            Update::Event(e) => {
                if let EventKind::ToolCallStart { name, .. } = &e.kind {
                    tools.push(name.clone());
                }
            }
            Update::TurnFinished(outcome) => break outcome,
            Update::Error(m) | Update::ResponseSaveFailed(m) | Update::SetupNeeded(m) => panic!("engine problem: {m}"),
            Update::SessionProfile(_) | Update::Responding(_) | Update::ApprovalState { .. } | Update::Notice(_) | Update::ResponseCompleted(_) | Update::ConversationMarker(_) | Update::MemoryChanges { .. } | Update::MessagePosted { .. } | Update::UserMessageSaved(_) => {}
        }
    };
    assert_eq!(outcome, TurnOutcome::Completed, "tools used: {tools:?}");
    println!("tools used: {tools:?}");

    // The agent submitted a review. Nothing is live yet.
    let pending = plugins.pending();
    assert_eq!(pending.len(), 1, "tools used: {tools:?}");
    let review = &pending[0];
    assert_eq!(review.name, "tea-header");
    assert!(manager.lock().unwrap().names().is_empty(), "nothing may run before the user approves");
    println!("capabilities: {:?}\nDIFF:\n{}", review.capability_lines, review.diff);
    assert!(review.manifest.capabilities.is_ui_only(), "the agent should only ask for ui: {:?}", review.capability_lines);

    // The user approves. The plugin goes live and draws the header.
    plugins.approve(&review.id, false).unwrap();
    let rendered = manager.lock().unwrap().render_slot("channel_header", &json!({"channel": "general", "pending_approvals": 0, "project": "p", "turn_running": false, "agent": {"name": "x", "provider": "y", "status": "idle"}}));
    let tree = serde_json::to_string(&rendered.expect("the approved plugin renders the header").element).unwrap();
    assert!(tree.contains("Tea time"), "{tree}");
    runtime.shutdown().await;
}

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_builds_a_plugin_that_the_user_approves_and_runs() {
    build_plugin(Arc::new(ClaudeProvider::new("claude")), Some("sonnet")).await;
}

#[tokio::test]
#[ignore = "uses the real codex CLI and the logged-in subscription"]
async fn real_codex_builds_a_plugin_that_the_user_approves_and_runs() {
    build_plugin(Arc::new(CodexProvider::new("codex")), None).await;
}
