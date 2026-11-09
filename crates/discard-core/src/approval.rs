//! Approval state machine: idempotent decisions, stale handling, audit records.

use crate::event::{ApprovalId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    #[default]
    ProviderTool, SandboxEscape, NetworkDomain, MainCheckoutWrite, WorktreeCreate, ConversationMove, BranchCreate,
}
impl ApprovalKind {
    pub fn as_str(self) -> &'static str { match self {
        Self::ProviderTool => "provider_tool", Self::SandboxEscape => "sandbox_escape", Self::NetworkDomain => "network_domain",
        Self::MainCheckoutWrite => "main_checkout_write", Self::WorktreeCreate => "worktree_create", Self::ConversationMove => "conversation_move", Self::BranchCreate => "branch_create",
    } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Deny,
    AlwaysAllow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Decided(Decision),
    /// The provider process exited before a decision. It can no longer be approved.
    Stale,
}

/// Who or what made a decision. Recorded in the audit table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decider {
    User,
    Policy(String),
    /// A plugin hook with the `approvals.decide` capability.
    Plugin(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// The decision was applied. The caller must now answer the provider.
    Applied,
    /// The same decision was already recorded. Nothing changed. Do not answer the provider again.
    Duplicate,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecisionError {
    #[error("{0}")]
    NotPermitted(&'static str),
    #[error("approval is stale: the provider process exited")]
    Stale,
    #[error("approval already decided as {existing:?}; refusing {attempted:?}")]
    Conflict { existing: Decision, attempted: Decision },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    pub approval_id: ApprovalId,
    pub decision: Decision,
    pub decider: Decider,
    pub policy: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub kind: ApprovalKind,
    pub id: ApprovalId,
    pub turn_id: TurnId,
    pub tool: String,
    pub input: Value,
    pub state: ApprovalState,
    pub audit: Vec<AuditRecord>,
}

impl Approval {
    pub fn new(turn_id: TurnId, tool: impl Into<String>, input: Value) -> Self {
        Self {
            kind: ApprovalKind::ProviderTool,
            id: ApprovalId::new(),
            turn_id,
            tool: tool.into(),
            input,
            state: ApprovalState::Pending,
            audit: Vec::new(),
        }
    }

    pub fn decide(
        &mut self,
        decision: Decision,
        decider: Decider,
        policy: &str,
    ) -> Result<DecisionOutcome, DecisionError> {
        if self.kind != ApprovalKind::ProviderTool {
            if !matches!(decider, Decider::User) { return Err(DecisionError::NotPermitted("Only the user can decide this approval.")); }
            if decision == Decision::AlwaysAllow { return Err(DecisionError::NotPermitted("Always allow is unavailable for this approval.")); }
        }
        match self.state {
            ApprovalState::Stale => Err(DecisionError::Stale),
            ApprovalState::Decided(existing) if existing == decision => Ok(DecisionOutcome::Duplicate),
            ApprovalState::Decided(existing) => {
                Err(DecisionError::Conflict { existing, attempted: decision })
            }
            ApprovalState::Pending => {
                self.state = ApprovalState::Decided(decision);
                self.audit.push(AuditRecord {
                    approval_id: self.id.clone(),
                    decision,
                    decider,
                    policy: policy.to_string(),
                });
                Ok(DecisionOutcome::Applied)
            }
        }
    }

    pub fn mark_stale(&mut self) {
        if self.state == ApprovalState::Pending {
            self.state = ApprovalState::Stale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugins_decide_only_provider_tool_approvals() {
        let mut escape = pending(); escape.kind = ApprovalKind::SandboxEscape;
        assert!(matches!(escape.decide(Decision::Approve, Decider::Plugin("p".into()), "ask"), Err(DecisionError::NotPermitted(_))));
        assert!(pending().decide(Decision::Approve, Decider::Plugin("p".into()), "ask").is_ok());
    }
    #[test]
    fn always_allow_is_refused_for_escapes_and_app_grants() {
        for kind in [ApprovalKind::SandboxEscape, ApprovalKind::NetworkDomain, ApprovalKind::MainCheckoutWrite, ApprovalKind::WorktreeCreate, ApprovalKind::ConversationMove, ApprovalKind::BranchCreate] {
            let mut a = pending(); a.kind = kind;
            assert!(matches!(a.decide(Decision::AlwaysAllow, Decider::User, "ask"), Err(DecisionError::NotPermitted(_))));
        }
    }

    fn pending() -> Approval {
        Approval::new(TurnId::new(), "Bash", json!({"command": "touch x"}))
    }

    #[test]
    fn first_decision_applies_and_writes_one_audit_record() {
        let mut a = pending();
        let out = a.decide(Decision::Approve, Decider::User, "ask").unwrap();
        assert_eq!(out, DecisionOutcome::Applied);
        assert_eq!(a.state, ApprovalState::Decided(Decision::Approve));
        assert_eq!(a.audit.len(), 1);
        assert_eq!(a.audit[0].decider, Decider::User);
        assert_eq!(a.audit[0].policy, "ask");
    }

    #[test]
    fn repeating_the_same_decision_is_a_no_op() {
        let mut a = pending();
        a.decide(Decision::Approve, Decider::User, "ask").unwrap();
        let out = a.decide(Decision::Approve, Decider::User, "ask").unwrap();
        assert_eq!(out, DecisionOutcome::Duplicate);
        assert_eq!(a.audit.len(), 1, "duplicates must not add audit rows");
    }

    #[test]
    fn a_conflicting_second_decision_is_rejected() {
        let mut a = pending();
        a.decide(Decision::Approve, Decider::User, "ask").unwrap();
        let err = a.decide(Decision::Deny, Decider::User, "ask").unwrap_err();
        assert_eq!(err, DecisionError::Conflict { existing: Decision::Approve, attempted: Decision::Deny });
        assert_eq!(a.state, ApprovalState::Decided(Decision::Approve));
    }

    #[test]
    fn stale_approvals_cannot_be_decided() {
        let mut a = pending();
        a.mark_stale();
        assert_eq!(a.decide(Decision::Approve, Decider::User, "ask"), Err(DecisionError::Stale));
        assert_eq!(a.state, ApprovalState::Stale);
        assert!(a.audit.is_empty());
    }

    #[test]
    fn marking_stale_does_not_undo_a_made_decision() {
        let mut a = pending();
        a.decide(Decision::Deny, Decider::User, "ask").unwrap();
        a.mark_stale();
        assert_eq!(a.state, ApprovalState::Decided(Decision::Deny));
    }

    #[test]
    fn plugin_and_policy_deciders_are_audited_by_name() {
        let mut a = pending();
        a.decide(Decision::Approve, Decider::Plugin("auto-ok".into()), "ask").unwrap();
        assert_eq!(a.audit[0].decider, Decider::Plugin("auto-ok".into()));
    }
}
