//! Runs one turn: reads provider events, persists them, and drives the turn and approval state machines.

use crate::approval::{Approval, ApprovalState, Decider, Decision, DecisionOutcome};
use crate::event::{AgentStatus, ApprovalId, Event, EventKind, Sequencer, SessionId};
use crate::provider::{ProviderError, ProviderSession};
use crate::store::{Store, StoreError};
use crate::turn::{Turn, TurnState};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// A decision coming from the UI, a policy, or a plugin.
#[derive(Debug, Clone)]
pub struct DecisionMsg {
    pub approval_id: ApprovalId,
    pub decision: Decision,
    pub decider: Decider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    Failed,
    Cancelled,
}

pub struct TurnInputs {
    pub decisions: mpsc::UnboundedReceiver<DecisionMsg>,
    pub cancel: mpsc::UnboundedReceiver<()>,
}

pub struct TurnContext<'a> {
    pub store: &'a Mutex<Store>,
    pub sequencer: &'a mut Sequencer,
    pub session_id: &'a SessionId,
    /// Name of the policy recorded in the audit table.
    pub policy_name: &'a str,
    /// Every stored event is also sent here so the UI can render it.
    pub updates: mpsc::UnboundedSender<Event>,
}

pub(crate) fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    // A poisoned lock only means another thread panicked; the data is still valid SQLite state.
    store.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_state(store: &Mutex<Store>, turn: &mut Turn, to: TurnState) -> Result<(), StoreError> {
    lock(store).update_turn_state(&turn.id, to)?;
    turn.state = to;
    Ok(())
}

/// Mark every undecided approval stale. A stale approval can never be answered.
fn stale_all(store: &Mutex<Store>, pending: &mut HashMap<ApprovalId, Approval>) -> Result<(), StoreError> {
    for approval in pending.values_mut() {
        if approval.state == ApprovalState::Pending {
            approval.mark_stale();
            lock(store).save_approval(approval)?;
        }
    }
    Ok(())
}

/// Run one turn to a terminal state.
///
/// `ProviderSession::next_event` must be cancel-safe: this loop drops it when a decision or a
/// cancel request arrives first.
pub async fn run_turn(
    ctx: TurnContext<'_>,
    session: &mut dyn ProviderSession,
    user_text: &str,
    inputs: TurnInputs,
) -> Result<(Turn, TurnOutcome), DriverError> {
    run_turn_with_images(ctx, session, user_text, &[], inputs).await
}

pub async fn run_turn_with_images(
    ctx: TurnContext<'_>,
    session: &mut dyn ProviderSession,
    user_text: &str,
    images: &[crate::attachments::Attachment],
    mut inputs: TurnInputs,
) -> Result<(Turn, TurnOutcome), DriverError> {
    let TurnContext { store, sequencer, session_id, policy_name, updates } = ctx;
    let mut turn = Turn::new(session_id.clone());
    lock(store).insert_turn(&turn)?;
    set_state(store, &mut turn, TurnState::Running)?;
    info!(session = %session_id, turn = %turn.id, "turn started");

    if let Err(e) = session.send_user_message_with_images(user_text, images).await {
        set_state(store, &mut turn, TurnState::Failed)?;
        return Err(e.into());
    }

    let mut pending: HashMap<ApprovalId, Approval> = HashMap::new();
    let (mut decisions_open, mut cancel_open) = (true, true);
    let mut saw_error = false;

    let outcome = loop {
        tokio::select! {
            next = session.next_event() => {
                let Some(raw) = next else {
                    warn!(session = %session_id, turn = %turn.id, "provider stream ended before the turn finished");
                    stale_all(store, &mut pending)?;
                    set_state(store, &mut turn, TurnState::Failed)?;
                    break TurnOutcome::Failed;
                };
                let event = sequencer.stamp(session_id, raw);
                lock(store).append_event(&event)?;
                let _ = updates.send(event.clone());
                match &event.kind {
                    EventKind::Error { .. } => saw_error = true,
                    EventKind::ApprovalRequest { approval_id, tool, input, .. } => {
                        let mut approval = Approval::new(turn.id.clone(), tool.clone(), input.clone());
                        approval.id = approval_id.clone();
                        lock(store).save_approval(&approval)?;
                        pending.insert(approval_id.clone(), approval);
                        if turn.state == TurnState::Running {
                            set_state(store, &mut turn, TurnState::AwaitingApproval)?;
                        }
                    }
                    EventKind::StatusChange { status: AgentStatus::Idle } => {
                        if turn.state == TurnState::AwaitingApproval {
                            set_state(store, &mut turn, TurnState::Running)?;
                        }
                        if saw_error {
                            stale_all(store, &mut pending)?;
                            set_state(store, &mut turn, TurnState::Failed)?;
                            break TurnOutcome::Failed;
                        }
                        set_state(store, &mut turn, TurnState::Completed)?;
                        break TurnOutcome::Completed;
                    }
                    _ => {}
                }
            }
            msg = inputs.decisions.recv(), if decisions_open => {
                let Some(msg) = msg else {
                    decisions_open = false;
                    continue;
                };
                let Some(approval) = pending.get_mut(&msg.approval_id) else {
                    warn!(approval = %msg.approval_id, "decision for unknown approval ignored");
                    continue;
                };
                match approval.decide(msg.decision, msg.decider, policy_name) {
                    Ok(DecisionOutcome::Applied) => {
                        // Record and audit first, then answer the provider.
                        lock(store).save_approval(approval)?;
                        session.respond_approval(&msg.approval_id, msg.decision).await?;
                        let any_pending = pending.values().any(|a| a.state == ApprovalState::Pending);
                        if !any_pending && turn.state == TurnState::AwaitingApproval {
                            set_state(store, &mut turn, TurnState::Running)?;
                        }
                    }
                    Ok(DecisionOutcome::Duplicate) => {
                        debug!(approval = %msg.approval_id, "duplicate decision ignored");
                    }
                    Err(e) => warn!(approval = %msg.approval_id, error = %e, "decision rejected"),
                }
            }
            req = inputs.cancel.recv(), if cancel_open => {
                if req.is_none() {
                    cancel_open = false;
                    continue;
                }
                session.cancel().await?;
                stale_all(store, &mut pending)?;
                set_state(store, &mut turn, TurnState::Cancelled)?;
                break TurnOutcome::Cancelled;
            }
        }
    };
    info!(session = %session_id, turn = %turn.id, ?outcome, "turn finished");
    Ok((turn, outcome))
}

#[allow(dead_code)]
fn _uses(_: Decision) {}
