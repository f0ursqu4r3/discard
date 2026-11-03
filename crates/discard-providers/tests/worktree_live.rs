//! Live check: a real agent works in its channel's git worktree, not in the main checkout.
//! Run with: cargo test -p discard-providers --test worktree_live -- --ignored --nocapture

use discard_core::agent::AgentConfig;
use discard_core::approval::Decision;
use discard_core::capability::ApprovalPolicy;
use discard_core::driver::TurnOutcome;
use discard_core::engine::Engine;
use discard_core::event::{EventKind, SessionId};
use discard_core::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use discard_core::provider::Provider;
use discard_core::store::Store;
use discard_core::ui_state::{Command, Update};
use discard_core::workspace::{ArchiveOutcome, GitWorkspace};
use discard_providers::claude::ClaudeProvider;
use discard_providers::codex::CodexProvider;
use std::path::Path;
use std::process::Command as Proc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) {
    let ok = Proc::new("git").args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).current_dir(dir).status().unwrap().success();
    assert!(ok, "git {args:?}");
}

async fn run(provider: Arc<dyn Provider>, model: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let db = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "repo".into(), path: repo.to_string_lossy().into() };
    db.insert_project(&project).unwrap();
    let agent_id = AgentId::new();
    db.insert_agent(&agent_id, &AgentConfig { name: "A".into(), avatar: None, provider: "any".into(), model: model.map(str::to_string), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: project.id.clone(), name: "general".into(), agent_id: agent_id.clone(), workspace_mode: WorkspaceMode::Worktree };
    db.insert_channel(&channel).unwrap();
    let session = SessionId::new();
    db.create_session(&session, &channel.id).unwrap();
    let store = Arc::new(Mutex::new(db));
    let workspace = Arc::new(GitWorkspace::new(&dir.path().join("worktrees"), Arc::clone(&store)));

    let engine = Engine {
        store: Arc::clone(&store),
        agent_id,
        project_id: project.id.clone(),
        memory: None,
        plugin_write: false,
        workspace_mode: WorkspaceMode::Worktree,
        workspace: Some(workspace.clone()),
        observer: None,
        channel_name: "general".into(),
        provider,
        channel_id: channel.id.clone(),
        session_id: session,
        cwd: repo.clone(),
        model: model.map(str::to_string),
        system_prompt: None,
        policy: ApprovalPolicy::Ask,
    };
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (updates_tx, mut updates) = mpsc::unbounded_channel();
    tokio::spawn(engine.run(commands_rx, updates_tx));
    commands.send(Command::Send("Run the shell command `touch from-agent.txt` and then say done.".into())).unwrap();
    let outcome = loop {
        let update = tokio::time::timeout(Duration::from_secs(120), updates.recv()).await.expect("an update within 120s").expect("engine alive");
        match update {
            Update::Event(e) => {
                if let EventKind::ApprovalRequest { approval_id, .. } = e.kind {
                    commands.send(Command::Decide { approval_id, decision: Decision::Approve }).unwrap();
                }
            }
            Update::TurnFinished(outcome) => break outcome,
            Update::Error(m) | Update::SetupNeeded(m) => panic!("engine problem: {m}"),
            _ => {}
        }
    };
    assert_eq!(outcome, TurnOutcome::Completed);

    let record = store.lock().unwrap().worktree_for(&channel.id).unwrap().expect("a worktree was created");
    assert!(Path::new(&record.path).join("from-agent.txt").exists(), "the agent worked in the worktree");
    assert!(!repo.join("from-agent.txt").exists(), "the main checkout must be untouched");

    // The agent's file is not in git yet, so archiving asks first and loses nothing.
    let ArchiveOutcome::NeedsConfirmation(items) = workspace.archive(&channel.id, false).unwrap() else { panic!("expected a confirmation request") };
    assert!(items.iter().any(|i| i.contains("from-agent.txt")), "{items:?}");
    assert!(Path::new(&record.path).join("from-agent.txt").exists());
}

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_works_in_its_worktree() {
    run(Arc::new(ClaudeProvider::new("claude")), Some("haiku")).await;
}

#[tokio::test]
#[ignore = "uses the real codex CLI and the logged-in subscription"]
async fn real_codex_works_in_its_worktree() {
    run(Arc::new(CodexProvider::new("codex")), None).await;
}
