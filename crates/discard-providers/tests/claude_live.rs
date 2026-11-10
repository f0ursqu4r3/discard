//! Live check against the real `claude` CLI. Uses the logged-in subscription (one small Haiku turn).
//! Run with: cargo test -p discard-providers --test claude_live -- --ignored --nocapture

mod common;

use common::*;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::capability::EnforcementMode;
use discard_core::driver::TurnOutcome;
use discard_core::event::EventKind;
use discard_core::provider::{Provider, SessionSpec, SetupStatus};
use discard_providers::claude::ClaudeProvider;

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_asks_for_approval_and_runs_the_approved_command() {
    let p = ClaudeProvider::new("claude");
    assert_eq!(p.check_setup().await, SetupStatus::Ready, "run `claude auth login` first");

    let dir = tempfile::tempdir().unwrap();
    let spec = SessionSpec {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding,
        cwd: dir.path().to_path_buf(),
        model: Some("haiku".into()),
        system_prompt: None,
        enforcement: EnforcementMode::HostPrompts,
        resume_handle: None,
        mcp_config: None,
        sandbox_autorun: false,
    };
    let mut h = Harness::new();
    let mut seen = Vec::new();
    let (_, outcome) = drive_with(
        &mut h,
        &p,
        spec,
        "Use the Bash tool to run `touch approved.txt`, then say done.",
        |ui| async {
            seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
        },
    )
    .await;

    assert_eq!(outcome, TurnOutcome::Completed, "events: {seen:#?}");
    let id = approval_id_of(&seen);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
    assert!(dir.path().join("approved.txt").exists(), "the approved command must have run");
    assert!(seen.iter().any(|e| matches!(e.kind, EventKind::Usage { .. })));
}

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_does_not_run_a_denied_command() {
    let p = ClaudeProvider::new("claude");
    let dir = tempfile::tempdir().unwrap();
    let spec = SessionSpec {
        reasoning_effort: None,
        mode: discard_core::provider::SessionMode::Coding,
        cwd: dir.path().to_path_buf(),
        model: Some("haiku".into()),
        system_prompt: None,
        enforcement: EnforcementMode::HostPrompts,
        resume_handle: None,
        mcp_config: None,
        sandbox_autorun: false,
    };
    let mut h = Harness::new();
    let (_, outcome) = drive_with(
        &mut h,
        &p,
        spec,
        "Use the Bash tool to run `touch denied.txt`, then say done.",
        |ui| async {
            collect(ui, |id, ui| decide(ui, id, Decision::Deny)).await;
        },
    )
    .await;
    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(!dir.path().join("denied.txt").exists(), "a denied command must not run");
}

#[tokio::test]
#[ignore = "uses the real claude CLI and the logged-in subscription"]
async fn real_claude_generates_text_without_a_session() {
    let p = ClaudeProvider::new("claude");
    let request = discard_core::provider::TextRequest {
        model: "haiku".into(),
        reasoning_effort: None,
        instructions: "Answer with the single word the user asks for. No other text.".into(),
        prompt: "Say: ready".into(),
    };
    let started = std::time::Instant::now();
    let text = p.generate_text(request).await.expect("text generation");
    assert!(text.to_lowercase().contains("ready"), "unexpected text: {text}");
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "took {:?}", started.elapsed());
}
