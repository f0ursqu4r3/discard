//! Rules for desktop notifications. This module has no GUI dependency.

use discard_core::driver::TurnOutcome;
use discard_core::ui_state::Update;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

/// How long after a notification a window focus still opens its channel.
pub const FOLLOW_WINDOW: Duration = Duration::from_secs(10);
const MAX_NAME_CHARS: usize = 60;
const SEEN_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    Approval { id: String, tool: String },
    Finished(TurnOutcome),
}

/// Sends a notification to the operating system.
pub trait Notifier: Send + Sync {
    fn notify(&self, notification: &Notification);
}

/// Finds the trigger in an update. A thread turn never triggers. A thread approval does.
pub fn trigger_of(update: &Update, is_thread: bool) -> Option<Trigger> {
    match update {
        // The request event alone is not enough: a policy may approve it. The engine sends this update only
        // when the user must decide.
        Update::UserApprovalNeeded { approval_id, tool } => Some(Trigger::Approval { id: approval_id.0.clone(), tool: tool.clone() }),
        Update::TurnFinished(outcome) if !is_thread => Some(Trigger::Finished(*outcome)),
        _ => None,
    }
}

/// Builds the notification for a trigger. Returns `None` when nothing should show.
pub fn compose(trigger: &Trigger, channel: &str, agent: &str, muted: bool) -> Option<Notification> {
    let body = match trigger {
        Trigger::Approval { tool, .. } => format!("Needs approval: {}", one_line(tool)),
        Trigger::Finished(_) if muted => return None,
        Trigger::Finished(TurnOutcome::Completed) => "Finished".to_string(),
        Trigger::Finished(TurnOutcome::Failed) => "Failed".to_string(),
        Trigger::Finished(TurnOutcome::Cancelled) => return None,
    };
    Some(Notification { title: format!("{} · {}", one_line(channel), one_line(agent)), body })
}

/// Joins whitespace runs into one space, trims, and cuts to 60 characters with `…`.
fn one_line(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_NAME_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(MAX_NAME_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Tracks which windows have focus and the newest notification.
#[derive(Debug, Default)]
pub struct FocusTracker {
    focused: HashSet<String>,
    last: Option<(String, Instant)>,
}

impl FocusTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_focus(&mut self, label: &str, focused: bool) {
        if focused {
            self.focused.insert(label.to_string());
        } else {
            self.focused.remove(label);
        }
    }

    /// A closed window sends no focus-lost event. Drop its focus here.
    pub fn window_closed(&mut self, label: &str) {
        self.focused.remove(label);
    }

    pub fn app_focused(&self) -> bool {
        !self.focused.is_empty()
    }

    /// Remembers the newest notification. It replaces any earlier one.
    pub fn notified(&mut self, channel_id: &str, now: Instant) {
        self.last = Some((channel_id.to_string(), now));
    }

    /// Returns the newest notification's channel if it is recent. Always clears it.
    pub fn take_follow(&mut self, now: Instant) -> Option<String> {
        let (channel, at) = self.last.take()?;
        (now.saturating_duration_since(at) <= FOLLOW_WINDOW).then_some(channel)
    }
}

/// Remembers the last 128 keys, so one approval notifies once.
#[derive(Debug, Default)]
pub struct Seen {
    order: VecDeque<String>,
    keys: HashSet<String>,
}

impl Seen {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true the first time a key appears.
    pub fn first(&mut self, key: &str) -> bool {
        if !self.keys.insert(key.to_string()) {
            return false;
        }
        self.order.push_back(key.to_string());
        if self.order.len() > SEEN_CAPACITY
            && let Some(old) = self.order.pop_front()
        {
            self.keys.remove(&old);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::driver::TurnOutcome;
    use discard_core::event::{ApprovalId, Event, EventId, EventKind, SessionId};
    use discard_core::ui_state::Update;
    use std::time::{Duration, Instant};

    fn approval_update(tool: &str, input: serde_json::Value) -> Update {
        Update::Event(Box::new(Event {
            id: EventId::new(),
            session_id: SessionId::new(),
            seq: 1,
            raw: None,
            kind: EventKind::ApprovalRequest {
                kind: discard_core::approval::ApprovalKind::ProviderTool,
                approval_id: ApprovalId("ap-1".into()),
                call_id: "shell".into(),
                tool: tool.into(),
                input,
                paths: vec![],
            },
        }))
    }

    fn needed_update(tool: &str) -> Update {
        Update::UserApprovalNeeded { approval_id: ApprovalId("ap-1".into()), tool: tool.into() }
    }

    fn approval(tool: &str) -> Trigger {
        Trigger::Approval { id: "ap-1".into(), tool: tool.into() }
    }

    #[test]
    fn an_approval_composes_title_and_body() {
        let n = compose(&approval("Bash"), "general", "Ada", false).unwrap();
        assert_eq!(n.title, "general · Ada");
        assert_eq!(n.body, "Needs approval: Bash");
    }

    #[test]
    fn a_finished_turn_says_finished_and_a_failed_turn_says_failed() {
        let done = compose(&Trigger::Finished(TurnOutcome::Completed), "general", "Ada", false).unwrap();
        assert_eq!(done.body, "Finished");
        let failed = compose(&Trigger::Finished(TurnOutcome::Failed), "general", "Ada", false).unwrap();
        assert_eq!(failed.body, "Failed");
    }

    #[test]
    fn a_cancelled_turn_composes_nothing() {
        assert!(compose(&Trigger::Finished(TurnOutcome::Cancelled), "general", "Ada", false).is_none());
    }

    #[test]
    fn a_muted_channel_drops_finished_but_keeps_approvals() {
        assert!(compose(&Trigger::Finished(TurnOutcome::Completed), "general", "Ada", true).is_none());
        assert!(compose(&Trigger::Finished(TurnOutcome::Failed), "general", "Ada", true).is_none());
        assert!(compose(&approval("Bash"), "general", "Ada", true).is_some());
    }

    #[test]
    fn a_thread_turn_gives_no_trigger_but_a_thread_approval_does() {
        let finished = Update::TurnFinished(TurnOutcome::Completed);
        assert_eq!(trigger_of(&finished, false), Some(Trigger::Finished(TurnOutcome::Completed)));
        assert_eq!(trigger_of(&finished, true), None);
        let needed = needed_update("Bash");
        assert_eq!(trigger_of(&needed, true), Some(approval("Bash")));
        assert_eq!(trigger_of(&needed, false), Some(approval("Bash")));
    }

    #[test]
    fn an_approval_request_event_alone_gives_no_trigger() {
        let request = approval_update("Bash", serde_json::json!({}));
        assert_eq!(trigger_of(&request, false), None);
        assert_eq!(trigger_of(&request, true), None);
    }

    #[test]
    fn other_updates_give_no_trigger() {
        assert_eq!(trigger_of(&Update::Notice("hi".into()), false), None);
        assert_eq!(trigger_of(&Update::Error("bad".into()), false), None);
        assert_eq!(trigger_of(&Update::ThreadFollowed, false), None);
        let delta = Update::Event(Box::new(Event {
            id: EventId::new(),
            session_id: SessionId::new(),
            seq: 1,
            raw: None,
            kind: EventKind::TextDelta { text: "hi".into() },
        }));
        assert_eq!(trigger_of(&delta, false), None);
    }

    #[test]
    fn names_with_newlines_or_500_characters_become_one_short_line() {
        let n = compose(&approval("a\n\tb   c"), "gen\neral", "Ada\r\nL", false).unwrap();
        assert_eq!(n.title, "gen eral · Ada L");
        assert_eq!(n.body, "Needs approval: a b c");
        let long = "x".repeat(500);
        let n = compose(&approval(&long), &long, &long, false).unwrap();
        let name = format!("{}…", "x".repeat(59));
        assert_eq!(name.chars().count(), 60);
        assert_eq!(n.title, format!("{name} · {name}"));
        assert_eq!(n.body, format!("Needs approval: {name}"));
    }

    #[test]
    fn the_body_holds_only_the_tool_name_never_the_input() {
        let update = needed_update("Bash");
        let trigger = trigger_of(&update, false).unwrap();
        let n = compose(&trigger, "general", "Ada", false).unwrap();
        assert!(!n.title.contains("secret") && !n.body.contains("secret"));
    }

    #[test]
    fn focus_tracker_reports_focus_while_any_window_is_focused() {
        let mut focus = FocusTracker::new();
        assert!(!focus.app_focused());
        focus.set_focus("main", true);
        focus.set_focus("project-a", true);
        assert!(focus.app_focused());
        focus.set_focus("main", false);
        assert!(focus.app_focused());
        focus.set_focus("project-a", false);
        assert!(!focus.app_focused());
    }

    #[test]
    fn a_closed_window_leaves_the_focus_set() {
        let mut focus = FocusTracker::new();
        focus.set_focus("project-a", true);
        focus.window_closed("project-a");
        assert!(!focus.app_focused());
    }

    #[test]
    fn a_focus_within_ten_seconds_follows_the_newest_notification_once() {
        let mut focus = FocusTracker::new();
        let t0 = Instant::now();
        focus.notified("a", t0);
        focus.notified("b", t0 + Duration::from_secs(2));
        assert_eq!(focus.take_follow(t0 + Duration::from_secs(9)), Some("b".into()));
        assert_eq!(focus.take_follow(t0 + Duration::from_secs(9)), None);
    }

    #[test]
    fn a_focus_after_ten_seconds_follows_nothing() {
        let mut focus = FocusTracker::new();
        let t0 = Instant::now();
        focus.notified("a", t0);
        assert_eq!(focus.take_follow(t0 + FOLLOW_WINDOW + Duration::from_millis(1)), None);
        assert_eq!(focus.take_follow(t0 + Duration::from_secs(1)), None, "a late focus still clears it");
    }

    #[test]
    fn seen_reports_a_key_once_and_forgets_after_128_newer_keys() {
        let mut seen = Seen::new();
        assert!(seen.first("k"));
        assert!(!seen.first("k"));
        for i in 0..127 {
            assert!(seen.first(&format!("n{i}")));
        }
        assert!(!seen.first("k"), "127 newer keys keep it");
        assert!(seen.first("n127"));
        assert!(seen.first("k"), "128 newer keys drop it");
    }
}
