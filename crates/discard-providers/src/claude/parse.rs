//! Maps Claude `stream-json` lines to normalized events. Pure: no I/O.

use discard_core::approval::ApprovalKind;
use discard_core::event::{AgentStatus, ApprovalId, EventKind, RawEvent};
use serde_json::Value;
use std::collections::HashSet;

/// Details needed later to answer a permission request.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequest {
    pub approval_id: ApprovalId,
    /// Claude's `request_id`. The `control_response` must echo it.
    pub request_id: String,
    pub input: Value,
    pub suggestions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Claude's own session ID. Stored as a resume handle.
    SessionId(String),
    Event(RawEvent),
    /// The CLI compacted the conversation (`compact_boundary`).
    Compacted,
    /// Always followed by the matching `ApprovalRequest` event.
    Request(PendingRequest),
}

#[derive(Debug, Default)]
pub struct StreamParser {
    seen_tools: HashSet<String>,
}

impl StreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse_line(&mut self, line: &str) -> Vec<Item> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            tracing::debug!("skipping a line that is not JSON");
            return Vec::new();
        };
        match v.get("type").and_then(Value::as_str) {
            Some("system") => self.system(&v),
            Some("stream_event") => stream_event(&v),
            Some("assistant") => self.assistant(&v),
            Some("user") => user(&v),
            Some("control_request") => control_request(&v),
            Some("result") => result(&v),
            _ => Vec::new(),
        }
    }

    fn system(&mut self, v: &Value) -> Vec<Item> {
        match v.get("subtype").and_then(Value::as_str) {
            Some("init") => {}
            Some("compact_boundary") => return vec![Item::Compacted],
            _ => return Vec::new(),
        }
        let mut out = Vec::new();
        if let Some(id) = v.get("session_id").and_then(Value::as_str) {
            out.push(Item::SessionId(id.to_string()));
        }
        out.push(event(EventKind::StatusChange { status: AgentStatus::Working }, v));
        out
    }

    fn assistant(&mut self, v: &Value) -> Vec<Item> {
        let blocks = v.pointer("/message/content").and_then(Value::as_array);
        let mut out = Vec::new();
        for block in blocks.into_iter().flatten() {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let (Some(id), Some(name)) = (
                block.get("id").and_then(Value::as_str),
                block.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !self.seen_tools.insert(id.to_string()) {
                continue;
            }
            let input = block.get("input").cloned().unwrap_or(Value::Null);
            out.push(event(EventKind::ToolCallStart { call_id: id.into(), name: name.into(), input }, v));
        }
        out
    }
}

fn event(kind: EventKind, raw: &Value) -> Item {
    Item::Event(RawEvent { kind, raw: Some(raw.clone()) })
}

fn stream_event(v: &Value) -> Vec<Item> {
    let Some(ev) = v.get("event") else { return Vec::new() };
    if ev.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return Vec::new();
    }
    let delta = &ev["delta"];
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return Vec::new();
    }
    match delta.get("text").and_then(Value::as_str) {
        Some(text) => vec![event(EventKind::TextDelta { text: text.to_string() }, v)],
        None => Vec::new(),
    }
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<String>(),
        _ => String::new(),
    }
}

fn user(v: &Value) -> Vec<Item> {
    let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|b| {
            let call_id = b.get("tool_use_id").and_then(Value::as_str)?.to_string();
            Some(event(
                EventKind::ToolCallResult {
                    call_id,
                    output: tool_result_text(b.get("content").unwrap_or(&Value::Null)),
                    is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                },
                v,
            ))
        })
        .collect()
}

fn control_request(v: &Value) -> Vec<Item> {
    let req = &v["request"];
    if req.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        return Vec::new();
    }
    let (Some(request_id), Some(tool)) = (
        v.get("request_id").and_then(Value::as_str),
        req.get("tool_name").and_then(Value::as_str),
    ) else {
        return Vec::new();
    };
    let input = req.get("input").cloned().unwrap_or(Value::Null);
    let paths = if matches!(tool, "Edit" | "Write" | "NotebookEdit") {
        input.get(if tool == "NotebookEdit" { "notebook_path" } else { "file_path" }).and_then(Value::as_str).map(|p| vec![p.to_owned()]).unwrap_or_default()
    } else { vec![] };
    let kind = if tool == "SandboxNetworkAccess" { ApprovalKind::NetworkDomain }
        else if input.get("dangerouslyDisableSandbox").and_then(Value::as_bool) == Some(true) { ApprovalKind::SandboxEscape }
        else { ApprovalKind::ProviderTool };
    let approval_id = ApprovalId::new();
    let suggestions = req
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    vec![
        Item::Request(PendingRequest {
            approval_id: approval_id.clone(),
            request_id: request_id.to_string(),
            input: input.clone(),
            suggestions,
        }),
        event(
            EventKind::ApprovalRequest { kind, paths,
                approval_id,
                call_id: req.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string(),
                tool: tool.to_string(),
                input,
            },
            v,
        ),
    ]
}

fn result(v: &Value) -> Vec<Item> {
    let tokens = |key: &str| v.pointer(&format!("/usage/{key}")).and_then(Value::as_u64).unwrap_or(0);
    let mut out = vec![event(
        EventKind::Usage { input_tokens: tokens("input_tokens"), output_tokens: tokens("output_tokens") },
        v,
    )];
    if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        let message = v.get("result").and_then(Value::as_str).unwrap_or("provider reported an error").to_string();
        out.push(event(EventKind::Error { message }, v));
    }
    out.push(event(EventKind::StatusChange { status: AgentStatus::Idle }, v));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::event::{AgentStatus, EventKind};
    use serde_json::json;

    fn events(items: &[Item]) -> Vec<&EventKind> {
        items
            .iter()
            .filter_map(|i| match i {
                Item::Event(e) => Some(&e.kind),
                _ => None,
            })
            .collect()
    }

    fn parse(p: &mut StreamParser, v: Value) -> Vec<Item> {
        p.parse_line(&v.to_string())
    }

    #[test]
    fn workspace_recording_identifies_edit_paths_and_network_domains() {
        let mut parser = StreamParser::new(); let mut edits = 0; let mut network = 0;
        for line in include_str!("../../fixtures/claude-workspace-turn.jsonl").lines() {
            for item in parser.parse_line(line) {
                if let Item::Event(e) = item && let EventKind::ApprovalRequest { tool, kind, paths, .. } = e.kind {
                    if tool == "Edit" { assert_eq!(paths, ["/synthetic/repo/edit.txt"]); edits += 1; }
                    if tool == "SandboxNetworkAccess" { assert_eq!(kind, discard_core::approval::ApprovalKind::NetworkDomain); network += 1; }
                }
            }
        }
        assert!(edits >= 2 && network >= 2);
    }

    #[test]
    fn init_yields_session_id_and_working_status() {
        let mut p = StreamParser::new();
        let items = parse(&mut p, json!({"type":"system","subtype":"init","session_id":"abc"}));
        assert_eq!(items[0], Item::SessionId("abc".into()));
        assert_eq!(events(&items), vec![&EventKind::StatusChange { status: AgentStatus::Working }]);
    }

    #[test]
    fn text_deltas_become_text_events_with_the_raw_line_attached() {
        let mut p = StreamParser::new();
        let line = json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"hi"}}});
        let items = parse(&mut p, line.clone());
        assert_eq!(events(&items), vec![&EventKind::TextDelta { text: "hi".into() }]);
        let Item::Event(e) = &items[0] else { panic!() };
        assert_eq!(e.raw.as_ref(), Some(&line));
    }

    #[test]
    fn thinking_and_signature_deltas_are_not_forwarded() {
        let mut p = StreamParser::new();
        for d in [
            json!({"type":"thinking_delta","thinking":"secret"}),
            json!({"type":"signature_delta","signature":"x"}),
            json!({"type":"input_json_delta","partial_json":"{"}),
        ] {
            let items = parse(&mut p, json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":d}}));
            assert!(items.is_empty());
        }
    }

    #[test]
    fn tool_use_in_an_assistant_message_starts_a_tool_call_once() {
        let mut p = StreamParser::new();
        let msg = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}});
        let first = parse(&mut p, msg.clone());
        assert_eq!(
            events(&first),
            vec![&EventKind::ToolCallStart { call_id: "t1".into(), name: "Bash".into(), input: json!({"command":"ls"}) }]
        );
        assert!(parse(&mut p, msg).is_empty(), "a repeated assistant message must not repeat the tool call");
    }

    #[test]
    fn assistant_text_and_thinking_blocks_are_ignored() {
        let mut p = StreamParser::new();
        let items = parse(&mut p, json!({"type":"assistant","message":{"content":[{"type":"text","text":"x"},{"type":"thinking","thinking":"y"}]}}));
        assert!(items.is_empty(), "text already arrived as deltas");
    }

    #[test]
    fn can_use_tool_requests_become_pending_approvals() {
        let mut p = StreamParser::new();
        let items = parse(
            &mut p,
            json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch x"},"tool_use_id":"t1","permission_suggestions":[{"type":"addRules"}]}}),
        );
        let Item::Request(req) = &items[0] else { panic!("expected a pending request first") };
        assert_eq!(req.request_id, "r1");
        assert_eq!(req.input, json!({"command":"touch x"}));
        assert_eq!(req.suggestions, vec![json!({"type":"addRules"})]);
        let EventKind::ApprovalRequest { approval_id, call_id, tool, .. } = events(&items)[0] else { panic!() };
        assert_eq!((call_id.as_str(), tool.as_str()), ("t1", "Bash"));
        assert_eq!(approval_id, &req.approval_id);
    }

    #[test]
    fn other_control_requests_are_ignored() {
        let mut p = StreamParser::new();
        assert!(parse(&mut p, json!({"type":"control_request","request_id":"r","request":{"subtype":"hook_callback"}})).is_empty());
    }

    #[test]
    fn tool_results_accept_string_or_block_content() {
        let mut p = StreamParser::new();
        let s = parse(&mut p, json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"out","is_error":false}]}}));
        assert_eq!(events(&s), vec![&EventKind::ToolCallResult { call_id: "t1".into(), output: "out".into(), is_error: false }]);
        let b = parse(&mut p, json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"is_error":true}]}}));
        assert_eq!(events(&b), vec![&EventKind::ToolCallResult { call_id: "t2".into(), output: "ab".into(), is_error: true }]);
    }

    #[test]
    fn plain_user_messages_are_ignored() {
        let mut p = StreamParser::new();
        assert!(parse(&mut p, json!({"type":"user","message":{"role":"user","content":"hello"}})).is_empty());
    }

    #[test]
    fn success_result_reports_usage_then_idle() {
        let mut p = StreamParser::new();
        let items = parse(&mut p, json!({"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":4,"output_tokens":175}}));
        assert_eq!(
            events(&items),
            vec![&EventKind::Usage { input_tokens: 4, output_tokens: 175 }, &EventKind::StatusChange { status: AgentStatus::Idle }]
        );
    }

    #[test]
    fn error_result_reports_an_error_before_idle() {
        let mut p = StreamParser::new();
        let items = parse(&mut p, json!({"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom","usage":{"input_tokens":1,"output_tokens":2}}));
        let kinds = events(&items);
        assert_eq!(kinds[1], &EventKind::Error { message: "boom".into() });
        assert_eq!(kinds.last().unwrap(), &&EventKind::StatusChange { status: AgentStatus::Idle });
    }

    #[test]
    fn unknown_and_malformed_lines_are_skipped() {
        let mut p = StreamParser::new();
        for l in ["", "   ", "not json", "{\"type\":\"rate_limit_event\"}", "{\"no_type\":1}"] {
            assert!(p.parse_line(l).is_empty(), "{l:?}");
        }
    }

    #[test]
    fn recorded_transcript_maps_to_the_expected_event_order() {
        let text = include_str!("../../fixtures/claude-approval-turn.jsonl");
        let mut p = StreamParser::new();
        let mut names = Vec::new();
        let mut session = None;
        for line in text.lines() {
            for item in p.parse_line(line) {
                match item {
                    Item::SessionId(s) => session = Some(s),
                    Item::Event(e) => {
                        let n = match e.kind {
                            EventKind::StatusChange { status: AgentStatus::Working } => "working",
                            EventKind::StatusChange { status: AgentStatus::Idle } => "idle",
                            EventKind::StatusChange { .. } => "status",
                            EventKind::ToolCallStart { .. } => "tool_start",
                            EventKind::ToolCallResult { .. } => "tool_result",
                            EventKind::ApprovalRequest { .. } => "approval",
                            EventKind::TextDelta { .. } => "text",
                            EventKind::Usage { .. } => "usage",
                            EventKind::Error { .. } => "error",
                        };
                        if names.last() != Some(&n) || n != "text" {
                            names.push(n);
                        }
                    }
                    Item::Request(_) | Item::Compacted => {}
                }
            }
        }
        assert_eq!(session.as_deref(), Some("00000000-0000-4000-8000-000000000001"));
        assert_eq!(names, vec!["working", "tool_start", "approval", "tool_result", "text", "usage", "idle"]);
    }
}
