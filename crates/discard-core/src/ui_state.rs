//! UI protocol and the state the views render. Pure logic: no GPUI, no I/O.

use crate::approval::{ApprovalState, Decision};
use crate::driver::TurnOutcome;
use crate::event::{AgentStatus, ApprovalId, Event};
use crate::model::{Block, ChannelId, Message, MessageId, Role};
use crate::timeline::{ApprovalPrompt, Timeline};

/// From the UI to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Send(String),
    Decide { approval_id: ApprovalId, decision: Decision },
    Cancel,
}

/// From the engine to the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Event(Box<Event>),
    ApprovalState { approval_id: ApprovalId, state: ApprovalState },
    TurnFinished(TurnOutcome),
    /// The provider CLI needs setup. The text names the exact command the user must run.
    SetupNeeded(String),
    Error(String),
    /// Something the user should know that does not stop the turn.
    Notice(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    channel_id: ChannelId,
    history: Vec<Message>,
    live: Timeline,
    turn_running: bool,
    notice: Option<String>,
}

impl UiState {
    pub fn new(channel_id: ChannelId, history: Vec<Message>) -> Self {
        Self { channel_id, history, live: Timeline::new(), turn_running: false, notice: None }
    }

    /// The user pressed send.
    pub fn user_sent(&mut self, text: &str) {
        self.history.push(Message {
            id: MessageId::new(),
            channel_id: self.channel_id.clone(),
            role: Role::User,
            blocks: vec![Block::Markdown { text: text.to_string() }],
        });
        self.live = Timeline::new();
        self.turn_running = true;
        self.notice = None;
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Event(event) => self.live.apply(&event.kind),
            Update::ApprovalState { approval_id, state } => self.live.mark_approval(&approval_id, state),
            Update::TurnFinished(_) => self.finish_turn(),
            Update::SetupNeeded(text) => {
                self.finish_turn();
                self.notice = Some(text);
            }
            Update::Error(text) | Update::Notice(text) => self.notice = Some(text),
        }
    }

    /// Move the live reply into history. Approvals that were never answered can no longer be.
    fn finish_turn(&mut self) {
        let unanswered: Vec<ApprovalId> = self
            .live
            .prompts()
            .iter()
            .filter(|p| p.view == crate::timeline::ApprovalView::Pending)
            .map(|p| p.approval_id.clone())
            .collect();
        for id in unanswered {
            self.live.mark_approval(&id, ApprovalState::Stale);
        }
        let live = std::mem::take(&mut self.live);
        if !live.blocks().is_empty() {
            self.history.push(live.into_message(self.channel_id.clone()));
        }
        self.turn_running = false;
    }

    /// Finished messages, oldest first.
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Blocks of the reply that is still streaming.
    pub fn live_blocks(&self) -> &[Block] {
        self.live.blocks()
    }

    pub fn prompts(&self) -> &[ApprovalPrompt] {
        self.live.prompts()
    }

    pub fn agent_status(&self) -> AgentStatus {
        if !self.turn_running {
            AgentStatus::Idle
        } else if self.live.pending_approvals() > 0 {
            AgentStatus::AwaitingApproval
        } else {
            AgentStatus::Working
        }
    }

    pub fn turn_running(&self) -> bool {
        self.turn_running
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Count for the inbox badge.
    pub fn pending_approvals(&self) -> usize {
        self.live.pending_approvals()
    }

    /// The state a plugin's `render(state)` function receives for UI slots.
    pub fn slot_state(&self, labels: &SlotLabels) -> serde_json::Value {
        let status = match self.agent_status() {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::AwaitingApproval => "awaiting_approval",
            AgentStatus::Offline => "offline",
        };
        serde_json::json!({
            "project": labels.project,
            "channel": labels.channel,
            "pending_approvals": self.pending_approvals(),
            "turn_running": self.turn_running,
            "agent": { "name": labels.agent_name, "provider": labels.provider, "status": status },
        })
    }
}

/// Names the engine knows and the UI state does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLabels {
    pub project: String,
    pub channel: String,
    pub agent_name: String,
    pub provider: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventId, EventKind, SessionId};

    fn ev(kind: EventKind) -> Update {
        Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 1, kind, raw: None }))
    }

    fn state() -> UiState {
        UiState::new(ChannelId::new(), vec![])
    }

    fn text(s: &str) -> Update {
        ev(EventKind::TextDelta { text: s.into() })
    }

    #[test]
    fn sending_adds_a_user_message_and_starts_a_turn() {
        let mut s = state();
        s.user_sent("hi");
        assert!(s.turn_running());
        assert_eq!(s.history().len(), 1);
        assert_eq!(s.history()[0].role, Role::User);
        assert_eq!(s.history()[0].blocks, vec![Block::Markdown { text: "hi".into() }]);
    }

    #[test]
    fn streamed_events_build_the_live_reply() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(text("Hel"));
        s.apply(text("lo"));
        assert_eq!(s.live_blocks(), &[Block::Markdown { text: "Hello".into() }]);
    }

    #[test]
    fn finishing_moves_the_live_reply_into_history() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(text("Hello"));
        s.apply(Update::TurnFinished(TurnOutcome::Completed));
        assert!(!s.turn_running());
        assert!(s.live_blocks().is_empty());
        assert_eq!(s.history().len(), 2);
        assert_eq!(s.history()[1].role, Role::Agent);
        assert_eq!(s.history()[1].blocks, vec![Block::Markdown { text: "Hello".into() }]);
    }

    #[test]
    fn an_empty_reply_adds_no_message() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(Update::TurnFinished(TurnOutcome::Cancelled));
        assert_eq!(s.history().len(), 1);
    }

    #[test]
    fn approval_prompts_are_pending_until_the_engine_reports_a_decision() {
        let mut s = state();
        s.user_sent("hi");
        let id = ApprovalId::new();
        s.apply(ev(EventKind::ApprovalRequest {
            approval_id: id.clone(),
            call_id: "c".into(),
            tool: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        }));
        assert_eq!(s.pending_approvals(), 1);
        s.apply(Update::ApprovalState { approval_id: id, state: ApprovalState::Decided(Decision::Approve) });
        assert_eq!(s.pending_approvals(), 0);
    }

    #[test]
    fn finishing_a_turn_stales_any_approval_still_pending() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(ev(EventKind::ApprovalRequest {
            approval_id: ApprovalId::new(),
            call_id: "c".into(),
            tool: "Bash".into(),
            input: serde_json::json!({}),
        }));
        s.apply(Update::TurnFinished(TurnOutcome::Failed));
        assert_eq!(s.pending_approvals(), 0);
        let Block::Approval { .. } = &s.history()[1].blocks[0] else { panic!("the prompt block is kept in history") };
    }

    #[test]
    fn status_follows_events_and_resets_after_the_turn() {
        let mut s = state();
        assert_eq!(s.agent_status(), AgentStatus::Idle);
        s.user_sent("hi");
        s.apply(ev(EventKind::StatusChange { status: AgentStatus::Working }));
        assert_eq!(s.agent_status(), AgentStatus::Working);
        s.apply(Update::TurnFinished(TurnOutcome::Completed));
        assert_eq!(s.agent_status(), AgentStatus::Idle);
    }

    #[test]
    fn a_running_turn_shows_as_working_before_the_first_event() {
        let mut s = state();
        s.user_sent("hi");
        assert_eq!(s.agent_status(), AgentStatus::Working);
    }

    #[test]
    fn setup_and_error_updates_set_a_notice_that_clears_on_the_next_send() {
        let mut s = state();
        s.apply(Update::SetupNeeded("Run `claude auth login`".into()));
        assert_eq!(s.notice(), Some("Run `claude auth login`"));
        s.apply(Update::Error("boom".into()));
        assert_eq!(s.notice(), Some("boom"));
        s.user_sent("retry");
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn slot_state_describes_the_channel_agent_and_pending_approvals() {
        let mut s = state();
        let labels = SlotLabels { project: "demo".into(), channel: "general".into(), agent_name: "Claude".into(), provider: "claude".into() };
        assert_eq!(
            s.slot_state(&labels),
            serde_json::json!({
                "project": "demo", "channel": "general", "pending_approvals": 0,
                "turn_running": false,
                "agent": { "name": "Claude", "provider": "claude", "status": "idle" },
            })
        );
        s.user_sent("hi");
        s.apply(ev(EventKind::ApprovalRequest {
            approval_id: ApprovalId::new(),
            call_id: "c".into(),
            tool: "Bash".into(),
            input: serde_json::json!({}),
        }));
        let v = s.slot_state(&labels);
        assert_eq!(v["pending_approvals"], 1);
        assert_eq!(v["agent"]["status"], "awaiting_approval");
        assert_eq!(v["turn_running"], true);
    }

    #[test]
    fn a_notice_is_shown_without_ending_the_turn() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(Update::Notice("Uncommitted changes were not copied".into()));
        assert_eq!(s.notice(), Some("Uncommitted changes were not copied"));
        assert!(s.turn_running(), "a notice is information, not the end of the turn");
        s.apply(text("still streaming"));
        assert_eq!(s.live_blocks().len(), 1);
    }

    #[test]
    fn setup_needed_ends_the_turn_that_was_waiting() {
        let mut s = state();
        s.user_sent("hi");
        s.apply(Update::SetupNeeded("login".into()));
        assert!(!s.turn_running());
    }
}
