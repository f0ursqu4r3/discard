//! The Codex provider against a fake `codex` executable that speaks the app-server protocol.

mod common;

use common::*;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::EnforcementMode;
use discard_core::driver::TurnOutcome;
use discard_core::event::EventKind;
use discard_core::provider::{Provider, ProviderError, SessionSpec, SetupStatus};
use discard_core::turn::TurnState;
use discard_providers::codex::CodexProvider;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fake-codex.py");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/codex-approval-turn.jsonl");

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
    fn provider(&self, mode: &str) -> CodexProvider {
        CodexProvider::new(FAKE)
            .with_env("FAKE_CODEX_MODE", mode)
            .with_env("FAKE_CODEX_LOG", self.log())
            .with_env("FAKE_CODEX_FIXTURE", FIXTURE)
            .with_env("FAKE_CODEX_CHILD_PID", self.pid_file())
    }
    fn log_text(&self) -> String {
        std::fs::read_to_string(self.log()).unwrap_or_default()
    }
    /// Messages the fake received from the provider, parsed.
    fn received(&self) -> Vec<Value> {
        self.log_text().lines().filter_map(|l| l.strip_prefix("STDIN ")).filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
    fn request(&self, method: &str) -> Value {
        self.received().into_iter().find(|m| m["method"] == method).unwrap_or_else(|| panic!("no {method} request in {}", self.log_text()))
    }
    /// The answer to a server request, by its id.
    fn answer(&self, id: Value) -> Value {
        self.received().into_iter().find(|m| m.get("method").is_none() && m["id"] == id).unwrap_or_else(|| panic!("no answer for {id}: {}", self.log_text()))
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
    SessionSpec { cwd: PathBuf::from("."), model: None, system_prompt: None, enforcement: EnforcementMode::HostPrompts, resume_handle: None, mcp_config: None }
}

#[tokio::test]
async fn setup_check_reports_a_missing_executable() {
    assert!(matches!(CodexProvider::new("/nonexistent/codex").check_setup().await, SetupStatus::NotInstalled { .. }));
}

#[tokio::test]
async fn setup_check_reports_logged_out_with_the_login_command() {
    let rig = Rig::new();
    let p = rig.provider("approval").with_env("FAKE_CODEX_LOGGED_IN", "false");
    assert_eq!(p.check_setup().await, SetupStatus::NotLoggedIn { login_command: "codex login".into() });
}

#[tokio::test]
async fn setup_check_reports_ready_when_logged_in() {
    assert_eq!(Rig::new().provider("approval").check_setup().await, SetupStatus::Ready);
}

#[tokio::test]
async fn approved_turn_runs_end_to_end_and_answers_with_accept() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("approval");
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Completed, "{seen:#?}");
    assert_eq!(h.turn_state(&turn), TurnState::Completed);
    let id = approval_id_of(&seen);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::ToolCallStart { .. })));
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::ToolCallResult { .. })));
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::Usage { .. })));
    // The fixture's approval request has id 0. The answer must echo it.
    let answer = rig.answer(json!(0));
    assert_eq!(answer["result"], json!({"decision": "accept"}));
    assert_eq!(answer["jsonrpc"], "2.0");
}

#[tokio::test]
async fn the_handshake_sends_initialize_initialized_thread_start_and_turn_start_in_order() {
    let rig = Rig::new();
    let mut h = Harness::new();
    drive(&mut h, &rig.provider("approval"), |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    let methods: Vec<String> = rig.received().iter().filter_map(|m| m["method"].as_str().map(str::to_string)).collect();
    assert_eq!(methods, ["initialize", "initialized", "thread/start", "turn/start"]);
    assert_eq!(rig.request("initialize")["params"]["clientInfo"]["name"], "discard");
    let turn = rig.request("turn/start");
    assert_eq!(turn["params"]["threadId"], "thread-fake-1");
    assert_eq!(turn["params"]["input"], json!([{"type": "text", "text": "hello"}]));
    assert_eq!(rig.request("thread/start")["params"]["approvalPolicy"], "untrusted");
}

#[tokio::test]
async fn denied_turn_answers_with_the_cancel_the_server_offered() {
    let rig = Rig::new();
    let mut h = Harness::new();
    drive(&mut h, &rig.provider("approval"), |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::Deny)).await;
    })
    .await;
    // The fixture offers only accept and cancel, so a denial is a cancel.
    assert_eq!(rig.answer(json!(0))["result"], json!({"decision": "cancel"}));
}

#[tokio::test]
async fn always_allow_falls_back_to_accept_when_the_server_does_not_offer_a_session_rule() {
    let rig = Rig::new();
    let mut h = Harness::new();
    drive(&mut h, &rig.provider("approval"), |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::AlwaysAllow)).await;
    })
    .await;
    assert_eq!(rig.answer(json!(0))["result"], json!({"decision": "accept"}));
}

#[tokio::test]
async fn the_thread_id_becomes_the_resume_handle_and_resume_uses_thread_resume() {
    let rig = Rig::new();
    let p = rig.provider("approval");
    let s = p.start_session(spec()).await.unwrap();
    assert_eq!(s.resume_handle().as_deref(), Some("thread-fake-1"));
    drop(s);

    let rig2 = Rig::new();
    let mut sp = spec();
    sp.resume_handle = Some("thread-old".into());
    let s = rig2.provider("approval").start_session(sp).await.unwrap();
    assert_eq!(s.resume_handle().as_deref(), Some("thread-old"));
    assert_eq!(rig2.request("thread/resume")["params"]["threadId"], "thread-old");
}

#[tokio::test]
async fn a_rejected_resume_is_reported_as_resume_rejected() {
    let rig = Rig::new();
    let mut sp = spec();
    sp.resume_handle = Some("gone".into());
    let err = rig.provider("resume_reject").start_session(sp).await.err().unwrap();
    assert!(matches!(&err, ProviderError::ResumeRejected(m) if m.contains("no rollout")), "{err:?}");
}

#[tokio::test]
async fn the_memory_mcp_config_is_translated_into_thread_config() {
    let rig = Rig::new();
    let cfg = rig.dir.path().join("mcp.json");
    std::fs::write(&cfg, json!({"mcpServers": {"discard-memory": {"type": "http", "url": "http://127.0.0.1:5555/mcp", "headers": {"Authorization": "Bearer tok"}}}}).to_string()).unwrap();
    let mut sp = spec();
    sp.mcp_config = Some(cfg);
    sp.system_prompt = Some("PROMPT".into());
    rig.provider("approval").start_session(sp).await.unwrap();
    let params = rig.request("thread/start")["params"].clone();
    assert_eq!(params["config"]["mcp_servers"]["discard-memory"]["url"], "http://127.0.0.1:5555/mcp");
    assert_eq!(params["config"]["mcp_servers"]["discard-memory"]["http_headers"]["Authorization"], "Bearer tok");
    assert_eq!(params["developerInstructions"], "PROMPT");
}

#[tokio::test]
async fn an_unreadable_mcp_config_fails_the_session_instead_of_starting_without_memory() {
    let rig = Rig::new();
    let mut sp = spec();
    sp.mcp_config = Some(PathBuf::from("/nonexistent/mcp.json"));
    let err = rig.provider("approval").start_session(sp).await.err().unwrap();
    assert!(matches!(err, ProviderError::Setup(_)), "{err:?}");
    assert!(rig.log_text().is_empty(), "no process may start");
}

#[tokio::test]
async fn deny_all_policy_starts_a_read_only_thread_that_never_asks() {
    let rig = Rig::new();
    let mut sp = spec();
    sp.enforcement = EnforcementMode::ProviderDenies;
    rig.provider("approval").start_session(sp).await.unwrap();
    let p = rig.request("thread/start")["params"].clone();
    assert_eq!((p["approvalPolicy"].clone(), p["sandbox"].clone()), (json!("never"), json!("read-only")));
}

#[tokio::test]
async fn unsupported_server_requests_are_answered_with_an_error_and_shown_to_the_user() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let mut seen = Vec::new();
    drive(&mut h, &rig.provider("unsupported"), |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    let answer = rig.answer(json!(77));
    assert!(answer["error"]["message"].as_str().is_some_and(|m| m.contains("does not support")), "{answer}");
    assert!(seen.iter().any(|e| matches!(&e.kind, EventKind::Error { message } if message.contains("item/permissions/requestApproval"))));
}

#[tokio::test]
async fn cancel_kills_the_whole_process_group() {
    let rig = Rig::new();
    let mut h = Harness::new();
    let p = rig.provider("hang");
    let pid_file = rig.pid_file();
    let (turn, outcome) = drive(&mut h, &p, |mut ui| async move {
        ui.updates.recv().await.expect("the turn started");
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
async fn early_exit_is_reported_with_the_redacted_stderr_tail() {
    let rig = Rig::new();
    let err = rig.provider("exit_early").start_session(spec()).await.err().unwrap();
    let ProviderError::Protocol(msg) = &err else { panic!("{err:?}") };
    assert!(msg.contains("invalid config"), "{msg}");
}

#[tokio::test]
async fn starting_fails_closed_without_interactive_approvals() {
    let rig = Rig::new();
    let mut matrix = rig.provider("approval").capabilities().clone();
    matrix.capabilities.interactive_approvals = false;
    let p = CodexProvider::with_matrix(FAKE, matrix).with_env("FAKE_CODEX_LOG", rig.log());
    let err = p.start_session(spec()).await.err().unwrap();
    assert!(matches!(err, ProviderError::Policy(_)));
    assert!(rig.log_text().is_empty());
}

#[tokio::test]
async fn spawn_failure_is_reported_not_panicked() {
    let err = CodexProvider::new("/nonexistent/codex").start_session(spec()).await.err().unwrap();
    assert!(matches!(err, ProviderError::Spawn(_)));
}
