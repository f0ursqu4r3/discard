//! The Claude provider against a fake `claude` executable that replays a recorded transcript.

mod common;

use common::*;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::EnforcementMode;
use discard_core::driver::TurnOutcome;
use discard_core::event::EventKind;
use discard_core::provider::{Provider, ProviderError, SessionSpec, SetupStatus};
use discard_core::turn::TurnState;
use discard_providers::claude::ClaudeProvider;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fake-claude.sh");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/claude-approval-turn.jsonl");

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }
    fn log(&self) -> PathBuf {
        self.dir.path().join("fake.log")
    }
    fn pid_file(&self) -> PathBuf {
        self.dir.path().join("child.pid")
    }
    fn provider(&self, mode: &str) -> ClaudeProvider {
        ClaudeProvider::new(FAKE)
            .with_env("FAKE_CLAUDE_MODE", mode)
            .with_env("FAKE_CLAUDE_LOG", self.log())
            .with_env("FAKE_CLAUDE_FIXTURE", FIXTURE)
            .with_env("FAKE_CLAUDE_CHILD_PID", self.pid_file())
    }
    fn log_text(&self) -> String {
        std::fs::read_to_string(self.log()).unwrap_or_default()
    }
}

fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

async fn wait_for_file(path: &Path) {
    for _ in 0..200 {
        if path.exists() && !std::fs::read_to_string(path).unwrap_or_default().trim().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{} never appeared", path.display());
}

fn spec() -> SessionSpec {
    SessionSpec {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding,
        cwd: PathBuf::from("."),
        model: None,
        system_prompt: None,
        enforcement: EnforcementMode::HostPrompts,
        resume_handle: None,
        mcp_config: None,
    }
}

#[tokio::test]
async fn setup_check_reports_a_missing_executable() {
    let p = ClaudeProvider::new("/nonexistent/claude");
    assert!(matches!(p.check_setup().await, SetupStatus::NotInstalled { .. }));
}

#[tokio::test]
async fn setup_check_reports_logged_out_with_the_login_command() {
    let rig = Rig::new();
    let p = rig.provider("approval").with_env("FAKE_CLAUDE_LOGGED_IN", "false");
    assert_eq!(p.check_setup().await, SetupStatus::NotLoggedIn { login_command: "claude auth login".into() });
}

#[tokio::test]
async fn setup_check_reports_ready_when_logged_in() {
    let rig = Rig::new();
    assert_eq!(rig.provider("approval").check_setup().await, SetupStatus::Ready);
}

#[tokio::test]
async fn approved_turn_runs_end_to_end_and_answers_with_allow() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("approval");
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Completed);
    assert_eq!(h.turn_state(&turn), TurnState::Completed);
    let id = approval_id_of(&seen);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::ToolCallStart { .. })));
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::ToolCallResult { .. })));
    let log = rig.log_text();
    assert!(log.contains("--permission-prompt-tool stdio"), "{log}");
    assert!(log.contains("\"content\":\"hello\""), "user message must reach the CLI: {log}");
    assert!(log.contains("\"behavior\":\"allow\""), "{log}");
    assert!(log.contains("\"request_id\":"), "the response must echo the request id: {log}");
}

#[tokio::test]
async fn denied_turn_answers_with_deny() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("approval");
    drive(&mut h, &p, |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::Deny)).await;
    })
    .await;
    assert!(rig.log_text().contains("\"behavior\":\"deny\""), "{}", rig.log_text());
}

#[tokio::test]
async fn always_allow_passes_the_providers_permission_suggestions_back() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("approval");
    drive(&mut h, &p, |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::AlwaysAllow)).await;
    })
    .await;
    let log = rig.log_text();
    assert!(log.contains("\"behavior\":\"allow\""), "{log}");
    assert!(log.contains("updatedPermissions"), "{log}");
}

#[tokio::test]
async fn session_exposes_the_providers_session_id_as_a_resume_handle() {
    let rig = Rig::new();
    let p = rig.provider("approval");
    let mut s = p.start_session(spec()).await.unwrap();
    assert_eq!(s.resume_handle(), None);
    s.send_user_message("hello").await.unwrap();
    s.next_event().await.expect("the init event");
    assert_eq!(s.resume_handle().as_deref(), Some("00000000-0000-4000-8000-000000000001"));
}

#[tokio::test]
async fn cancel_kills_the_whole_process_group() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("hang");
    let pid_file = rig.pid_file();
    let (turn, outcome) = drive(&mut h, &p, |mut ui| async move {
        ui.updates.recv().await.expect("init event");
        wait_for_file(&pid_file).await;
        ui.cancel.send(()).unwrap();
        while ui.updates.recv().await.is_some() {}
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(h.turn_state(&turn), TurnState::Cancelled);
    let child: i32 = std::fs::read_to_string(rig.pid_file()).unwrap().trim().parse().unwrap();
    for _ in 0..100 {
        if !pid_alive(child) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the grandchild process {child} survived cancel");
}

#[tokio::test]
async fn early_exit_surfaces_stderr_as_an_error_and_fails_the_turn() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("exit_early");
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |_, _| {}).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(h.turn_state(&turn), TurnState::Failed);
    let msg = seen
        .iter()
        .find_map(|e| match &e.kind {
            EventKind::Error { message } => Some(message.clone()),
            _ => None,
        })
        .expect("an error event");
    assert!(msg.contains("No conversation found"), "{msg}");
}

#[tokio::test]
async fn resume_handle_is_passed_to_the_cli() {
    let rig = Rig::new();
    let p = rig.provider("exit_early");
    let mut sp = spec();
    sp.resume_handle = Some("abc-123".into());
    let mut s = p.start_session(sp).await.unwrap();
    s.send_user_message("hello").await.ok();
    while s.next_event().await.is_some() {}
    assert!(rig.log_text().contains("--resume abc-123"), "{}", rig.log_text());
}

#[tokio::test]
async fn starting_fails_closed_without_interactive_approvals() {
    let rig = Rig::new();
    let mut matrix = rig.provider("approval").capabilities().clone();
    matrix.capabilities.interactive_approvals = false;
    let p = ClaudeProvider::with_matrix(FAKE, matrix).with_env("FAKE_CLAUDE_LOG", rig.log());
    let err = p.start_session(spec()).await.err().unwrap();
    assert!(matches!(err, ProviderError::Policy(_)));
    assert!(rig.log_text().is_empty(), "no process may be started");
}

#[tokio::test]
async fn spawn_failure_is_reported_not_panicked() {
    let p = ClaudeProvider::new("/nonexistent/claude");
    let err = p.start_session(spec()).await.err().unwrap();
    assert!(matches!(err, ProviderError::Spawn(_)));
}

fn conversation_spec(rig: &Rig) -> SessionSpec {
    let path = rig.dir.path().join("memory.json");
    std::fs::write(&path, serde_json::json!({"mcpServers":{"discard-memory":{"url":"http://127.0.0.1:1/mcp","headers":{"Authorization":"Bearer synthetic"}}}}).to_string()).unwrap();
    let mut s = spec();
    s.mode = discard_core::provider::SessionMode::Conversation;
    s.model = Some("selected-claude-model".into());
    s.system_prompt = Some("Fresh synthetic memory".into());
    s.mcp_config = Some(path);
    s
}

#[tokio::test]
async fn conversation_process_forces_memory_environment_and_empty_cwd() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let mut session = rig
        .provider("approval")
        .with_env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "0")
        .with_env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "0")
        .with_env("CLAUDE_CODE_PLUGIN_DIRS", "/unsafe")
        .start_session(s)
        .await
        .unwrap();
    // Wait for the fixture initialization so its argv/environment log is complete.
    let _ = session.next_event().await;
    let log = rig.log_text();
    assert!(log.contains("MEMORY_ENV 1 1 "), "{log}");
    assert!(log.contains("--model selected-claude-model"));
    assert!(log.contains("--append-system-prompt Fresh synthetic memory"));
    for flag in ["--tools", "--strict-mcp-config", "--restricted", "--disable-slash-commands", "--system-prompt-snapshot off"] {
        assert!(log.contains(flag), "{flag}");
    }
    let cwd = PathBuf::from(log.lines().find_map(|l| l.strip_prefix("CWD ")).unwrap());
    assert!(std::fs::read_dir(&cwd).unwrap().next().is_none());
    drop(session);
    assert!(!cwd.exists());
}

#[tokio::test]
async fn conversation_missing_required_control_fails_before_session() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let error = rig.provider("approval").with_env("FAKE_CLAUDE_HELP", "--tools").start_session(s).await.err().unwrap().to_string();
    assert!(error.contains("required conversation control"), "{error}");
    assert!(!rig.log_text().contains("STDIN"));
}

async fn check_conversation_setup_timeout_kills_child(argument: &str) {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let error = rig.provider("approval").with_env("FAKE_CLAUDE_HANG_SETUP", argument).start_session(s).await.err().unwrap().to_string();
    assert!(error.contains("timed out"), "{error}");
    assert_setup_process_terminated(&rig.pid_file()).await;
}

#[tokio::test]
async fn conversation_version_timeout_kills_setup_process() {
    check_conversation_setup_timeout_kills_child("--version").await;
}

#[tokio::test]
async fn conversation_help_timeout_kills_setup_process() {
    check_conversation_setup_timeout_kills_child("--help").await;
}
