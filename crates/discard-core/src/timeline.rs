//! Folds the normalized event stream into what the message pane shows.
//! All UI state logic lives here so the UI layer stays thin.

use crate::approval::{ApprovalState, Decision};
use crate::event::{AgentStatus, ApprovalId, EventKind};
use crate::model::{ApprovalOutcome, Block, ChannelId, DecidedBy, Message, MessageId, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalView {
    Pending,
    Approved,
    Denied,
    Stale,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalPrompt {
    pub rule_words: Option<Vec<String>>,
    pub rule_host: Option<String>,
    pub kind: crate::approval::ApprovalKind,
    pub approval_id: ApprovalId,
    pub tool: String,
    pub summary: String,
    pub view: ApprovalView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    blocks: Vec<Block>,
    prompts: Vec<ApprovalPrompt>,
    status: AgentStatus,
    /// True while the last block is streaming text that the next delta should extend.
    text_open: bool,
    /// True when shell calls in this response run under sandbox autorun.
    autorun: bool,
    /// Calls that raised an approval card. Autorun does not apply to them.
    approved_calls: Vec<String>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}

/// One-line description of a tool input for an approval prompt.
pub fn summarize_input(_tool: &str, input: &serde_json::Value) -> String {
    const MAX_CHARS: usize = 120;
    let preferred = ["host", "command", "file_path", "path", "url", "pattern"]
        .iter()
        .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str));
    let full = preferred.map(str::to_string).unwrap_or_else(|| input.to_string());
    if full.chars().count() <= MAX_CHARS {
        return full;
    }
    let mut cut: String = full.chars().take(MAX_CHARS).collect();
    cut.push('…');
    cut
}

impl Timeline {
    pub fn new() -> Self {
        Self { blocks: Vec::new(), prompts: Vec::new(), status: AgentStatus::Idle, text_open: false, autorun: false, approved_calls: Vec::new() }
    }

    /// Mark shell calls without an approval card as sandboxed. Call after the session starts.
    pub fn set_autorun(&mut self, on: bool) {
        self.autorun = on;
    }

    pub fn apply(&mut self, kind: &EventKind) {
        match kind {
            EventKind::TextDelta { text } => {
                if self.text_open
                    && let Some(Block::Markdown { text: existing }) = self.blocks.last_mut()
                {
                    existing.push_str(text);
                } else {
                    self.blocks.push(Block::Markdown { text: text.clone() });
                    self.text_open = true;
                }
            }
            EventKind::ToolCallStart { call_id, name, input } => {
                self.text_open = false;
                self.blocks.push(Block::ToolCall {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    output: None,
                    is_error: None,
                    sandboxed: None,
                });
            }
            EventKind::ToolCallResult { call_id, output, is_error } => {
                for block in &mut self.blocks {
                    if let Block::ToolCall { call_id: id, name, output: slot, is_error: flag, sandboxed, .. } = block
                        && id == call_id
                    {
                        *slot = Some(output.clone());
                        *flag = Some(*is_error);
                        if self.autorun && matches!(name.as_str(), "Bash" | "command") && !self.approved_calls.contains(call_id) {
                            *sandboxed = Some(true);
                        }
                    }
                }
            }
            EventKind::ApprovalRequest { approval_id, call_id, tool, input, kind, .. } => {
                self.text_open = false;
                self.approved_calls.push(call_id.clone());
                if *kind == crate::approval::ApprovalKind::NetworkDomain {
                    // A network request may carry its own call id. It belongs to an open shell call.
                    for block in &self.blocks {
                        if let Block::ToolCall { call_id: id, name, output: None, .. } = block
                            && matches!(name.as_str(), "Bash" | "command")
                        {
                            self.approved_calls.push(id.clone());
                        }
                    }
                }
                let summary = summarize_input(tool, input);
                self.blocks.push(Block::Approval {
                    kind: *kind,
                    approval_id: approval_id.to_string(),
                    tool: tool.clone(),
                    summary: summary.clone(),
                    call_id: (!call_id.is_empty()).then(|| call_id.clone()),
                    state: None,
                    decided_by: None,
                });
                self.prompts.push(ApprovalPrompt {
                    rule_words: if *kind == crate::approval::ApprovalKind::ProviderTool && matches!(tool.as_str(), "Bash" | "command") { input.get("command").and_then(|v| v.as_str()).and_then(crate::always_allow::command_words) } else { None },
                    rule_host: if *kind == crate::approval::ApprovalKind::NetworkDomain { input.get("host").and_then(|v| v.as_str()).and_then(crate::always_allow::normalize_host) } else { None },
                    kind: *kind,
                    approval_id: approval_id.clone(),
                    tool: tool.clone(),
                    summary,
                    view: ApprovalView::Pending,
                });
            }
            EventKind::StatusChange { status } => self.status = *status,
            EventKind::Usage { .. } => {}
            EventKind::Error { message } => {
                self.text_open = false;
                self.blocks.push(Block::Markdown { text: format!("**Error:** {message}") });
            }
        }
    }

    /// Reflect a decision or a state change that did not come from the provider.
    /// The saved block records the outcome. `decided_by` applies to decisions only.
    pub fn mark_approval(&mut self, id: &ApprovalId, state: ApprovalState, decided_by: Option<DecidedBy>) {
        let view = match state {
            ApprovalState::Pending => ApprovalView::Pending,
            ApprovalState::Stale => ApprovalView::Stale,
            ApprovalState::Decided(Decision::Deny) => ApprovalView::Denied,
            ApprovalState::Decided(Decision::Approve | Decision::AlwaysAllow) => ApprovalView::Approved,
        };
        if let Some(prompt) = self.prompts.iter_mut().find(|p| &p.approval_id == id) {
            prompt.view = view;
        }
        let outcome = match view {
            ApprovalView::Pending => None,
            ApprovalView::Approved => Some(ApprovalOutcome::Approved),
            ApprovalView::Denied => Some(ApprovalOutcome::Denied),
            ApprovalView::Stale => Some(ApprovalOutcome::Stale),
        };
        let wanted = id.to_string();
        for block in &mut self.blocks {
            if let Block::Approval { approval_id, state, decided_by: by, .. } = block
                && *approval_id == wanted
            {
                *state = outcome;
                *by = if outcome == Some(ApprovalOutcome::Stale) { None } else { decided_by };
            }
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn status(&self) -> AgentStatus {
        self.status
    }

    pub fn prompts(&self) -> &[ApprovalPrompt] {
        &self.prompts
    }

    /// Count for the inbox badge.
    pub fn pending_approvals(&self) -> usize {
        self.prompts.iter().filter(|p| p.view == ApprovalView::Pending).count()
    }

    pub fn into_message(self, channel_id: ChannelId) -> Message {
        Message { id: MessageId::new(), channel_id, role: Role::Agent, blocks: self.blocks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(t: &str) -> EventKind {
        EventKind::TextDelta { text: t.into() }
    }

    fn approval(id: &ApprovalId) -> EventKind { approval_for("c1", id) }

    fn approval_for(call_id: &str, id: &ApprovalId) -> EventKind {
        EventKind::ApprovalRequest { kind: Default::default(), paths: vec![],
            approval_id: id.clone(),
            call_id: call_id.into(),
            tool: "Bash".into(),
            input: json!({"command": "touch x"}),
        }
    }

    #[test]
    fn saved_approval_blocks_preserve_kind_and_read_old_records() {
        let mut timeline = Timeline::new(); let mut event = approval(&ApprovalId::new());
        if let EventKind::ApprovalRequest { kind, .. } = &mut event { *kind = crate::approval::ApprovalKind::SandboxEscape; }
        timeline.apply(&event);
        assert_eq!(serde_json::to_value(&timeline.blocks()[0]).unwrap()["kind"], "sandbox_escape");
        let old: Block = serde_json::from_value(json!({"type":"approval","approval_id":"old","tool":"Bash","summary":"run"})).unwrap();
        assert_eq!(serde_json::to_value(old).unwrap()["kind"], "provider_tool");
    }
    #[test]
    fn consecutive_text_deltas_merge_into_one_markdown_block() {
        let mut t = Timeline::new();
        for s in ["Hel", "lo ", "there"] {
            t.apply(&text(s));
        }
        assert_eq!(t.blocks(), &[Block::Markdown { text: "Hello there".into() }]);
    }

    #[test]
    fn text_after_a_tool_call_starts_a_new_block() {
        let mut t = Timeline::new();
        t.apply(&text("a"));
        t.apply(&EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({}) });
        t.apply(&text("b"));
        assert_eq!(t.blocks().len(), 3);
        assert_eq!(t.blocks()[2], Block::Markdown { text: "b".into() });
    }

    #[test]
    fn tool_results_attach_to_the_matching_call() {
        let mut t = Timeline::new();
        t.apply(&EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({"command":"ls"}) });
        t.apply(&EventKind::ToolCallStart { call_id: "c2".into(), name: "Read".into(), input: json!({}) });
        t.apply(&EventKind::ToolCallResult { call_id: "c1".into(), output: "files".into(), is_error: false });
        let Block::ToolCall { output, .. } = &t.blocks()[0] else { panic!() };
        assert_eq!(output.as_deref(), Some("files"));
        let Block::ToolCall { output, .. } = &t.blocks()[1] else { panic!() };
        assert_eq!(*output, None);
    }

    #[test]
    fn a_result_for_an_unknown_call_is_ignored() {
        let mut t = Timeline::new();
        t.apply(&EventKind::ToolCallResult { call_id: "nope".into(), output: "x".into(), is_error: false });
        assert!(t.blocks().is_empty());
    }

    #[test]
    fn tool_result_error_flag_reaches_the_block() {
        for is_error in [true, false] {
            let mut t = Timeline::new();
            t.apply(&EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({}) });
            t.apply(&EventKind::ToolCallResult { call_id: "c1".into(), output: "boom".into(), is_error });
            assert!(matches!(&t.blocks()[0], Block::ToolCall { is_error: Some(flag), output: Some(output), .. } if *flag == is_error && output == "boom"));
            assert_eq!(serde_json::to_value(&t.blocks()[0]).unwrap()["is_error"], is_error);
        }
    }

    #[test]
    fn a_started_tool_call_has_no_error_flag() {
        let mut t = Timeline::new();
        t.apply(&EventKind::ToolCallStart { call_id: "c1".into(), name: "Bash".into(), input: json!({}) });
        assert!(matches!(&t.blocks()[0], Block::ToolCall { is_error: None, .. }));
        assert!(serde_json::to_value(&t.blocks()[0]).unwrap().get("is_error").is_none());
    }

    #[test]
    fn a_stored_tool_call_without_the_flag_deserializes() {
        let block: Block = serde_json::from_str(
            r#"{"type":"tool_call","call_id":"c","name":"n","input":null,"output":"x"}"#,
        ).unwrap();
        assert!(matches!(block, Block::ToolCall { is_error: None, .. }));
    }

    fn started(call_id: &str, name: &str) -> EventKind {
        EventKind::ToolCallStart { call_id: call_id.into(), name: name.into(), input: json!({"command": "ls"}) }
    }

    fn finished(call_id: &str) -> EventKind {
        EventKind::ToolCallResult { call_id: call_id.into(), output: "ok".into(), is_error: false }
    }

    #[test]
    fn autorun_marks_shell_calls_without_an_approval() {
        let mut t = Timeline::new();
        t.set_autorun(true);
        t.apply(&started("c1", "Bash"));
        t.apply(&finished("c1"));
        t.apply(&started("c2", "Bash"));
        t.apply(&approval_for("c2", &ApprovalId::new()));
        t.apply(&finished("c2"));
        t.apply(&started("c3", "command"));
        t.apply(&finished("c3"));
        assert!(matches!(&t.blocks()[0], Block::ToolCall { sandboxed: Some(true), .. }));
        assert!(matches!(&t.blocks()[1], Block::ToolCall { sandboxed: None, .. }));
        assert!(matches!(&t.blocks()[3], Block::ToolCall { sandboxed: Some(true), .. }));
        assert_eq!(serde_json::to_value(&t.blocks()[0]).unwrap()["sandboxed"], true);
        assert!(serde_json::to_value(&t.blocks()[1]).unwrap().get("sandboxed").is_none());
    }

    #[test]
    fn a_network_approval_unmarks_the_open_shell_call() {
        // Claude's SandboxNetworkAccess request carries its own tool_use_id, not the shell call's.
        let mut t = Timeline::new();
        t.set_autorun(true);
        t.apply(&started("c1", "Bash"));
        t.apply(&finished("c1"));
        t.apply(&started("c2", "Bash"));
        t.apply(&EventKind::ApprovalRequest {
            kind: crate::approval::ApprovalKind::NetworkDomain,
            paths: vec![],
            approval_id: ApprovalId::new(),
            call_id: "unrelated-network-id".into(),
            tool: "SandboxNetworkAccess".into(),
            input: json!({"host": "example.com"}),
        });
        t.apply(&finished("c2"));
        t.apply(&started("c3", "Bash"));
        t.apply(&finished("c3"));
        assert!(matches!(&t.blocks()[0], Block::ToolCall { sandboxed: Some(true), .. }), "a finished call keeps its mark");
        assert!(matches!(&t.blocks()[1], Block::ToolCall { sandboxed: None, .. }), "the open call asked for network access");
        assert!(matches!(&t.blocks()[3], Block::ToolCall { sandboxed: Some(true), .. }), "a later call is marked again");
    }

    #[test]
    fn calls_are_not_marked_when_autorun_is_off() {
        let mut t = Timeline::new();
        t.apply(&started("c1", "Bash"));
        t.apply(&finished("c1"));
        assert!(matches!(&t.blocks()[0], Block::ToolCall { sandboxed: None, .. }));
    }

    #[test]
    fn edits_are_not_marked_sandboxed() {
        let mut t = Timeline::new();
        t.set_autorun(true);
        t.apply(&started("c1", "Edit"));
        t.apply(&finished("c1"));
        assert!(matches!(&t.blocks()[0], Block::ToolCall { sandboxed: None, .. }));
    }

    #[test]
    fn approval_requests_become_pending_prompts_and_blocks() {
        let mut t = Timeline::new();
        let id = ApprovalId::new();
        t.apply(&approval(&id));
        assert_eq!(t.pending_approvals(), 1);
        assert_eq!(t.prompts()[0].summary, "touch x");
        assert!(matches!(&t.blocks()[0], Block::Approval { tool, summary, .. } if tool == "Bash" && summary == "touch x"));
    }

    #[test]
    fn decisions_and_stale_states_update_the_prompt() {
        let mut t = Timeline::new();
        let (a, b, c) = (ApprovalId::new(), ApprovalId::new(), ApprovalId::new());
        for id in [&a, &b, &c] {
            t.apply(&approval(id));
        }
        t.mark_approval(&a, ApprovalState::Decided(Decision::Approve), Some(DecidedBy::User));
        t.mark_approval(&b, ApprovalState::Decided(Decision::Deny), Some(DecidedBy::User));
        t.mark_approval(&c, ApprovalState::Stale, None);
        let views: Vec<_> = t.prompts().iter().map(|p| p.view).collect();
        assert_eq!(views, vec![ApprovalView::Approved, ApprovalView::Denied, ApprovalView::Stale]);
        assert_eq!(t.pending_approvals(), 0);
    }

    fn approval_block(t: &Timeline, index: usize) -> serde_json::Value {
        serde_json::to_value(&t.blocks()[index]).unwrap()
    }

    #[test]
    fn the_approval_block_carries_the_call_id_and_no_outcome_while_pending() {
        let mut t = Timeline::new();
        t.apply(&approval_for("call-7", &ApprovalId::new()));
        assert!(matches!(&t.blocks()[0], Block::Approval { call_id: Some(id), state: None, decided_by: None, .. } if id == "call-7"));
        let saved = approval_block(&t, 0);
        assert_eq!(saved["call_id"], "call-7");
        assert!(saved.get("state").is_none());
        assert!(saved.get("decided_by").is_none());
    }

    #[test]
    fn a_user_decision_lands_in_the_saved_block() {
        let mut t = Timeline::new();
        let (a, b) = (ApprovalId::new(), ApprovalId::new());
        t.apply(&approval_for("c1", &a));
        t.apply(&approval_for("c2", &b));
        t.mark_approval(&a, ApprovalState::Decided(Decision::Approve), Some(DecidedBy::User));
        t.mark_approval(&b, ApprovalState::Decided(Decision::Deny), Some(DecidedBy::User));
        let (first, second) = (approval_block(&t, 0), approval_block(&t, 1));
        assert_eq!((first["state"].as_str(), first["decided_by"].as_str()), (Some("approved"), Some("user")));
        assert_eq!((second["state"].as_str(), second["decided_by"].as_str()), (Some("denied"), Some("user")));
    }

    #[test]
    fn a_policy_decision_lands_in_the_saved_block() {
        let mut t = Timeline::new();
        let id = ApprovalId::new();
        t.apply(&approval(&id));
        t.mark_approval(&id, ApprovalState::Decided(Decision::Approve), Some(DecidedBy::Policy));
        let saved = approval_block(&t, 0);
        assert_eq!((saved["state"].as_str(), saved["decided_by"].as_str()), (Some("approved"), Some("policy")));
    }

    #[test]
    fn a_stale_approval_lands_in_the_saved_block_without_a_decider() {
        let mut t = Timeline::new();
        let id = ApprovalId::new();
        t.apply(&approval(&id));
        t.mark_approval(&id, ApprovalState::Stale, None);
        let saved = approval_block(&t, 0);
        assert_eq!(saved["state"], "stale");
        assert!(saved.get("decided_by").is_none());
    }

    #[test]
    fn an_old_approval_record_without_the_new_fields_still_loads() {
        let old: Block = serde_json::from_value(json!({"type":"approval","kind":"provider_tool","approval_id":"old","tool":"Bash","summary":"run"})).unwrap();
        assert!(matches!(&old, Block::Approval { call_id: None, state: None, decided_by: None, .. }));
        let saved = serde_json::to_value(&old).unwrap();
        assert!(saved.get("call_id").is_none() && saved.get("state").is_none() && saved.get("decided_by").is_none());
    }

    #[test]
    fn always_allow_shows_as_approved() {
        let mut t = Timeline::new();
        let id = ApprovalId::new();
        t.apply(&approval(&id));
        t.mark_approval(&id, ApprovalState::Decided(Decision::AlwaysAllow), Some(DecidedBy::User));
        assert_eq!(t.prompts()[0].view, ApprovalView::Approved);
    }

    #[test]
    fn marking_an_unknown_approval_changes_nothing() {
        let mut t = Timeline::new();
        t.apply(&approval(&ApprovalId::new()));
        t.mark_approval(&ApprovalId::new(), ApprovalState::Stale, None);
        assert_eq!(t.pending_approvals(), 1);
    }

    #[test]
    fn status_changes_are_tracked_and_errors_become_visible_blocks() {
        let mut t = Timeline::new();
        t.apply(&EventKind::StatusChange { status: AgentStatus::Working });
        assert_eq!(t.status(), AgentStatus::Working);
        t.apply(&EventKind::Error { message: "boom".into() });
        assert_eq!(t.blocks(), &[Block::Markdown { text: "**Error:** boom".into() }]);
    }

    #[test]
    fn usage_events_add_no_blocks() {
        let mut t = Timeline::new();
        t.apply(&EventKind::Usage { input_tokens: 1, output_tokens: 2 });
        assert!(t.blocks().is_empty());
    }

    #[test]
    fn summaries_prefer_the_command_and_truncate_long_input() {
        assert_eq!(summarize_input("Bash", &json!({"command": "ls -la"})), "ls -la");
        assert_eq!(summarize_input("Read", &json!({"file_path": "/a/b"})), "/a/b");
        let long = summarize_input("Other", &json!({"k": "x".repeat(500)}));
        assert!(long.chars().count() <= 121, "{}", long.chars().count());
        assert!(long.ends_with('…'));
    }

    #[test]
    fn into_message_keeps_the_blocks_as_an_agent_message() {
        let mut t = Timeline::new();
        t.apply(&text("hi"));
        let m = t.into_message(ChannelId::new());
        assert_eq!(m.role, Role::Agent);
        assert_eq!(m.blocks, vec![Block::Markdown { text: "hi".into() }]);
    }
}
