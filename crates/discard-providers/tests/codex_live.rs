//! Live checks against the real `codex` CLI (ChatGPT subscription login). Each test runs one small turn.
//! Run with: cargo test -p discard-providers --test codex_live -- --ignored --nocapture

mod common;

use common::*;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::EnforcementMode;
use discard_core::driver::TurnOutcome;
use discard_core::event::EventKind;
use discard_core::provider::{Provider, SessionSpec, SetupStatus};
use discard_providers::codex::CodexProvider;

fn spec(dir: &std::path::Path) -> SessionSpec {
    SessionSpec { reasoning_effort: None, mode: discard_core::provider::SessionMode::Coding, cwd: dir.to_path_buf(), model: None, system_prompt: None, enforcement: EnforcementMode::HostPrompts, resume_handle: None, mcp_config: None, sandbox_autorun: false }
}

#[tokio::test]
#[ignore = "uses the real codex CLI and the logged-in subscription"]
async fn real_codex_asks_for_approval_and_runs_the_approved_command() {
    let p = CodexProvider::new("codex");
    assert_eq!(p.check_setup().await, SetupStatus::Ready, "run `codex login` first");
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    let mut seen = Vec::new();
    let (_, outcome) = drive_with(&mut h, &p, spec(dir.path()), "Run the shell command `touch approved.txt` and then say done.", |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Completed, "events: {seen:#?}");
    let id = approval_id_of(&seen);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
    assert!(dir.path().join("approved.txt").exists(), "the approved command must have run");
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::Usage { .. })));
}

#[tokio::test]
#[ignore = "uses the real codex CLI and the logged-in subscription"]
async fn real_codex_does_not_run_a_denied_command() {
    let p = CodexProvider::new("codex");
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    let (_, _outcome) = drive_with(&mut h, &p, spec(dir.path()), "Run the shell command `touch denied.txt` and then say done.", |ui| async {
        collect(ui, |id, ui| decide(ui, id, Decision::Deny)).await;
    })
    .await;
    assert!(!dir.path().join("denied.txt").exists(), "a denied command must not run");
}
