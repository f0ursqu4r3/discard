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
        self.received()
            .into_iter()
            .find(|m| m.get("method").is_none() && m["id"] == id)
            .unwrap_or_else(|| panic!("no answer for {id}: {}", self.log_text()))
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
async fn reasoning_level_reaches_new_and_resumed_turns() {
    for handle in [None, Some("thread-old".to_string())] {
        let rig = Rig::new();
        let mut sp = spec();
        sp.resume_handle = handle;
        sp.reasoning_effort = Some("high".into());
        let mut session = rig.provider("approval").start_session(sp).await.unwrap();
        session.send_user_message("hello").await.unwrap();
        // Wait for the fixture to process and record the turn request.
        tokio::time::timeout(Duration::from_secs(5), session.next_event()).await.unwrap();
        assert_eq!(rig.request("turn/start")["params"]["effort"], "high");
    }
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
    assert!(
        params["config"]["mcp_servers"]["discard-memory"].get("default_tools_approval_mode").is_none(),
        "Coding mode must keep its existing approval behavior"
    );
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

fn conversation_spec(rig: &Rig) -> SessionSpec {
    let cache = json!({"client_version":"0.166.0","models":[{"slug":"exact-model","context_window":200000,"unrelated":{"keep":true}}]});
    std::fs::write(rig.dir.path().join("models_cache.json"), cache.to_string()).unwrap();
    let mcp = rig.dir.path().join("memory.json");
    std::fs::write(
        &mcp,
        json!({"mcpServers":{"discard-memory":{"url":"http://127.0.0.1:1/mcp","headers":{"Authorization":"Bearer synthetic"}}}})
            .to_string(),
    )
    .unwrap();
    let mut s = spec();
    s.mode = discard_core::provider::SessionMode::Conversation;
    s.model = Some("exact-model".into());
    s.system_prompt = Some("Synthetic scoped memory.".into());
    s.mcp_config = Some(mcp);
    s
}

#[tokio::test]
async fn conversation_startup_disables_inherited_servers_before_prompt_and_cleans_catalog() {
    let rig = Rig::new();
    let provider = rig
        .provider("approval")
        .with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_CONFIG", json!({"mcp_servers":{"inherited":{"command":"must-not-start"}}}).to_string());
    let s = conversation_spec(&rig);
    let original = std::fs::read(rig.dir.path().join("models_cache.json")).unwrap();
    let session = provider.start_session(s).await.unwrap();
    let request = rig.request("thread/start");
    assert_eq!(request["params"]["allowProviderModelFallback"], false);
    assert_eq!(request["params"]["config"]["mcp_servers"]["discard-memory"]["default_tools_approval_mode"], "approve");
    assert_eq!(request["params"]["approvalPolicy"], "never");
    assert_eq!(request["params"]["sandbox"], "read-only");
    assert_eq!(request["params"]["model"], "exact-model");
    assert_eq!(request["params"]["config"]["mcp_servers"]["inherited"]["enabled"], false);
    assert_eq!(request["params"]["developerInstructions"], "Synthetic scoped memory.");
    assert_eq!(
        request["params"]["config"]["mcp_servers"]["discard-memory"]["enabled_tools"],
        json!(["memory_search", "memory_list", "memory_read", "memory_save", "memory_edit", "memory_forget",
            "worktree_inspect", "worktree_request_removal", "worktree_request_creation", "main_checkout_request_write", "project_list_files", "project_read_file", "project_search"])
    );
    let cwd = PathBuf::from(request["params"]["cwd"].as_str().unwrap());
    assert!(std::fs::read_dir(&cwd).unwrap().next().is_none());
    let catalog = cwd.parent().unwrap().join("models.json");
    let sanitized: Value = serde_json::from_slice(&std::fs::read(&catalog).unwrap()).unwrap();
    assert_eq!(sanitized["models"][0]["apply_patch_tool_type"], Value::Null);
    assert_eq!(sanitized["models"][0]["tool_mode"], "direct");
    assert_eq!(std::fs::read(rig.dir.path().join("models_cache.json")).unwrap(), original);
    assert_eq!(provider.model_context_window("exact-model"), Some(200000));
    assert!(!rig.received().iter().any(|v| v["method"] == "model/list"), "a valid cache needs no refresh");
    assert!(rig.log_text().contains("\"inherited\" = {\"enabled\" = false}"));
    drop(session);
    assert!(!catalog.exists());
}

#[tokio::test]
async fn conversation_control_override_fails_before_memory_or_transcript_delivery() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let provider = rig.provider("control_override").with_env("CODEX_HOME", rig.dir.path());
    let error = provider.start_session(s).await.err().unwrap().to_string();
    assert!(error.contains("shell_tool"), "{error}");
    assert!(!rig.received().iter().any(|v| v["method"] == "thread/start" || v["method"] == "turn/start"));
}

#[tokio::test]
async fn conversation_catalog_errors_never_start_a_thread_or_substitute_model() {
    for broken in ["missing", "malformed", "wrong-model", "wrong-version"] {
        let rig = Rig::new();
        let mut s = conversation_spec(&rig);
        let path = rig.dir.path().join("models_cache.json");
        match broken {
            "missing" => std::fs::remove_file(path).unwrap(),
            "malformed" => std::fs::write(path, "not json").unwrap(),
            "wrong-model" => s.model = Some("absent".into()),
            _ => std::fs::write(path, json!({"client_version":"0.1.0","models":[]}).to_string()).unwrap(),
        }
        assert!(rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).start_session(s).await.is_err());
        let requests = rig.received();
        assert_eq!(requests.iter().filter(|v| v["method"] == "model/list").count(), 1);
        assert!(!requests.iter().any(|v| v["method"] == "thread/start" || v["method"] == "turn/start"));
    }
}

#[tokio::test]
async fn conversation_repairs_missing_stale_and_malformed_catalogs_before_starting_once() {
    for broken in ["missing", "malformed", "wrong-version", "missing-model"] {
        let rig = Rig::new();
        let s = conversation_spec(&rig);
        let path = rig.dir.path().join("models_cache.json");
        let refreshed = rig.dir.path().join("refreshed-cache.json");
        std::fs::copy(&path, &refreshed).unwrap();
        match broken {
            "missing" => std::fs::remove_file(&path).unwrap(),
            "malformed" => std::fs::write(&path, "not json").unwrap(),
            "missing-model" => std::fs::write(&path, json!({"client_version":"0.166.0","models":[{"slug":"another-model"}]}).to_string()).unwrap(),
            _ => std::fs::write(&path, json!({"client_version":"0.164.0","models":[]}).to_string()).unwrap(),
        }
        let provider = rig.provider("approval")
            .with_env("CODEX_HOME", rig.dir.path())
            .with_env("FAKE_CODEX_REFRESH_CACHE", &refreshed);
        let session = provider.start_session(s).await.unwrap();
        let requests = rig.received();
        assert_eq!(requests.iter().filter(|v| v["method"] == "model/list").count(), 1, "{broken}");
        assert_eq!(requests.iter().filter(|v| v["method"] == "thread/start").count(), 1, "{broken}");
        assert!(!requests.iter().any(|v| v["method"] == "turn/start"), "setup must not send the user message");
        let request = rig.request("thread/start");
        assert_eq!(request["params"]["model"], "exact-model");
        assert_eq!(request["params"]["allowProviderModelFallback"], false);
        let cwd = PathBuf::from(request["params"]["cwd"].as_str().unwrap());
        let catalog: Value = serde_json::from_slice(&std::fs::read(cwd.parent().unwrap().join("models.json")).unwrap()).unwrap();
        assert_eq!(catalog["models"][0]["shell_type"], "disabled");
        assert_eq!(catalog["models"][0]["apply_patch_tool_type"], Value::Null);
        assert_eq!(catalog["models"][0]["unrelated"]["keep"], true);
        drop(session);
    }
}

#[tokio::test]
async fn failed_automatic_catalog_refresh_is_bounded_and_can_be_retried() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let path = rig.dir.path().join("models_cache.json");
    let refreshed = rig.dir.path().join("refreshed-cache.json");
    std::fs::copy(&path, &refreshed).unwrap();
    std::fs::remove_file(&path).unwrap();
    let error = rig.provider("approval")
        .with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_REFRESH_CACHE", "fail")
        .start_session(s.clone()).await.err().unwrap().to_string();
    assert!(error.contains("Automatic Codex model refresh failed"), "{error}");
    let requests = rig.received();
    assert_eq!(requests.iter().filter(|v| v["method"] == "model/list").count(), 1);
    assert!(!requests.iter().any(|v| v["method"] == "thread/start" || v["method"] == "turn/start"));
    let session = rig.provider("approval")
        .with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_REFRESH_CACHE", &refreshed)
        .start_session(s).await.unwrap();
    assert_eq!(rig.request("thread/start")["params"]["model"], "exact-model");
    drop(session);
}

#[tokio::test]
async fn unsupported_codex_version_never_attempts_catalog_recovery() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    std::fs::remove_file(rig.dir.path().join("models_cache.json")).unwrap();
    let error = rig.provider("approval")
        .with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_VERSION", "codex-cli 0.164.0")
        .start_session(s).await.err().unwrap().to_string();
    assert!(error.contains("Codex 0.164.0 is older than the tested version 0.166.0. Update Codex, then retry."), "{error}");
    assert!(rig.received().is_empty());
}

#[tokio::test]
async fn conversation_version_timeout_kills_setup_process() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let error = rig
        .provider("approval")
        .with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_HANG_SETUP", "1")
        .start_session(s)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert_setup_process_terminated(&rig.pid_file()).await;
}

#[tokio::test]
async fn workspace_thread_starts_in_the_granted_directory() {
    let rig = Rig::new(); let mut s = conversation_spec(&rig);
    s.mode = discard_core::provider::SessionMode::Workspace; s.cwd = rig.dir.path().canonicalize().unwrap();
    let mut session = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).start_session(s.clone()).await.unwrap();
    session.send_user_message("synthetic").await.unwrap();
    let log = rig.log_text();
    let params = log.lines().filter_map(|l| l.strip_prefix("STDIN ")).filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["method"] == "thread/start").unwrap();
    assert_eq!(params["params"]["cwd"], s.cwd.to_string_lossy().as_ref());
    assert_eq!(params["params"]["sandbox"], "workspace-write");
    assert!(params["params"]["config"]["mcp_servers"]["discard-memory"]["enabled_tools"].as_array().unwrap().contains(&json!("worktree_commit")));
    assert!(log.contains(&format!("CWD {}", s.cwd.display())));
}

#[tokio::test]
async fn workspace_repairs_stale_catalog_and_keeps_native_write_tools() {
    let rig = Rig::new(); let mut s = conversation_spec(&rig);
    s.mode = discard_core::provider::SessionMode::Workspace; s.cwd = rig.dir.path().canonicalize().unwrap();
    let path = rig.dir.path().join("models_cache.json"); let refreshed = rig.dir.path().join("refreshed-cache.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    cache["models"][0]["shell_type"] = json!("default"); cache["models"][0]["apply_patch_tool_type"] = json!("freeform");
    std::fs::write(&refreshed, cache.to_string()).unwrap();
    std::fs::write(&path, json!({"client_version":"0.167.0","models":[]}).to_string()).unwrap();
    let session = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).with_env("FAKE_CODEX_REFRESH_CACHE", &refreshed).start_session(s).await.unwrap();
    assert_eq!(rig.received().iter().filter(|v| v["method"] == "model/list").count(), 1);
    let request = rig.request("thread/start");
    assert_eq!(request["params"]["sandbox"], "workspace-write");
    let catalog: Value = serde_json::from_slice(&std::fs::read(request["params"]["config"]["model_catalog_json"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(catalog["models"][0]["shell_type"], "default");
    assert_eq!(catalog["models"][0]["apply_patch_tool_type"], "freeform");
    drop(session);
}

#[tokio::test]
async fn workspace_missing_model_keeps_setup_error() {
    let rig = Rig::new(); let mut s = conversation_spec(&rig);
    s.mode = discard_core::provider::SessionMode::Workspace; s.cwd = rig.dir.path().canonicalize().unwrap(); s.model = None;
    let error = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).start_session(s).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)), "{error:?}");
    assert!(error.to_string().contains("select an exact Codex model"));
}

#[tokio::test]
async fn newer_codex_refreshes_tested_cache_and_exposes_notice_after_verification() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let mut cache: Value = serde_json::from_slice(&std::fs::read(rig.dir.path().join("models_cache.json")).unwrap()).unwrap();
    cache["client_version"] = json!("0.167.0");
    let refreshed = rig.dir.path().join("refreshed.json");
    std::fs::write(&refreshed, cache.to_string()).unwrap();
    let session = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_VERSION", "codex-cli 0.167.0").with_env("FAKE_CODEX_REFRESH_CACHE", &refreshed)
        .start_session(s).await.unwrap();
    assert_eq!(session.notice().as_deref(), Some("Codex 0.167.0 is newer than the tested version 0.166.0. Discard verified the isolation controls before this response."));
    assert_eq!(rig.received().iter().filter(|v| v["method"] == "model/list").count(), 1);
    assert_eq!(rig.request("thread/start")["params"]["config"]["features"]["shell_tool"], false);
}

#[tokio::test]
async fn newer_codex_still_rejects_overridden_controls_with_both_versions() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let path = rig.dir.path().join("models_cache.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    cache["client_version"] = json!("0.167.0");
    std::fs::write(path, cache.to_string()).unwrap();
    let error = rig.provider("control_override").with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_VERSION", "codex-cli 0.167.0").start_session(s).await.err().unwrap();
    let message = error.to_string();
    assert!(message.contains("0.167.0") && message.contains("0.166.0"), "{message}");
    assert!(!rig.received().iter().any(|v| v["method"] == "thread/start"));
}

#[tokio::test]
async fn unknown_codex_version_fails_before_catalog_recovery() {
    let rig = Rig::new();
    let s = conversation_spec(&rig);
    let error = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path())
        .with_env("FAKE_CODEX_VERSION", "unreadable").start_session(s).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)));
    assert!(error.to_string().contains("version could not be read"));
    assert!(rig.received().is_empty());
}

#[tokio::test]
async fn failed_codex_version_probe_does_not_accept_valid_stdout() {
    let rig = Rig::new();
    let error = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).with_env("FAKE_CODEX_VERSION_EXIT", "1")
        .start_session(conversation_spec(&rig)).await.err().unwrap();
    assert!(matches!(error, ProviderError::Setup(_)));
    assert!(error.to_string().contains("version could not be read"), "{error}");
}

#[tokio::test]
async fn tested_codex_version_has_no_notice() {
    let rig = Rig::new();
    let session = rig.provider("approval").with_env("CODEX_HOME", rig.dir.path()).start_session(conversation_spec(&rig)).await.unwrap();
    assert_eq!(session.notice(), None);
}
