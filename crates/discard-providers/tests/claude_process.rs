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
        resumable: false,
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding,
        cwd: PathBuf::from("."),
        model: None,
        system_prompt: None,
        enforcement: EnforcementMode::HostPrompts,
        resume_handle: None,
        mcp_config: None,
        sandbox_autorun: false,
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

#[tokio::test]
#[cfg(target_os = "macos")]
async fn workspace_session_runs_in_the_granted_directory() {
    let rig = Rig::new(); let mut s = conversation_spec(&rig);
    s.mode = discard_core::provider::SessionMode::Workspace;
    s.cwd = rig.dir.path().canonicalize().unwrap();
    s.cwd = linked_workspace(rig.dir.path());
    let mut session = rig.provider("approval").start_session(s.clone()).await.unwrap();
    let _ = session.next_event().await;
    let log = rig.log_text(); assert!(log.contains(&format!("CWD {}", s.cwd.display())), "{log}");
    assert!(log.contains("GIT_LOCKS 0")); assert!(log.contains("MEMORY_ENV 1 1 "));
    drop(session); assert!(s.cwd.exists());
}

#[tokio::test]
#[cfg(target_os = "macos")]
async fn workspace_missing_model_keeps_setup_error() {
    let rig = Rig::new(); let mut s = conversation_spec(&rig);
    s.mode = discard_core::provider::SessionMode::Workspace; s.cwd = rig.dir.path().canonicalize().unwrap(); s.model = None;
    s.cwd = linked_workspace(rig.dir.path());
    let error = rig.provider("approval").start_session(s).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)), "{error:?}");
    assert!(error.to_string().contains("select an exact Claude model"));
}

#[cfg(target_os = "macos")]
fn linked_workspace(base: &std::path::Path) -> PathBuf {
    let repo = base.join("repo"); std::fs::create_dir(&repo).unwrap();
    let root = base.join("worktree");
    for args in [vec!["init", "-q"], vec!["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "init"], vec!["worktree", "add", "-b", "feature", root.to_str().unwrap()]] {
        assert!(discard_core::process::spawn_std(std::process::Command::new("git").args(["-c", "core.hooksPath=/dev/null"]).args(args).current_dir(&repo)).and_then(|mut child| child.wait()).unwrap().success());
    }
    root.canonicalize().unwrap()
}

#[cfg(target_os = "macos")]
fn main_checkout_spec(rig: &Rig) -> SessionSpec {
    let mut s = conversation_spec(rig);
    s.mode = discard_core::provider::SessionMode::Workspace;
    let repo = rig.dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    assert!(discard_core::process::spawn_std(std::process::Command::new("git").args(["init", "-q"]).current_dir(&repo)).and_then(|mut child| child.wait()).unwrap().success());
    s.cwd = repo.canonicalize().unwrap();
    s
}

#[tokio::test]
#[cfg(target_os = "macos")]
async fn claude_main_checkout_args_deny_writes_to_dot_git() {
    let rig = Rig::new(); let s = main_checkout_spec(&rig);
    let mut session = rig.provider("approval").start_session(s.clone()).await.unwrap();
    let _ = session.next_event().await;
    let log = rig.log_text();
    let settings = log.split_once("--settings ").expect("sandbox settings in CLI args").1;
    let settings = serde_json::Deserializer::from_str(settings).into_iter::<serde_json::Value>().next().unwrap().unwrap();
    assert!(settings["sandbox"]["filesystem"]["denyWrite"].as_array().unwrap().contains(&serde_json::json!(s.cwd.join(".git").canonicalize().unwrap())));
    assert!(log.contains(&format!("CWD {}", s.cwd.display())));
}

#[tokio::test]
#[cfg(target_os = "macos")]
async fn claude_main_checkout_file_tools_require_full_access_cards() {
    use discard_core::{approval::ApprovalKind, driver::{TurnContext, TurnInputs, run_turn}};
    for (tool, path) in [("Write", ".git/config"), ("Edit", ".git/hooks/pre-commit")] {
        let rig = Rig::new(); let s = main_checkout_spec(&rig);
        let fixture = rig.dir.path().join("git-edit.jsonl");
        let events: Vec<_> = std::fs::read_to_string(FIXTURE).unwrap().lines().map(|line| {
            let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event["type"] == "control_request" {
                event["request"]["tool_name"] = tool.into();
                event["request"]["input"] = serde_json::json!({"file_path":s.cwd.join(path), "content":"synthetic", "old_string":"old", "new_string":"new"});
                event["request"]["blocked_path"] = serde_json::json!(s.cwd.join(path));
            }
            event.to_string()
        }).collect();
        std::fs::write(&fixture, events.join("\n")).unwrap();
        let provider = rig.provider("approval").with_env("FAKE_CLAUDE_FIXTURE", fixture);
        let mut session = provider.start_session(s.clone()).await.unwrap();
        let mut h = Harness::new();
        let (updates, receive) = tokio::sync::mpsc::unbounded_channel();
        let (decisions, decision_rx) = tokio::sync::mpsc::unbounded_channel();
        let (cancel, cancel_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = TurnContext { workspace_root: Some(&s.cwd), store: &h.store, sequencer: &mut h.sequencer, session_id: &h.session_id, policy_name: "ask", updates };
        let ui = Ui { updates: receive, decisions, cancel };
        let (result, seen) = tokio::join!(
            run_turn(ctx, &mut *session, "edit", TurnInputs { decisions: decision_rx, cancel: cancel_rx }),
            collect(ui, |id, ui| decide(ui, id, Decision::Deny)),
        );
        assert_eq!(result.unwrap().1, TurnOutcome::Completed);
        let id = approval_id_of(&seen);
        assert!(seen.iter().any(|event| matches!(&event.kind, EventKind::ApprovalRequest { kind: ApprovalKind::SandboxEscape, tool: actual, .. } if actual == tool)), "{tool} {path}");
        let approval = h.store.lock().unwrap().load_approval(&id).unwrap();
        assert_eq!(approval.kind, ApprovalKind::SandboxEscape);
        assert_eq!(approval.state, ApprovalState::Decided(Decision::Deny));
        assert_eq!(approval.audit[0].decider, discard_core::approval::Decider::User);
    }
}

#[tokio::test]
async fn newer_claude_verifies_every_emitted_flag_and_exposes_notice() {
    let rig = Rig::new();
    let mut s = conversation_spec(&rig);
    s.reasoning_effort = Some("high".into());
    let session = rig.provider("approval").with_env("FAKE_CLAUDE_VERSION", "2.1.321 (Claude Code)").start_session(s).await.unwrap();
    assert_eq!(session.notice().as_deref(), Some("Claude Code 2.1.321 is newer than the tested version 2.1.318. Discard verified the isolation controls before this response."));
}

#[tokio::test]
async fn newer_claude_rejects_each_missing_emitted_flag() {
    let rig = Rig::new();
    let mut s = conversation_spec(&rig);
    s.reasoning_effort = Some("high".into());
    let args = discard_providers::claude::build_args(&s, &[], None).unwrap();
    let flags: Vec<_> = args.iter().filter(|arg| arg.starts_with("--")).collect();
    for missing in &flags {
        let help = flags.iter().filter(|flag| flag != &missing).map(|flag| flag.as_str()).collect::<Vec<_>>().join(" ");
        // A flag with a longer name must not count as the missing flag.
        let help = format!("{help} {missing}-unsupported");
        let error = rig.provider("approval").with_env("FAKE_CLAUDE_VERSION", "2.1.321 (Claude Code)")
            .with_env("FAKE_CLAUDE_HELP", help).start_session(s.clone()).await.err().unwrap();
        assert!(matches!(error, ProviderError::Setup(_)));
        let message = error.to_string();
        assert!(message.contains(missing.as_str()) && message.contains("2.1.321") && message.contains("2.1.318"), "{message}");
    }
    assert!(!rig.log_text().contains("ARGS"));
}

#[tokio::test]
async fn older_and_unknown_claude_versions_fail_before_session() {
    let rig = Rig::new();
    for (version, expected) in [("2.1.317 (Claude Code)", "Claude Code 2.1.317 is older than the tested version 2.1.318. Update Claude Code, then retry."), ("unreadable", "version could not be read")] {
        let error = rig.provider("approval").with_env("FAKE_CLAUDE_VERSION", version).start_session(conversation_spec(&rig)).await.err().unwrap();
        assert!(matches!(error, ProviderError::Setup(_)));
        assert!(error.to_string().contains(expected), "{error}");
    }
    assert!(!rig.log_text().contains("ARGS"));
}

#[tokio::test]
async fn unreachable_claude_api_blocks_all_session_modes_before_cli_start() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let rig = Rig::new();
    for mode in [discard_core::provider::SessionMode::Coding, discard_core::provider::SessionMode::Conversation, discard_core::provider::SessionMode::Workspace] {
        let mut s = conversation_spec(&rig); s.mode = mode;
        let start = std::time::Instant::now();
        let error = rig.provider("approval").with_env("ANTHROPIC_BASE_URL", format!("http://user:secret@127.0.0.1:{port}/private"))
            .start_session(s).await.err().unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(matches!(error, ProviderError::Setup(_)));
        assert!(error.to_string().contains(&format!("http://127.0.0.1:{port} (ANTHROPIC_BASE_URL)")));
        assert!(!error.to_string().contains("secret") && !error.to_string().contains("private"));
    }
    assert!(!rig.log().exists());
}

#[tokio::test]
async fn failed_claude_version_probe_does_not_accept_valid_stdout() {
    let rig = Rig::new();
    let error = rig.provider("approval").with_env("FAKE_CLAUDE_VERSION_EXIT", "1")
        .start_session(conversation_spec(&rig)).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)));
    assert!(error.to_string().contains("version could not be read"), "{error}");
}

#[tokio::test]
async fn tested_claude_version_has_no_notice() {
    let rig = Rig::new();
    let session = rig.provider("approval").start_session(conversation_spec(&rig)).await.unwrap();
    assert_eq!(session.notice(), None);
}

fn resumable_spec(rig: &Rig) -> SessionSpec {
    let mut s = conversation_spec(rig);
    s.resumable = true;
    s.cwd = rig.dir.path().join("stable-cwd");
    std::fs::create_dir_all(&s.cwd).unwrap();
    s.cwd = s.cwd.canonicalize().unwrap();
    s
}

fn log_line(log: &str, prefix: &str) -> String {
    log.lines().find_map(|l| l.strip_prefix(prefix)).unwrap_or_else(|| panic!("no {prefix} line in {log}")).to_string()
}

#[tokio::test]
async fn resumable_session_passes_resume_and_the_same_isolation_flags_as_a_fresh_one() {
    let mut lists = Vec::new();
    let rig = Rig::new();
    for handle in [None, Some("sess-9".to_string())] {
        let mut s = resumable_spec(&rig);
        s.resume_handle = handle;
        let mut session = rig.provider("approval").start_session(s).await.unwrap();
        let _ = session.next_event().await;
        lists.push(rig.log_text().lines().filter_map(|l| l.strip_prefix("ARGS ")).next_back().unwrap().to_string());
    }
    assert!(!lists[0].contains("--resume"), "{}", lists[0]);
    assert_eq!(lists[1].replace(" --resume sess-9", ""), lists[0]);
    assert!(lists[1].contains("--resume sess-9"));
    for flag in ["--tools", "--strict-mcp-config", "--restricted", "--disable-slash-commands", "--system-prompt-snapshot off", "--append-system-prompt Fresh synthetic memory", "--mcp-config"] {
        assert!(lists[1].contains(flag), "{flag}: {}", lists[1]);
    }
    assert!(!lists[1].contains("no-session-persistence"));
}

#[tokio::test]
async fn resumable_session_runs_in_the_given_directory() {
    let rig = Rig::new();
    let mut s = resumable_spec(&rig);
    s.resume_handle = Some("sess-9".into());
    let cwd = s.cwd.clone();
    let mut session = rig.provider("approval").start_session(s).await.unwrap();
    let _ = session.next_event().await;
    assert_eq!(log_line(&rig.log_text(), "CWD "), cwd.display().to_string());
    drop(session);
    assert!(cwd.exists(), "the directory belongs to the caller");
    // A relative or missing directory is refused.
    for bad in [PathBuf::from("relative"), rig.dir.path().join("missing")] {
        let mut s = resumable_spec(&rig);
        s.cwd = bad;
        let error = rig.provider("approval").start_session(s).await.err().unwrap();
        assert!(matches!(error, ProviderError::Setup(_)), "{error:?}");
    }
}

#[tokio::test]
async fn a_handle_without_resumable_is_still_refused_in_conversation() {
    let rig = Rig::new();
    let mut s = conversation_spec(&rig);
    s.resume_handle = Some("sess-9".into());
    let error = rig.provider("approval").start_session(s).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)), "{error:?}");
    assert!(error.to_string().contains("fresh transcript replay"), "{error}");
    assert!(!rig.log_text().contains("ARGS"));
}

#[tokio::test]
async fn the_unknown_session_error_maps_to_resume_rejected() {
    let rig = Rig::new();
    let mut s = resumable_spec(&rig);
    s.resume_handle = Some("gone-1".into());
    let mut session = rig.provider("resume_rejected").start_session(s).await.unwrap();
    let error = session.send_user_message("hello").await.expect_err("the rejection");
    assert!(matches!(&error, ProviderError::ResumeRejected(m) if m.contains("No conversation found")), "{error:?}");
    // No generic turn failure follows.
    assert!(session.next_event().await.is_none());
}

#[tokio::test]
async fn resumable_sessions_set_disable_auto_compact() {
    let rig = Rig::new();
    let mut session = rig.provider("approval").start_session(resumable_spec(&rig)).await.unwrap();
    let _ = session.next_event().await;
    assert_eq!(log_line(&rig.log_text(), "AUTO_COMPACT "), "1");
    let rig = Rig::new();
    let mut session = rig.provider("approval").start_session(conversation_spec(&rig)).await.unwrap();
    let _ = session.next_event().await;
    assert_eq!(log_line(&rig.log_text(), "AUTO_COMPACT ").trim(), "");
}

#[tokio::test]
async fn a_compact_boundary_event_sets_compacted() {
    let rig = Rig::new();
    let mut lines: Vec<String> = std::fs::read_to_string(FIXTURE).unwrap().lines().map(String::from).collect();
    lines.insert(1, r#"{"type":"system","subtype":"compact_boundary","session_id":"s","compact_metadata":{"trigger":"auto","pre_tokens":1}}"#.into());
    let fixture = rig.dir.path().join("compact.jsonl");
    std::fs::write(&fixture, lines.join("\n")).unwrap();
    let mut session = rig.provider("approval").with_env("FAKE_CLAUDE_FIXTURE", fixture).start_session(resumable_spec(&rig)).await.unwrap();
    assert!(!session.compacted());
    session.send_user_message("hello").await.unwrap();
    let mut seen = 0;
    while let Some(event) = session.next_event().await {
        seen += 1;
        if matches!(event.kind, EventKind::ApprovalRequest { .. }) { break; }
    }
    assert!(seen > 0 && session.compacted());
}

#[tokio::test]
async fn resume_epoch_returns_the_cli_version() {
    let rig = Rig::new();
    let p = rig.provider("approval");
    assert_eq!(p.resume_epoch(), None);
    assert_eq!(p.check_setup().await, SetupStatus::Ready);
    assert_eq!(p.resume_epoch().as_deref(), Some("2.1.318"));
    let p = rig.provider("approval").with_env("FAKE_CLAUDE_VERSION", "2.1.325 (Claude Code)");
    let _ = p.start_session(conversation_spec(&rig)).await.unwrap();
    assert_eq!(p.resume_epoch().as_deref(), Some("2.1.325"));
}

fn error_message(event: &discard_core::event::RawEvent) -> Option<&str> {
    match &event.kind { EventKind::Error { message } => Some(message), _ => None }
}

#[tokio::test]
async fn a_non_resumable_resume_with_an_unknown_session_keeps_the_generic_error() {
    for mode in ["exit_early", "resume_rejected"] {
        let rig = Rig::new();
        let mut sp = spec();
        sp.resume_handle = Some("gone-1".into());
        let mut s = rig.provider(mode).start_session(sp).await.unwrap();
        s.send_user_message("hello").await.expect("no ResumeRejected without resumable");
        let mut errors = Vec::new();
        while let Some(e) = s.next_event().await { if let Some(m) = error_message(&e) { errors.push(m.to_string()); } }
        assert!(errors.iter().any(|m| m.contains("No conversation found")), "{mode}: {errors:?}");
    }
}

#[tokio::test]
async fn disable_auto_compact_is_limited_to_resumable_conversation_and_workspace_sessions() {
    let rig = Rig::new();
    let mut sp = spec();
    sp.resumable = true;
    let mut s = rig.provider("approval").start_session(sp).await.unwrap();
    let _ = s.next_event().await;
    assert_eq!(log_line(&rig.log_text(), "AUTO_COMPACT ").trim(), "", "Coding must not get the variable");
}

#[tokio::test]
async fn an_accepted_resume_works_normally() {
    let rig = Rig::new();
    let mut sp = resumable_spec(&rig);
    sp.resume_handle = Some("sess-9".into());
    let mut s = rig.provider("approval").start_session(sp).await.unwrap();
    s.send_user_message("hello").await.unwrap();
    let mut kinds = 0;
    while let Some(e) = s.next_event().await {
        assert!(error_message(&e).is_none());
        kinds += 1;
        if matches!(e.kind, EventKind::ApprovalRequest { .. }) { break; }
    }
    assert!(kinds > 1);
    assert_eq!(s.resume_handle().as_deref(), Some("00000000-0000-4000-8000-000000000001"));
}

#[tokio::test]
async fn the_stderr_only_unknown_session_exit_maps_to_resume_rejected() {
    let rig = Rig::new();
    let mut sp = resumable_spec(&rig);
    sp.resume_handle = Some("nope".into());
    let mut s = rig.provider("exit_early").start_session(sp).await.unwrap();
    let error = s.send_user_message("hello").await.expect_err("the rejection");
    assert!(matches!(&error, ProviderError::ResumeRejected(m) if m.contains("No conversation found")), "{error:?}");
}

#[tokio::test]
async fn a_mid_turn_error_result_after_init_is_not_a_resume_rejection() {
    let rig = Rig::new();
    let mut lines: Vec<String> = std::fs::read_to_string(FIXTURE).unwrap().lines().map(String::from).collect();
    lines.insert(1, r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom","errors":["No conversation found with session ID: x"],"usage":{"input_tokens":0,"output_tokens":0}}"#.into());
    let fixture = rig.dir.path().join("mid-turn-error.jsonl");
    std::fs::write(&fixture, lines.join("\n")).unwrap();
    let mut sp = resumable_spec(&rig);
    sp.resume_handle = Some("sess-9".into());
    let mut s = rig.provider("approval").with_env("FAKE_CLAUDE_FIXTURE", fixture).start_session(sp).await.unwrap();
    s.send_user_message("hello").await.expect("not ResumeRejected");
    let mut saw_error = false;
    while let Some(e) = s.next_event().await {
        if error_message(&e) == Some("boom") { saw_error = true; break; }
    }
    assert!(saw_error);
}
