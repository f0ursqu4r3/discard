//! Normalized provider event stream.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);
        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7().to_string())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(EventId);
id_type!(SessionId);
id_type!(TurnId);
id_type!(ApprovalId);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    TextDelta { text: String },
    ToolCallStart { call_id: String, name: String, input: Value },
    ToolCallResult { call_id: String, output: String, is_error: bool },
    ApprovalRequest { approval_id: ApprovalId, call_id: String, tool: String, input: Value },
    StatusChange { status: AgentStatus },
    Usage { input_tokens: u64, output_tokens: u64 },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    AwaitingApproval,
    Offline,
}

/// A provider event before the host assigns identity and order.
#[derive(Debug, Clone, PartialEq)]
pub struct RawEvent {
    pub kind: EventKind,
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub session_id: SessionId,
    pub seq: u64,
    pub kind: EventKind,
    pub raw: Option<Value>,
}

/// Assigns a durable ID and a gap-free, per-session sequence number.
#[derive(Debug, Default)]
pub struct Sequencer {
    next: HashMap<SessionId, u64>,
}

impl Sequencer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resume numbering after `last_seq` for a session loaded from storage.
    pub fn resume(&mut self, session: &SessionId, last_seq: u64) {
        self.next.insert(session.clone(), last_seq);
    }

    pub fn stamp(&mut self, session: &SessionId, event: RawEvent) -> Event {
        let counter = self.next.entry(session.clone()).or_insert(0);
        *counter += 1;
        Event {
            id: EventId::new(),
            session_id: session.clone(),
            seq: *counter,
            kind: event.kind,
            raw: event.raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> RawEvent {
        RawEvent { kind: EventKind::TextDelta { text: s.into() }, raw: None }
    }

    #[test]
    fn sequence_numbers_start_at_one_and_have_no_gaps() {
        let mut seq = Sequencer::new();
        let s = SessionId::new();
        let seqs: Vec<u64> = (0..3).map(|_| seq.stamp(&s, text("x")).seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[test]
    fn sequences_are_independent_per_session() {
        let mut seq = Sequencer::new();
        let (a, b) = (SessionId::new(), SessionId::new());
        seq.stamp(&a, text("x"));
        seq.stamp(&a, text("x"));
        assert_eq!(seq.stamp(&b, text("x")).seq, 1);
        assert_eq!(seq.stamp(&a, text("x")).seq, 3);
    }

    #[test]
    fn event_ids_are_unique() {
        let mut seq = Sequencer::new();
        let s = SessionId::new();
        let a = seq.stamp(&s, text("x"));
        let b = seq.stamp(&s, text("x"));
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn resume_continues_after_last_stored_sequence() {
        let mut seq = Sequencer::new();
        let s = SessionId::new();
        seq.resume(&s, 41);
        assert_eq!(seq.stamp(&s, text("x")).seq, 42);
    }

    #[test]
    fn stamp_keeps_the_raw_provider_payload() {
        let mut seq = Sequencer::new();
        let s = SessionId::new();
        let raw = serde_json::json!({"vendor": "field"});
        let ev = seq.stamp(&s, RawEvent { kind: EventKind::TextDelta { text: "x".into() }, raw: Some(raw.clone()) });
        assert_eq!(ev.raw, Some(raw));
    }

    #[test]
    fn events_round_trip_through_json() {
        let mut seq = Sequencer::new();
        let s = SessionId::new();
        let ev = seq.stamp(&s, text("hello"));
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);
    }
}
