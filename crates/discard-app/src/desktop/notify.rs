//! Rules for desktop notifications. This module has no GUI dependency.

use discard_core::driver::TurnOutcome;
use discard_core::ui_state::Update;
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_NAME_CHARS: usize = 60;
const SEEN_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// The channel a click on the notification opens. For a thread, this is the parent channel.
    pub channel_id: String,
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
pub fn compose(trigger: &Trigger, channel: &str, agent: &str, muted: bool, channel_id: &str) -> Option<Notification> {
    let body = match trigger {
        Trigger::Approval { tool, .. } => format!("Needs approval: {}", one_line(tool)),
        Trigger::Finished(_) if muted => return None,
        Trigger::Finished(TurnOutcome::Completed) => "Finished".to_string(),
        Trigger::Finished(TurnOutcome::Failed) => "Failed".to_string(),
        Trigger::Finished(TurnOutcome::Cancelled) => return None,
    };
    Some(Notification { title: format!("{} · {}", one_line(channel), one_line(agent)), body, channel_id: channel_id.to_string() })
}

/// The channel a click on a notification opens. Only a `tap` counts, and only when the notification carries a
/// non-empty string `channelId`.
pub fn channel_from_click(action_id: &str, extra: &HashMap<String, serde_json::Value>) -> Option<String> {
    if action_id != "tap" {
        return None;
    }
    extra.get("channelId").and_then(serde_json::Value::as_str).filter(|id| !id.is_empty()).map(str::to_string)
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

/// Tracks which windows have focus.
#[derive(Debug, Default)]
pub struct FocusTracker {
    focused: HashSet<String>,
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

    /// Forgets every focused window.
    pub fn clear(&mut self) {
        self.focused.clear();
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
        let n = compose(&approval("Bash"), "general", "Ada", false, "chan-1").unwrap();
        assert_eq!(n.title, "general · Ada");
        assert_eq!(n.body, "Needs approval: Bash");
    }

    #[test]
    fn a_finished_turn_says_finished_and_a_failed_turn_says_failed() {
        let done = compose(&Trigger::Finished(TurnOutcome::Completed), "general", "Ada", false, "chan-1").unwrap();
        assert_eq!(done.body, "Finished");
        let failed = compose(&Trigger::Finished(TurnOutcome::Failed), "general", "Ada", false, "chan-1").unwrap();
        assert_eq!(failed.body, "Failed");
    }

    #[test]
    fn a_cancelled_turn_composes_nothing() {
        assert!(compose(&Trigger::Finished(TurnOutcome::Cancelled), "general", "Ada", false, "chan-1").is_none());
    }

    #[test]
    fn a_muted_channel_drops_finished_but_keeps_approvals() {
        assert!(compose(&Trigger::Finished(TurnOutcome::Completed), "general", "Ada", true, "chan-1").is_none());
        assert!(compose(&Trigger::Finished(TurnOutcome::Failed), "general", "Ada", true, "chan-1").is_none());
        assert!(compose(&approval("Bash"), "general", "Ada", true, "chan-1").is_some());
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
        let n = compose(&approval("a\n\tb   c"), "gen\neral", "Ada\r\nL", false, "chan-1").unwrap();
        assert_eq!(n.title, "gen eral · Ada L");
        assert_eq!(n.body, "Needs approval: a b c");
        let long = "x".repeat(500);
        let n = compose(&approval(&long), &long, &long, false, "chan-1").unwrap();
        let name = format!("{}…", "x".repeat(59));
        assert_eq!(name.chars().count(), 60);
        assert_eq!(n.title, format!("{name} · {name}"));
        assert_eq!(n.body, format!("Needs approval: {name}"));
    }

    #[test]
    fn the_body_holds_only_the_tool_name_never_the_input() {
        let update = needed_update("Bash");
        let trigger = trigger_of(&update, false).unwrap();
        let n = compose(&trigger, "general", "Ada", false, "chan-1").unwrap();
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
    fn clear_drops_every_focused_window() {
        let mut focus = FocusTracker::new();
        focus.set_focus("main", true);
        focus.set_focus("project-a", true);
        focus.clear();
        assert!(!focus.app_focused());
    }

    #[test]
    fn compose_carries_the_channel_id() {
        let approval = compose(&approval("Bash"), "general", "Ada", false, "chan-7").unwrap();
        assert_eq!(approval.channel_id, "chan-7");
        let finished = compose(&Trigger::Finished(TurnOutcome::Completed), "general", "Ada", false, "chan-8").unwrap();
        assert_eq!(finished.channel_id, "chan-8");
    }

    fn extra(value: serde_json::Value) -> std::collections::HashMap<String, serde_json::Value> {
        std::collections::HashMap::from([("channelId".to_string(), value)])
    }

    #[test]
    fn channel_from_click_returns_the_id_for_a_tap() {
        assert_eq!(channel_from_click("tap", &extra(serde_json::json!("chan-1"))), Some("chan-1".to_string()));
    }

    #[test]
    fn channel_from_click_ignores_another_action() {
        assert_eq!(channel_from_click("approve", &extra(serde_json::json!("chan-1"))), None);
        assert_eq!(channel_from_click("", &extra(serde_json::json!("chan-1"))), None);
    }

    #[test]
    fn channel_from_click_ignores_a_missing_or_non_string_id() {
        assert_eq!(channel_from_click("tap", &std::collections::HashMap::new()), None);
        assert_eq!(channel_from_click("tap", &extra(serde_json::json!(7))), None);
        assert_eq!(channel_from_click("tap", &extra(serde_json::Value::Null)), None);
        assert_eq!(channel_from_click("tap", &extra(serde_json::json!(""))), None);
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
