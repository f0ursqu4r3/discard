//! Durable turn state machine.

use crate::event::{SessionId, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    Queued,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid turn transition {from:?} -> {to:?}")]
pub struct TransitionError {
    pub from: TurnState,
    pub to: TurnState,
}

impl TurnState {
    pub fn is_terminal(self) -> bool {
        matches!(self, TurnState::Completed | TurnState::Failed | TurnState::Cancelled)
    }

    pub fn can_transition_to(self, to: TurnState) -> bool {
        use TurnState::*;
        match (self, to) {
            (Queued, Running) => true,
            (Running, AwaitingApproval | Completed) => true,
            (AwaitingApproval, Running) => true,
            (from, Failed | Cancelled) => !from.is_terminal(),
            _ => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TurnState::Queued => "queued",
            TurnState::Running => "running",
            TurnState::AwaitingApproval => "awaiting_approval",
            TurnState::Completed => "completed",
            TurnState::Failed => "failed",
            TurnState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<TurnState> {
        Some(match s {
            "queued" => TurnState::Queued,
            "running" => TurnState::Running,
            "awaiting_approval" => TurnState::AwaitingApproval,
            "completed" => TurnState::Completed,
            "failed" => TurnState::Failed,
            "cancelled" => TurnState::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: TurnId,
    pub session_id: SessionId,
    pub state: TurnState,
}

impl Turn {
    pub fn new(session_id: SessionId) -> Self {
        Self { id: TurnId::new(), session_id, state: TurnState::Queued }
    }

    pub fn transition(&mut self, to: TurnState) -> Result<(), TransitionError> {
        if !self.state.can_transition_to(to) {
            return Err(TransitionError { from: self.state, to });
        }
        self.state = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TurnState::*;

    const ALL: [TurnState; 6] = [Queued, Running, AwaitingApproval, Completed, Failed, Cancelled];

    #[test]
    fn happy_path_queued_running_completed() {
        let mut t = Turn::new(SessionId::new());
        t.transition(Running).unwrap();
        t.transition(Completed).unwrap();
        assert_eq!(t.state, Completed);
    }

    #[test]
    fn running_can_wait_for_approval_and_resume() {
        let mut t = Turn::new(SessionId::new());
        t.transition(Running).unwrap();
        t.transition(AwaitingApproval).unwrap();
        t.transition(Running).unwrap();
    }

    #[test]
    fn terminal_states_accept_no_transitions() {
        for from in [Completed, Failed, Cancelled] {
            assert!(from.is_terminal());
            for to in ALL {
                assert!(!from.can_transition_to(to), "{from:?} -> {to:?} must be rejected");
            }
        }
    }

    #[test]
    fn queued_cannot_skip_to_completed() {
        let mut t = Turn::new(SessionId::new());
        assert_eq!(t.transition(Completed), Err(TransitionError { from: Queued, to: Completed }));
        assert_eq!(t.state, Queued, "failed transition must not change state");
    }

    #[test]
    fn every_non_terminal_state_can_be_cancelled_or_failed() {
        for from in [Queued, Running, AwaitingApproval] {
            assert!(!from.is_terminal());
            assert!(from.can_transition_to(Cancelled));
            assert!(from.can_transition_to(Failed));
        }
    }

    #[test]
    fn state_string_round_trip() {
        for s in ALL {
            assert_eq!(TurnState::parse(s.as_str()), Some(s));
        }
        assert_eq!(TurnState::parse("bogus"), None);
    }
}
