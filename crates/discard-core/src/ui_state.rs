//! UI protocol and the state the views render. Pure logic: no GUI, no I/O.

pub use crate::workspace_session::{AutorunState, SessionProfile};

use crate::approval::{ApprovalState, Decision};
use crate::driver::TurnOutcome;
use crate::event::{AgentStatus, ApprovalId, Event};
use crate::model::{Block, ChannelId, Message, MessageId, Role};
use crate::timeline::{ApprovalPrompt, Timeline};

/// Whether a message continues the previous author's group.
pub fn continues_group(previous: Option<Role>, role: Role) -> bool { previous == Some(role) }

/// The first unread agent message after a known read marker.
pub fn first_unread_index(messages: &[Message], last_read: Option<&str>) -> Option<usize> {
    let start = match last_read { Some(id) => messages.iter().position(|message| message.id.0 == id)? + 1, None => 0 };
    messages.iter().enumerate().skip(start).find(|(_, message)| message.role == Role::Agent).map(|(index, _)| index)
}

/// The marker to save at startup, if any. A channel that already has history and no usable marker
/// (a first run of this feature, or a marker whose message is gone) counts as read, so old messages
/// do not show as unread. An empty history keeps `None`, so the first reply counts as unread.
pub fn startup_marker(messages: &[Message], last_read: Option<&str>) -> Option<String> {
    let known = last_read.is_some_and(|id| messages.iter().any(|message| message.id.0 == id));
    if known { None } else { messages.last().map(|message| message.id.0.clone()) }
}

/// Keep the first unread message anchored until the user sends or it leaves history.
pub fn new_divider_anchor(messages: &[Message], last_read: Option<&str>, anchor: Option<&str>, sent: bool) -> Option<String> {
    if sent { return None; }
    if let Some(id) = anchor {
        return messages.iter().any(|message| message.id.0 == id).then(|| id.to_owned());
    }
    first_unread_index(messages, last_read).map(|index| messages[index].id.0.clone())
}

/// From the UI to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Send(String),
    Continue { agent: crate::model::AgentId, note: String },
    SendAttachments { text: String, attachments: Vec<crate::attachments::Attachment> },
    Decide { approval_id: ApprovalId, decision: Decision, rule_cut: Option<usize> },
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
    ConversationMarker(Message),
    ResponseCompleted(Message),
    Responding(crate::model::MessageAuthor),
    SessionProfile(SessionProfile),
    /// The live reply remains available for copying; no saved response ID exists.
    ResponseSaveFailed(String),
    MemoryChanges { response: MessageId, changes: Vec<crate::memory_items::MemoryChange> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub session_profile: SessionProfile,
    channel_id: ChannelId,
    history: Vec<Message>,
    authors: std::collections::HashMap<MessageId, crate::model::MessageAuthor>,
    responder: Option<crate::model::MessageAuthor>,
    live: Timeline,
    turn_running: bool,
    response_save_failed: bool,
    notice: Option<String>,
}

impl UiState {
    pub fn new(channel_id: ChannelId, history: Vec<Message>) -> Self {
        Self { session_profile: SessionProfile::default(), channel_id, history, authors: Default::default(), responder: None, live: Timeline::new(), turn_running: false, response_save_failed: false, notice: None }
    }

    pub fn with_authors(mut self, authors: std::collections::HashMap<MessageId, crate::model::MessageAuthor>) -> Self { self.authors = authors; self }
    pub fn authors(&self) -> &std::collections::HashMap<MessageId, crate::model::MessageAuthor> { &self.authors }
    pub fn responder(&self) -> Option<&crate::model::MessageAuthor> { self.responder.as_ref() }

    /// The user pressed send.
    pub fn channel_id(&self) -> &ChannelId { &self.channel_id }

    pub fn response_started(&mut self) {
        self.turn_running = true;
        self.notice = None;
        self.response_save_failed = false;
    }

    pub fn user_sent(&mut self, text: &str) {
        self.user_sent_with_attachments(text, &[]);
    }
    pub fn user_sent_with_attachments(&mut self, text: &str, attachments: &[crate::attachments::Attachment]) {
        self.history.push(Message {
            id: MessageId::new(),
            channel_id: self.channel_id.clone(),
            role: Role::User,
            blocks: crate::attachments::blocks(text, attachments),
        });
        self.live = Timeline::new();
        self.turn_running = true;
        self.responder = None;
        self.response_save_failed = false;
        self.notice = None;
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::SessionProfile(profile) => self.session_profile = profile,
            Update::ResponseSaveFailed(message) => { self.response_save_failed = true; self.notice = Some(message); },
            Update::Responding(author) => self.responder = Some(author),
            Update::ResponseCompleted(message) => {
                if let Some(author) = &self.responder { self.authors.insert(message.id.clone(), author.clone()); }
                self.live = Timeline::new(); self.history.push(message);
            },
            Update::ConversationMarker(message) => self.history.push(message),
            Update::MemoryChanges { .. } => {},
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
        if self.response_save_failed { self.turn_running = false; return; }
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

    /// Current reply blocks, including an unsaved reply after a storage failure.
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
        // The responder can differ from the saved channel agent. Name the agent that owns the running turn.
        let (id, name, provider) = match self.responder.as_ref().filter(|_| self.turn_running) {
            Some(author) => (author.agent_id.0.as_str(), author.name.as_str(), author.provider.as_str()),
            None => (labels.agent_id.as_str(), labels.agent_name.as_str(), labels.provider.as_str()),
        };
        serde_json::json!({
            "project": labels.project,
            "channel": labels.channel,
            "pending_approvals": self.pending_approvals(),
            "turn_running": self.turn_running,
            "agent": { "id": id, "name": name, "provider": provider, "status": status },
            "agents": self.slot_agents(labels, id, status),
        })
    }

    /// Every agent in the conversation: the saved agent first, then the other agents in order of first reply.
    /// Only the agent that owns the running turn has a status other than "idle".
    fn slot_agents(&self, labels: &SlotLabels, owner_id: &str, owner_status: &str) -> Vec<serde_json::Value> {
        // (id, name, provider), in first-appearance order. A later reply updates the name and provider.
        let mut entries: Vec<(&str, &str, &str)> = vec![(labels.agent_id.as_str(), labels.agent_name.as_str(), labels.provider.as_str())];
        for message in &self.history {
            let Some(author) = self.authors.get(&message.id) else { continue };
            let id = author.agent_id.0.as_str();
            match entries.iter_mut().find(|entry| entry.0 == id) {
                Some(_) if id == labels.agent_id => {}
                Some(entry) => *entry = (id, author.name.as_str(), author.provider.as_str()),
                None => entries.push((id, author.name.as_str(), author.provider.as_str())),
            }
        }
        if let Some(author) = self.responder.as_ref().filter(|_| self.turn_running) {
            let id = author.agent_id.0.as_str();
            if !entries.iter().any(|entry| entry.0 == id) {
                entries.push((id, author.name.as_str(), author.provider.as_str()));
            }
        }
        entries
            .into_iter()
            .map(|(id, name, provider)| {
                let status = if self.turn_running && id == owner_id { owner_status } else { "idle" };
                serde_json::json!({ "id": id, "name": name, "provider": provider, "status": status })
            })
            .collect()
    }
}

/// Names the engine knows and the UI state does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLabels {
    pub project: String,
    pub channel: String,
    pub agent_id: String,
    pub agent_name: String,
    pub provider: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventId, EventKind, SessionId};

    #[test]
    fn unread_starts_after_known_read_message_and_only_counts_agents() {
        let channel = ChannelId::new();
        let make = |id: &str, role| Message { id: MessageId(id.into()), channel_id: channel.clone(), role, blocks: vec![] };
        let messages = vec![make("a", Role::User), make("b", Role::Agent), make("c", Role::Agent)];
        assert_eq!(first_unread_index(&[], Some("a")), None);
        assert_eq!(first_unread_index(&[], None), None);
        assert_eq!(first_unread_index(&messages[..1], None), None);
        assert_eq!(first_unread_index(&messages, None), Some(1));
        assert_eq!(first_unread_index(&messages, Some("missing")), None);
        assert_eq!(first_unread_index(&messages, Some("a")), Some(1));
        assert_eq!(first_unread_index(&messages, Some("c")), None);
    }

    #[test]
    fn startup_marks_old_history_as_read_but_keeps_an_empty_channel_unread() {
        let channel = ChannelId::new();
        let make = |id: &str, role| Message { id: MessageId(id.into()), channel_id: channel.clone(), role, blocks: vec![] };
        let messages = vec![make("a", Role::User), make("b", Role::Agent), make("c", Role::Agent)];
        assert_eq!(startup_marker(&[], None), None, "empty history: nothing to save");
        assert_eq!(startup_marker(&[], Some("a")), None, "empty history: nothing to save");
        assert_eq!(startup_marker(&messages, None), Some("c".into()), "no marker: all read");
        assert_eq!(startup_marker(&messages, Some("missing")), Some("c".into()), "unknown marker: all read");
        assert_eq!(startup_marker(&messages, Some("a")), None, "a known marker is kept");
        assert_eq!(startup_marker(&messages, Some("c")), None, "a known marker is kept");
    }

    #[test]
    fn divider_anchor_tracks_unread_until_send_or_removal() {
        let channel = ChannelId::new();
        let make = |id: &str, role| Message { id: MessageId(id.into()), channel_id: channel.clone(), role, blocks: vec![] };
        let messages = vec![make("a", Role::User), make("b", Role::Agent), make("c", Role::Agent)];
        assert_eq!(new_divider_anchor(&messages, Some("c"), None, false), None);
        assert_eq!(new_divider_anchor(&messages, Some("a"), None, false), Some("b".into()));
        assert_eq!(new_divider_anchor(&messages, Some("c"), Some("b"), false), Some("b".into()));
        assert_eq!(new_divider_anchor(&messages, Some("a"), Some("b"), true), None);
        // After a send, an older unread message must not become the anchor again.
        assert_eq!(new_divider_anchor(&messages, Some("b"), None, false), Some("c".into()));
        assert_eq!(new_divider_anchor(&messages[..1], Some("a"), Some("b"), false), None);
    }

    #[test]
    fn groups_only_adjacent_matching_roles() {
        assert!(!continues_group(None, Role::Agent));
        assert!(continues_group(Some(Role::Agent), Role::Agent));
        assert!(!continues_group(Some(Role::User), Role::Agent));
    }

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
        s.apply(ev(EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],
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
        s.apply(ev(EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],
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
        let labels = SlotLabels { project: "demo".into(), channel: "general".into(), agent_id: "agent-claude".into(), agent_name: "Claude".into(), provider: "claude".into() };
        assert_eq!(
            s.slot_state(&labels),
            serde_json::json!({
                "project": "demo", "channel": "general", "pending_approvals": 0,
                "turn_running": false,
                "agent": { "id": "agent-claude", "name": "Claude", "provider": "claude", "status": "idle" },
                "agents": [{ "id": "agent-claude", "name": "Claude", "provider": "claude", "status": "idle" }],
            })
        );
        s.user_sent("hi");
        s.apply(ev(EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],
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
    fn slot_state_names_the_responder_while_a_turn_runs() {
        let mut s = state();
        let labels = SlotLabels { project: "demo".into(), channel: "general".into(), agent_id: "agent-jordan".into(), agent_name: "Jordan".into(), provider: "codex".into() };
        s.user_sent("hi");
        s.apply(Update::Responding(crate::model::MessageAuthor { agent_id: crate::model::AgentId("agent-casey".into()), name: "Casey".into(), provider: "claude".into(), avatar: None }));
        assert_eq!(s.slot_state(&labels)["agent"], serde_json::json!({ "id": "agent-casey", "name": "Casey", "provider": "claude", "status": "working" }));
    }

    fn author(id: &str, name: &str, provider: &str) -> crate::model::MessageAuthor {
        crate::model::MessageAuthor { agent_id: crate::model::AgentId(id.into()), name: name.into(), provider: provider.into(), avatar: None }
    }

    fn reply(s: &mut UiState, id: &str, by: crate::model::MessageAuthor) {
        s.apply(Update::Responding(by));
        let message = Message { id: MessageId(id.into()), channel_id: s.channel_id().clone(), role: Role::Agent, blocks: vec![] };
        s.apply(Update::ResponseCompleted(message));
    }

    fn jordan_labels() -> SlotLabels {
        SlotLabels { project: "demo".into(), channel: "general".into(), agent_id: "agent-jordan".into(), agent_name: "Jordan".into(), provider: "codex".into() }
    }

    #[test]
    fn slot_state_lists_every_agent_once_with_the_saved_agent_first() {
        let mut s = state();
        reply(&mut s, "m1", author("agent-casey", "Casey", "claude"));
        reply(&mut s, "m2", author("agent-jordan", "Jordan Old Name", "claude"));
        reply(&mut s, "m3", author("agent-riley", "Riley", "codex"));
        reply(&mut s, "m4", author("agent-casey", "Casey B", "codex"));
        assert_eq!(
            s.slot_state(&jordan_labels())["agents"],
            serde_json::json!([
                { "id": "agent-jordan", "name": "Jordan", "provider": "codex", "status": "idle" },
                { "id": "agent-casey", "name": "Casey B", "provider": "codex", "status": "idle" },
                { "id": "agent-riley", "name": "Riley", "provider": "codex", "status": "idle" },
            ])
        );
    }

    #[test]
    fn slot_state_marks_only_the_responder_as_working_while_a_turn_runs() {
        let mut s = state();
        reply(&mut s, "m1", author("agent-jordan", "Jordan", "codex"));
        reply(&mut s, "m2", author("agent-casey", "Casey", "claude"));
        s.user_sent("again");
        s.apply(Update::Responding(author("agent-casey", "Casey", "claude")));
        let v = s.slot_state(&jordan_labels());
        assert_eq!(
            v["agents"],
            serde_json::json!([
                { "id": "agent-jordan", "name": "Jordan", "provider": "codex", "status": "idle" },
                { "id": "agent-casey", "name": "Casey", "provider": "claude", "status": "working" },
            ])
        );
        assert_eq!(v["agent"]["id"], "agent-casey");
    }

    #[test]
    fn slot_state_marks_the_saved_agent_as_working_before_a_responder_is_known() {
        let mut s = state();
        s.user_sent("hi");
        assert_eq!(
            s.slot_state(&jordan_labels())["agents"],
            serde_json::json!([{ "id": "agent-jordan", "name": "Jordan", "provider": "codex", "status": "working" }])
        );
    }

    #[test]
    fn slot_state_appends_a_responder_that_has_not_replied_yet() {
        let mut s = state();
        reply(&mut s, "m1", author("agent-riley", "Riley", "codex"));
        s.user_sent("hi");
        s.apply(Update::Responding(author("agent-casey", "Casey", "claude")));
        assert_eq!(
            s.slot_state(&jordan_labels())["agents"],
            serde_json::json!([
                { "id": "agent-jordan", "name": "Jordan", "provider": "codex", "status": "idle" },
                { "id": "agent-riley", "name": "Riley", "provider": "codex", "status": "idle" },
                { "id": "agent-casey", "name": "Casey", "provider": "claude", "status": "working" },
            ])
        );
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
