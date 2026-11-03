//! Headless end-to-end tests: the turn driver, the store, and a scripted fake provider.

mod common;

use common::*;
use discard_core::approval::{ApprovalState, Decision};
use discard_core::driver::TurnOutcome;
use discard_core::event::{ApprovalId, EventKind};
use discard_core::turn::TurnState;
use discard_providers::fake::{FakeProvider, Step};
use serde_json::json;

fn bash() -> Step {
    Step::Approval { tool: "Bash".into(), input: json!({"command": "touch x"}) }
}

#[tokio::test]
async fn simple_turn_completes_and_persists_ordered_events() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), Step::text("hi"), Step::idle()]);
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async { seen = collect(ui, |_, _| {}).await }).await;
    assert_eq!(outcome, TurnOutcome::Completed);
    assert_eq!(h.turn_state(&turn), TurnState::Completed);
    assert_eq!(seen.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(h.store.lock().unwrap().events_for_session(&h.session_id).unwrap(), seen);
    assert_eq!(p.probe.sent.lock().unwrap().as_slice(), ["hello"]);
}

#[tokio::test]
async fn approved_tool_call_resumes_the_turn_and_is_audited() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::text("done"), Step::idle()]);
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Approve)).await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(outcome, TurnOutcome::Completed);
    assert_eq!(h.turn_state(&turn), TurnState::Completed);
    assert_eq!(p.probe.responses.lock().unwrap().as_slice(), [(id.clone(), Decision::Approve)]);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
    assert_eq!(h.audit_len(&id), 1);
}

#[tokio::test]
async fn denied_tool_call_is_reported_to_the_provider() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]);
    let mut seen = Vec::new();
    drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| decide(ui, id, Decision::Deny)).await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(p.probe.responses.lock().unwrap().as_slice(), [(id.clone(), Decision::Deny)]);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Deny));
}

#[tokio::test]
async fn duplicate_decision_answers_the_provider_only_once() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]);
    let mut seen = Vec::new();
    drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| {
            decide(ui, id, Decision::Approve);
            decide(ui, id, Decision::Approve);
        })
        .await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(p.probe.responses.lock().unwrap().len(), 1);
    assert_eq!(h.audit_len(&id), 1);
}

#[tokio::test]
async fn conflicting_second_decision_is_ignored() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]);
    let mut seen = Vec::new();
    drive(&mut h, &p, |ui| async {
        seen = collect(ui, |id, ui| {
            decide(ui, id, Decision::Approve);
            decide(ui, id, Decision::Deny);
        })
        .await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(p.probe.responses.lock().unwrap().as_slice(), [(id.clone(), Decision::Approve)]);
    assert_eq!(h.approval_state(&id), ApprovalState::Decided(Decision::Approve));
}

#[tokio::test]
async fn decision_for_an_unknown_approval_is_ignored() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]);
    let (_, outcome) = drive(&mut h, &p, |ui| async {
        collect(ui, |id, ui| {
            decide(ui, &ApprovalId::new(), Decision::Approve);
            decide(ui, id, Decision::Approve);
        })
        .await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Completed);
    assert_eq!(p.probe.responses.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn provider_crash_mid_turn_fails_the_turn() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), Step::text("par"), Step::Crash]);
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        collect(ui, |_, _| {}).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(h.turn_state(&turn), TurnState::Failed);
}

#[tokio::test]
async fn crash_while_awaiting_approval_marks_the_approval_stale() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![
        Step::working(),
        Step::ApprovalThenCrash { tool: "Bash".into(), input: json!({}) },
    ]);
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |_, _| {}).await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(h.turn_state(&turn), TurnState::Failed);
    assert_eq!(h.approval_state(&id), ApprovalState::Stale);
}

#[tokio::test]
async fn cancel_stops_a_hanging_turn() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), Step::Hang]);
    let (turn, outcome) = drive(&mut h, &p, |mut ui| async move {
        ui.updates.recv().await.unwrap();
        ui.cancel.send(()).unwrap();
        while ui.updates.recv().await.is_some() {}
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(h.turn_state(&turn), TurnState::Cancelled);
    assert!(p.probe.cancelled.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_while_awaiting_approval_marks_it_stale() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![Step::working(), bash(), Step::idle()]);
    let mut seen = Vec::new();
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        seen = collect(ui, |_, ui| ui.cancel.send(()).unwrap()).await;
    })
    .await;
    let id = approval_id_of(&seen);
    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(h.turn_state(&turn), TurnState::Cancelled);
    assert_eq!(h.approval_state(&id), ApprovalState::Stale);
}

#[tokio::test]
async fn an_error_event_makes_the_turn_fail_even_if_the_provider_then_goes_idle() {
    let mut h = Harness::new();
    let p = FakeProvider::new(vec![
        Step::working(),
        Step::Emit(EventKind::Error { message: "boom".into() }),
        Step::idle(),
    ]);
    let (turn, outcome) = drive(&mut h, &p, |ui| async {
        collect(ui, |_, _| {}).await;
    })
    .await;
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(h.turn_state(&turn), TurnState::Failed);
}
