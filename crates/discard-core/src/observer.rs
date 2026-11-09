//! Lets plugins observe what happens in a channel. The engine calls an `EventObserver`. It never waits for it.

use crate::driver::TurnOutcome;
use crate::event::{AgentStatus, EventKind};
use serde_json::{Value, json};

/// Receives named events with a JSON payload. Implementations must not block.
pub trait EventObserver: Send + Sync {
    fn observe(&self, event: &str, payload: &Value);
}

/// Names of the events a plugin can subscribe to with `discard.on`.
pub const EVENT_NAMES: &[&str] = &["message_sent", "tool_call", "tool_result", "approval_requested", "agent_status", "turn_finished"];

/// Events that describe tool calls. A provider must have tool-call visibility to produce them.
pub fn is_tool_event(name: &str) -> bool {
    matches!(name, "tool_call" | "tool_result" | "approval_requested")
}

pub fn message_sent(channel_id: &str, text: &str) -> (&'static str, Value) {
    ("message_sent", json!({ "channel_id": channel_id, "text": text }))
}

pub fn turn_finished(outcome: TurnOutcome) -> (&'static str, Value) {
    let outcome = match outcome {
        TurnOutcome::Completed => "completed",
        TurnOutcome::Failed => "failed",
        TurnOutcome::Cancelled => "cancelled",
    };
    ("turn_finished", json!({ "outcome": outcome }))
}

/// Map a provider event to a plugin event. Payloads carry names, ids, and flags only. They never carry tool
/// input, tool output, or agent text, because those can hold secrets.
pub fn from_event(kind: &EventKind) -> Option<(&'static str, Value)> {
    match kind {
        EventKind::ToolCallStart { call_id, name, .. } => Some(("tool_call", json!({ "call_id": call_id, "tool": name }))),
        EventKind::ToolCallResult { call_id, is_error, .. } => Some(("tool_result", json!({ "call_id": call_id, "is_error": is_error }))),
        EventKind::ApprovalRequest { approval_id, call_id, tool, .. } => {
            Some(("approval_requested", json!({ "approval_id": approval_id.to_string(), "call_id": call_id, "tool": tool })))
        }
        EventKind::StatusChange { status } => {
            let status = match status {
                AgentStatus::Idle => "idle",
                AgentStatus::Working => "working",
                AgentStatus::AwaitingApproval => "awaiting_approval",
                AgentStatus::Offline => "offline",
            };
            Some(("agent_status", json!({ "status": status })))
        }
        EventKind::TextDelta { .. } | EventKind::Usage { .. } | EventKind::Error { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ApprovalId;

    #[test]
    fn a_tool_call_carries_the_tool_name_and_id_but_not_the_input() {
        let kind = EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({"command": "echo secret-token"}) };
        let (name, payload) = from_event(&kind).unwrap();
        assert_eq!(name, "tool_call");
        assert_eq!(payload, json!({ "call_id": "c1", "tool": "Bash" }));
    }

    #[test]
    fn a_tool_result_carries_the_error_flag_but_not_the_output() {
        let kind = EventKind::ToolCallResult { call_id: "c1".into(), output: "secret-token".into(), is_error: true };
        assert_eq!(from_event(&kind).unwrap(), ("tool_result", json!({ "call_id": "c1", "is_error": true })));
    }

    #[test]
    fn an_approval_request_carries_ids_and_the_tool_but_not_the_input() {
        let id = ApprovalId("a1".into());
        let kind = EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],  approval_id: id, call_id: "c1".into(), tool: "Bash".into(), input: json!({"command": "rm -rf x"}) };
        assert_eq!(from_event(&kind).unwrap(), ("approval_requested", json!({ "approval_id": "a1", "call_id": "c1", "tool": "Bash" })));
    }

    #[test]
    fn every_status_has_a_stable_name() {
        for (status, name) in [
            (AgentStatus::Idle, "idle"),
            (AgentStatus::Working, "working"),
            (AgentStatus::AwaitingApproval, "awaiting_approval"),
            (AgentStatus::Offline, "offline"),
        ] {
            assert_eq!(from_event(&EventKind::StatusChange { status }).unwrap(), ("agent_status", json!({ "status": name })));
        }
    }

    #[test]
    fn text_usage_and_errors_are_not_sent_to_plugins() {
        assert!(from_event(&EventKind::TextDelta { text: "hi".into() }).is_none());
        assert!(from_event(&EventKind::Usage { input_tokens: 1, output_tokens: 2 }).is_none());
        assert!(from_event(&EventKind::Error { message: "token sk-123".into() }).is_none());
    }

    #[test]
    fn every_mapped_event_name_is_listed_in_event_names() {
        let kinds = [
            EventKind::ToolCallStart { call_id: "c".into(), name: "n".into(), input: json!({}) },
            EventKind::ToolCallResult { call_id: "c".into(), output: String::new(), is_error: false },
            EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],  approval_id: ApprovalId("a".into()), call_id: "c".into(), tool: "t".into(), input: json!({}) },
            EventKind::StatusChange { status: AgentStatus::Idle },
        ];
        let mut names: Vec<&str> = kinds.iter().filter_map(from_event).map(|(n, _)| n).collect();
        names.push(message_sent("c", "t").0);
        names.push(turn_finished(TurnOutcome::Completed).0);
        names.sort_unstable();
        let mut listed = EVENT_NAMES.to_vec();
        listed.sort_unstable();
        assert_eq!(names, listed);
    }

    #[test]
    fn tool_events_are_the_ones_that_need_tool_call_visibility() {
        assert!(is_tool_event("tool_call") && is_tool_event("tool_result") && is_tool_event("approval_requested"));
        assert!(!is_tool_event("message_sent") && !is_tool_event("agent_status") && !is_tool_event("turn_finished"));
    }

    #[test]
    fn turn_outcomes_have_stable_names() {
        assert_eq!(turn_finished(TurnOutcome::Failed).1, json!({ "outcome": "failed" }));
        assert_eq!(turn_finished(TurnOutcome::Cancelled).1, json!({ "outcome": "cancelled" }));
    }

    #[test]
    fn a_message_carries_the_channel_and_text() {
        assert_eq!(message_sent("ch", "hello").1, json!({ "channel_id": "ch", "text": "hello" }));
    }
}
