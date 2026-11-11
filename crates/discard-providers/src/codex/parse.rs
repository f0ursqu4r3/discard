//! Maps Codex app-server messages to normalized events. Pure: no I/O.

use discard_core::approval::{ApprovalKind, Decision};
use discard_core::provider::SessionMode;
use discard_core::event::{AgentStatus, ApprovalId, EventKind, RawEvent};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Command,
    FileChange,
    McpElicitation,
}

/// A server request waiting for our answer. The answer must echo `rpc_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequest {
    pub approval_id: ApprovalId,
    pub rpc_id: Value,
    pub kind: RequestKind,
    /// Decisions the server said it accepts. Empty means it did not say.
    pub available: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// The turn started. The ID is only used for logging.
    TurnStarted(String),
    Event(RawEvent),
    /// Always followed by the matching `ApprovalRequest` event.
    Request(PendingRequest),
    /// Answer a server request that Discard does not support, so Codex does not wait forever.
    RejectRequest { rpc_id: Value, message: String },
}

#[derive(Debug, Default)]
pub struct Mapper {
    started: HashSet<String>,
    mode: SessionMode,
    /// The session runs under `on-request`. Codex then asks only to leave the sandbox.
    autorun: bool,
    edit_paths: std::collections::HashMap<String, Vec<String>>,
    input_tokens: u64,
    output_tokens: u64,
}

impl Mapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mode(mode: SessionMode) -> Self { Self { mode, ..Self::default() } }

    /// Mark the session as autorun (`approvalPolicy: "on-request"`).
    pub fn with_autorun(mut self, autorun: bool) -> Self { self.autorun = autorun; self }

    /// Map one server message. Responses to our own requests are not handled here.
    pub fn on_message(&mut self, msg: &Value) -> Vec<Item> {
        let Some(method) = msg.get("method").and_then(Value::as_str) else { return Vec::new() };
        let params = msg.get("params").unwrap_or(&Value::Null);
        match msg.get("id") {
            Some(id) => self.server_request(method, id, params, msg),
            None => self.notification(method, params, msg),
        }
    }

    fn notification(&mut self, method: &str, params: &Value, raw: &Value) -> Vec<Item> {
        match method {
            "turn/started" => {
                let id = params.pointer("/turn/id").and_then(Value::as_str).unwrap_or_default().to_string();
                vec![Item::TurnStarted(id), event(EventKind::StatusChange { status: AgentStatus::Working }, raw)]
            }
            "item/agentMessage/delta" => match params.get("delta").and_then(Value::as_str) {
                Some(text) => vec![event(EventKind::TextDelta { text: text.to_string() }, raw)],
                None => Vec::new(),
            },
            "item/started" => params.get("item").map(|i| self.item_started(i, raw)).unwrap_or_default(),
            "item/completed" => params.get("item").map(|i| self.item_completed(i, raw)).unwrap_or_default(),
            "thread/tokenUsage/updated" => {
                let total = |key: &str| params.pointer(&format!("/tokenUsage/total/{key}")).and_then(Value::as_u64);
                self.input_tokens = total("inputTokens").unwrap_or(self.input_tokens);
                self.output_tokens = total("outputTokens").unwrap_or(self.output_tokens);
                Vec::new()
            }
            "turn/completed" => {
                let mut out = vec![event(EventKind::Usage { input_tokens: self.input_tokens, output_tokens: self.output_tokens }, raw)];
                if params.pointer("/turn/status").and_then(Value::as_str) == Some("failed") {
                    let message = params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex reported that the turn failed")
                        .to_string();
                    out.push(event(EventKind::Error { message }, raw));
                }
                out.push(event(EventKind::StatusChange { status: AgentStatus::Idle }, raw));
                out
            }
            "error" if !params.get("willRetry").and_then(Value::as_bool).unwrap_or(false) => {
                let message = params.pointer("/error/message").and_then(Value::as_str).unwrap_or("Codex reported an error").to_string();
                vec![event(EventKind::Error { message }, raw)]
            }
            _ => Vec::new(),
        }
    }

    fn item_started(&mut self, item: &Value, raw: &Value) -> Vec<Item> {
        let Some((call_id, name, input)) = describe_item(item) else { return Vec::new() };
        if name == "file_change" { self.edit_paths.insert(call_id.clone(), input["paths"].as_array().map(|p| p.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()); }
        if !self.started.insert(call_id.clone()) {
            return Vec::new();
        }
        vec![event(EventKind::ToolCallStart { call_id, name, input }, raw)]
    }

    fn item_completed(&mut self, item: &Value, raw: &Value) -> Vec<Item> {
        let Some((call_id, name, input)) = describe_item(item) else { return Vec::new() };
        let mut out = Vec::new();
        if self.started.insert(call_id.clone()) {
            out.push(event(EventKind::ToolCallStart { call_id: call_id.clone(), name, input }, raw));
        }
        let (output, is_error) = item_result(item);
        out.push(event(EventKind::ToolCallResult { call_id, output, is_error }, raw));
        out
    }

    fn server_request(&mut self, method: &str, id: &Value, params: &Value, raw: &Value) -> Vec<Item> {
        let (kind, tool, mut input) = match method {
            "item/commandExecution/requestApproval" => (
                RequestKind::Command,
                "command".to_string(),
                serde_json::json!({"command": params.get("command"), "cwd": params.get("cwd"), "reason": params.get("reason")}),
            ),
            "item/fileChange/requestApproval" => (
                RequestKind::FileChange,
                "file_change".to_string(),
                serde_json::json!({"reason": params.get("reason"), "grantRoot": params.get("grantRoot")}),
            ),
            "mcpServer/elicitation/request" => {
                let server = params.get("serverName").and_then(Value::as_str).unwrap_or("unknown");
                let tool = params.pointer("/_meta/tool_name").and_then(Value::as_str).unwrap_or("elicitation");
                (RequestKind::McpElicitation, mcp_tool_name(server, tool), serde_json::json!({"message": params.get("message")}))
            }
            other => {
                let message = format!("Codex sent a request that Discard does not support ({other}), so Discard declined it.");
                return vec![Item::RejectRequest { rpc_id: id.clone(), message: message.clone() }, event(EventKind::Error { message }, raw)];
            }
        };
        let available = params
            .get("availableDecisions")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let approval_id = ApprovalId::new();
        let call_id = params.get("itemId").and_then(Value::as_str).unwrap_or_default().to_string();
        // Only complete item notifications carry every edit and rename target.
        let paths = self.edit_paths.get(&call_id).cloned().unwrap_or_default();
        if kind == RequestKind::FileChange { input["paths"] = serde_json::json!(paths); }
        // Under on-request, a file-change request with a reason is a sandbox retry ("command failed; retry
        // without sandbox?"). That leaves the sandbox, so it gets the escape card whatever the path is.
        let retry = self.autorun && kind == RequestKind::FileChange && !params["reason"].is_null();
        let approval_kind = if self.mode == SessionMode::Workspace && (kind == RequestKind::Command || kind == RequestKind::FileChange && (!params["grantRoot"].is_null() || retry)) {
            ApprovalKind::SandboxEscape
        } else { ApprovalKind::ProviderTool };
        vec![
            Item::Request(PendingRequest { approval_id: approval_id.clone(), rpc_id: id.clone(), kind, available }),
            event(EventKind::ApprovalRequest { approval_id, call_id, tool, input, kind: approval_kind, paths }, raw),
        ]
    }
}

fn event(kind: EventKind, raw: &Value) -> Item {
    Item::Event(RawEvent { kind, raw: Some(raw.clone()) })
}

fn mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// (call id, tool name, input) for items that are tool calls.
fn describe_item(item: &Value) -> Option<(String, String, Value)> {
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    match item.get("type").and_then(Value::as_str)? {
        "commandExecution" => Some((id, "command".into(), serde_json::json!({"command": item.get("command"), "cwd": item.get("cwd")}))),
        "fileChange" => {
            let paths = item.get("changes").and_then(Value::as_array).and_then(|changes| {
                let mut paths = Vec::new();
                for change in changes {
                    paths.push(change.get("path")?.as_str()?);
                    let kind = change.get("kind")?;
                    match kind.get("type")?.as_str()? { "add" | "delete" | "update" => {}, _ => return None }
                    if let Some(destination) = kind.get("move_path").filter(|value| !value.is_null()) { paths.push(destination.as_str()?); }
                }
                Some(paths)
            }).unwrap_or_default();
            Some((id, "file_change".into(), serde_json::json!({"paths": paths})))
        }
        "mcpToolCall" => {
            let server = item.get("server").and_then(Value::as_str)?;
            let tool = item.get("tool").and_then(Value::as_str)?;
            Some((id, mcp_tool_name(server, tool), item.get("arguments").cloned().unwrap_or(Value::Null)))
        }
        _ => None,
    }
}

/// (output text, is_error) for a finished tool item.
fn item_result(item: &Value) -> (String, bool) {
    let status = item.get("status").and_then(Value::as_str).unwrap_or_default();
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "commandExecution" => {
            let output = item.get("aggregatedOutput").and_then(Value::as_str).unwrap_or_default().to_string();
            let failed = status == "failed" || item.get("exitCode").and_then(Value::as_i64).is_some_and(|c| c != 0);
            (output, failed)
        }
        "mcpToolCall" => {
            if let Some(message) = item.pointer("/error/message").and_then(Value::as_str) {
                return (message.to_string(), true);
            }
            let text: String = item
                .pointer("/result/content")
                .and_then(Value::as_array)
                .map(|blocks| blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            (text, status == "failed")
        }
        _ => (format!("file change {status}"), status == "failed"),
    }
}

/// The JSON `result` that answers an approval request.
pub fn approval_response(kind: RequestKind, decision: Decision, available: &[String]) -> Value {
    if kind == RequestKind::McpElicitation {
        return match decision {
            Decision::Deny => serde_json::json!({"action": "decline"}),
            Decision::Approve | Decision::AlwaysAllow => serde_json::json!({"action": "accept", "content": null}),
        };
    }
    let offers = |word: &str| available.iter().any(|a| a == word);
    let word = match decision {
        Decision::Approve => "accept",
        Decision::AlwaysAllow if offers("acceptForSession") => "acceptForSession",
        Decision::AlwaysAllow => "accept",
        // The server lists what it accepts. "cancel" also interrupts the turn, so it is the fallback.
        Decision::Deny if available.is_empty() || offers("decline") => "decline",
        Decision::Deny => "cancel",
    };
    serde_json::json!({"decision": word})
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

    fn feed(m: &mut Mapper, v: Value) -> Vec<Item> {
        m.on_message(&v)
    }

    fn command_item(status: &str, exit: Value, output: Value) -> Value {
        json!({"type": "commandExecution", "id": "c1", "command": "ls -la", "cwd": "/tmp/p", "status": status, "exitCode": exit, "aggregatedOutput": output})
    }

    #[test]
    fn rename_approvals_include_the_destination_and_reject_partial_paths() {
        let mut mapper = Mapper::with_mode(SessionMode::Workspace);
        for (changes, expected) in [
            (json!([{"path":"/root/a","kind":{"type":"update","move_path":"/outside"}}]), vec!["/root/a", "/outside"]),
            (json!([{"path":"/root/a","kind":{"type":"add"}}, {"kind":{"type":"add"}}]), vec![]),
            (json!([{"path":"/root/a","kind":{"type":"update","move_path":true}}]), vec![]),
        ] {
            feed(&mut mapper, json!({"method":"item/started","params":{"item":{"id":"rename","type":"fileChange","changes":changes}}}));
            let items = feed(&mut mapper, json!({"id":1,"method":"item/fileChange/requestApproval","params":{"itemId":"rename","grantRoot":null}}));
            let EventKind::ApprovalRequest { paths, input, .. } = events(&items)[0] else { panic!() };
            assert_eq!(paths, &expected);
            assert_eq!(input["paths"], json!(expected));
        }
    }

    #[test]
    fn workspace_recording_uses_file_paths_and_escape_fallback() {
        let mut mapper = Mapper::with_mode(SessionMode::Workspace); let mut edits = 0; let mut commands = 0;
        for line in include_str!("../../fixtures/codex-workspace-turn.jsonl").lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            for item in mapper.on_message(&value) {
                if let Item::Event(e) = item && let EventKind::ApprovalRequest { tool, paths, kind, .. } = e.kind {
                    if tool == "command" { assert_eq!(kind, ApprovalKind::SandboxEscape); commands += 1; }
                    if tool == "file_change" { assert!(!paths.is_empty()); edits += 1; }
                }
            }
        }
        assert!(edits >= 2 && commands >= 4);
    }

    /// Replays `codex-autorun-turn.jsonl`, recorded from `codex_workspace.py --on-request`: a linked worktree
    /// under `on-request`. Returns every approval as (item id, tool, kind) and the timeline built with autorun on.
    fn replay_autorun() -> (Vec<(String, String, ApprovalKind)>, discard_core::timeline::Timeline) {
        let mut mapper = Mapper::with_mode(SessionMode::Workspace).with_autorun(true);
        let mut timeline = discard_core::timeline::Timeline::new();
        timeline.set_autorun(true);
        let mut approvals = Vec::new();
        for line in include_str!("../../fixtures/codex-autorun-turn.jsonl").lines() {
            for item in mapper.on_message(&serde_json::from_str(line).unwrap()) {
                if let Item::Event(e) = item {
                    if let EventKind::ApprovalRequest { call_id, tool, kind, .. } = &e.kind { approvals.push((call_id.clone(), tool.clone(), *kind)); }
                    timeline.apply(&e.kind);
                }
            }
        }
        (approvals, timeline)
    }

    /// The `sandboxed` mark of the tool call whose input text contains `needle`.
    fn sandboxed_mark(timeline: &discard_core::timeline::Timeline, needle: &str) -> Option<bool> {
        use discard_core::model::Block;
        timeline.blocks().iter().find_map(|block| match block {
            Block::ToolCall { input, sandboxed, .. } if input.to_string().contains(needle) => Some(*sandboxed),
            _ => None,
        }).unwrap_or_else(|| panic!("no tool call containing {needle}"))
    }

    #[test]
    fn autorun_recording_asks_only_to_leave_the_sandbox() {
        let (approvals, _) = replay_autorun();
        let commands: Vec<_> = approvals.iter().filter(|(_, tool, _)| tool == "command").collect();
        // The one escalation is the only command approval. Every command approval is a full-access card.
        assert_eq!(commands.len(), 1, "{approvals:?}");
        assert_eq!(commands[0].2, ApprovalKind::SandboxEscape);
        // The sandbox retry ("command failed; retry without sandbox?") is an escape whatever its path is.
        assert!(approvals.contains(&("onrequest_4".into(), "file_change".into(), ApprovalKind::SandboxEscape)), "{approvals:?}");
        // Other file-change requests keep today's classification. The engine reclassifies them by path.
        for id in ["onrequest_1", "onrequest_2", "onrequest_5", "onrequest_9"] {
            assert!(approvals.contains(&(id.into(), "file_change".into(), ApprovalKind::ProviderTool)), "{id}: {approvals:?}");
        }
        // Commands and the in-root edit that ran in the sandbox raised no request.
        let asked: Vec<&str> = approvals.iter().map(|(id, ..)| id.as_str()).collect();
        for id in ["onrequest_10", "onrequest_11", "onrequest_12", "onrequest_13", "onrequest_14"] {
            assert!(!asked.contains(&id), "{id} asked: {approvals:?}");
        }
    }

    #[test]
    fn autorun_recording_marks_only_commands_that_ran_without_a_card() {
        let (_, timeline) = replay_autorun();
        assert_eq!(sandboxed_mark(&timeline, "touch inside.txt"), Some(true));
        assert_eq!(sandboxed_mark(&timeline, "git add -A"), Some(true), "a command that failed in the sandbox still ran there");
        assert_eq!(sandboxed_mark(&timeline, "example.com"), Some(true));
        assert_eq!(sandboxed_mark(&timeline, "escalation-marker"), None, "an escalation had a card");
        assert_eq!(sandboxed_mark(&timeline, "edit.txt"), None, "edits are not shell calls");
    }

    #[test]
    fn a_sandbox_retry_is_an_escape_only_under_autorun() {
        let request = json!({"method": "item/fileChange/requestApproval", "id": 0, "params": {"itemId": "f1", "reason": "command failed; retry without sandbox?", "grantRoot": null}});
        let kind = |mut mapper: Mapper| {
            let items = mapper.on_message(&request);
            items.iter().find_map(|i| match i { Item::Event(e) => match &e.kind { EventKind::ApprovalRequest { kind, .. } => Some(*kind), _ => None }, _ => None }).unwrap()
        };
        assert_eq!(kind(Mapper::with_mode(SessionMode::Workspace).with_autorun(true)), ApprovalKind::SandboxEscape);
        assert_eq!(kind(Mapper::with_mode(SessionMode::Workspace)), ApprovalKind::ProviderTool, "untrusted sessions keep today's classification");
        assert_eq!(kind(Mapper::with_mode(SessionMode::Coding).with_autorun(true)), ApprovalKind::ProviderTool);
    }

    #[test]
    fn turn_started_reports_the_turn_and_working_status() {
        let mut m = Mapper::new();
        let items = feed(&mut m, json!({"method": "turn/started", "params": {"threadId": "t", "turn": {"id": "turn-9"}}}));
        assert_eq!(items[0], Item::TurnStarted("turn-9".into()));
        assert_eq!(events(&items), vec![&EventKind::StatusChange { status: AgentStatus::Working }]);
    }

    #[test]
    fn agent_message_deltas_become_text_with_the_raw_message_attached() {
        let mut m = Mapper::new();
        let msg = json!({"method": "item/agentMessage/delta", "params": {"itemId": "m", "delta": "hi"}});
        let items = feed(&mut m, msg.clone());
        assert_eq!(events(&items), vec![&EventKind::TextDelta { text: "hi".into() }]);
        let Item::Event(e) = &items[0] else { panic!() };
        assert_eq!(e.raw.as_ref(), Some(&msg));
    }

    #[test]
    fn command_items_become_a_tool_call_start_and_result() {
        let mut m = Mapper::new();
        let start = feed(&mut m, json!({"method": "item/started", "params": {"item": command_item("inProgress", Value::Null, Value::Null)}}));
        assert_eq!(events(&start), vec![&EventKind::ToolCallStart { call_id: "c1".into(), name: "command".into(), input: json!({"command": "ls -la", "cwd": "/tmp/p"}) }]);
        let done = feed(&mut m, json!({"method": "item/completed", "params": {"item": command_item("completed", json!(0), json!("out"))}}));
        assert_eq!(events(&done), vec![&EventKind::ToolCallResult { call_id: "c1".into(), output: "out".into(), is_error: false }]);
    }

    #[test]
    fn a_nonzero_exit_or_failed_status_is_an_error_result() {
        let mut m = Mapper::new();
        for item in [command_item("completed", json!(2), json!("bad")), command_item("failed", Value::Null, Value::Null)] {
            let done = feed(&mut m, json!({"method": "item/completed", "params": {"item": item}}));
            let EventKind::ToolCallResult { is_error, .. } = events(&done).last().unwrap() else { panic!() };
            assert!(is_error);
        }
    }

    #[test]
    fn a_completed_item_that_never_started_still_gets_a_start_event_first() {
        let mut m = Mapper::new();
        let done = feed(&mut m, json!({"method": "item/completed", "params": {"item": command_item("completed", json!(0), json!("x"))}}));
        let kinds = events(&done);
        assert!(matches!(kinds[0], EventKind::ToolCallStart { .. }) && matches!(kinds[1], EventKind::ToolCallResult { .. }));
    }

    #[test]
    fn an_item_that_started_is_not_started_twice() {
        let mut m = Mapper::new();
        feed(&mut m, json!({"method": "item/started", "params": {"item": command_item("inProgress", Value::Null, Value::Null)}}));
        let done = feed(&mut m, json!({"method": "item/completed", "params": {"item": command_item("completed", json!(0), json!("x"))}}));
        assert_eq!(events(&done).len(), 1);
    }

    #[test]
    fn file_changes_list_their_paths_and_report_failure() {
        let mut m = Mapper::new();
        let item = |status: &str| json!({"type": "fileChange", "id": "f1", "status": status, "changes": [{"path": "a.rs", "kind": {"type": "add"}, "diff": "+x"}, {"path": "b.rs", "kind": {"type": "update"}, "diff": "-y"}]});
        let start = feed(&mut m, json!({"method": "item/started", "params": {"item": item("inProgress")}}));
        assert_eq!(events(&start), vec![&EventKind::ToolCallStart { call_id: "f1".into(), name: "file_change".into(), input: json!({"paths": ["a.rs", "b.rs"]}) }]);
        let done = feed(&mut m, json!({"method": "item/completed", "params": {"item": item("failed")}}));
        let EventKind::ToolCallResult { is_error, output, .. } = events(&done)[0] else { panic!() };
        assert!(*is_error && output.contains("failed"), "{output}");
    }

    #[test]
    fn mcp_tool_calls_use_the_claude_style_tool_name_and_extract_result_text() {
        let mut m = Mapper::new();
        let item = |status: &str, result: Value, error: Value| json!({"type": "mcpToolCall", "id": "k1", "server": "discard-memory", "tool": "memory_append", "arguments": {"scope": "agent"}, "status": status, "result": result, "error": error});
        let start = feed(&mut m, json!({"method": "item/started", "params": {"item": item("inProgress", Value::Null, Value::Null)}}));
        assert_eq!(events(&start), vec![&EventKind::ToolCallStart { call_id: "k1".into(), name: "mcp__discard-memory__memory_append".into(), input: json!({"scope": "agent"}) }]);
        let ok = feed(&mut m, json!({"method": "item/completed", "params": {"item": item("completed", json!({"content": [{"type": "text", "text": "Saved"}]}), Value::Null)}}));
        assert_eq!(events(&ok), vec![&EventKind::ToolCallResult { call_id: "k1".into(), output: "Saved".into(), is_error: false }]);
        let mut m2 = Mapper::new();
        let failed = feed(&mut m2, json!({"method": "item/completed", "params": {"item": item("failed", Value::Null, json!({"message": "nope"}))}}));
        let EventKind::ToolCallResult { output, is_error, .. } = events(&failed).last().unwrap() else { panic!() };
        assert!(*is_error && output == "nope");
    }

    #[test]
    fn other_items_and_notifications_are_ignored() {
        let mut m = Mapper::new();
        for msg in [
            json!({"method": "item/started", "params": {"item": {"type": "reasoning", "id": "r"}}}),
            json!({"method": "item/started", "params": {"item": {"type": "userMessage", "id": "u"}}}),
            json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "a", "text": "x"}}}),
            json!({"method": "hook/started", "params": {}}),
            json!({"method": "mcpServer/startupStatus/updated", "params": {}}),
            json!({"method": "account/rateLimits/updated", "params": {}}),
            json!({"method": "thread/status/changed", "params": {}}),
            json!({"method": "serverRequest/resolved", "params": {}}),
            json!({"no_method": 1}),
            json!("text"),
        ] {
            assert!(feed(&mut m, msg.clone()).is_empty(), "{msg}");
        }
    }

    #[test]
    fn usage_comes_from_the_last_token_update_and_is_reported_when_the_turn_ends() {
        let mut m = Mapper::new();
        let update = |i: u64, o: u64| json!({"method": "thread/tokenUsage/updated", "params": {"tokenUsage": {"total": {"inputTokens": i, "outputTokens": o, "totalTokens": i + o}}}});
        assert!(feed(&mut m, update(100, 5)).is_empty());
        assert!(feed(&mut m, update(250, 40)).is_empty());
        let done = feed(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "t", "status": "completed", "error": null}}}));
        assert_eq!(events(&done), vec![&EventKind::Usage { input_tokens: 250, output_tokens: 40 }, &EventKind::StatusChange { status: AgentStatus::Idle }]);
    }

    #[test]
    fn a_failed_turn_reports_its_error_before_idle() {
        let mut m = Mapper::new();
        let done = feed(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "t", "status": "failed", "error": {"message": "rate limited"}}}}));
        let kinds = events(&done);
        assert_eq!(kinds[1], &EventKind::Error { message: "rate limited".into() });
        assert_eq!(kinds.last().unwrap(), &&EventKind::StatusChange { status: AgentStatus::Idle });
    }

    #[test]
    fn a_failed_turn_without_a_message_still_reports_an_error() {
        let mut m = Mapper::new();
        let done = feed(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "t", "status": "failed", "error": null}}}));
        assert!(events(&done).iter().any(|k| matches!(k, EventKind::Error { .. })));
    }

    #[test]
    fn an_interrupted_turn_ends_idle_without_an_error() {
        let mut m = Mapper::new();
        let done = feed(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "t", "status": "interrupted", "error": null}}}));
        assert!(!events(&done).iter().any(|k| matches!(k, EventKind::Error { .. })));
        assert_eq!(events(&done).last().unwrap(), &&EventKind::StatusChange { status: AgentStatus::Idle });
    }

    #[test]
    fn error_notifications_are_reported_unless_codex_will_retry() {
        let mut m = Mapper::new();
        let fatal = feed(&mut m, json!({"method": "error", "params": {"error": {"message": "boom"}, "willRetry": false}}));
        assert_eq!(events(&fatal), vec![&EventKind::Error { message: "boom".into() }]);
        assert!(feed(&mut m, json!({"method": "error", "params": {"error": {"message": "blip"}, "willRetry": true}})).is_empty());
    }

    #[test]
    fn command_approval_requests_become_pending_approvals_with_the_offered_decisions() {
        let mut m = Mapper::new();
        let items = feed(
            &mut m,
            json!({"method": "item/commandExecution/requestApproval", "id": 0, "params": {"kind": "command", "itemId": "c1", "command": "touch x", "cwd": "/tmp/p", "reason": "write",
                "availableDecisions": ["accept", {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": ["touch"]}}, "cancel"]}}),
        );
        let Item::Request(req) = &items[0] else { panic!("expected a pending request first") };
        assert_eq!((req.rpc_id.clone(), req.kind, req.available.clone()), (json!(0), RequestKind::Command, vec!["accept".to_string(), "cancel".to_string()]));
        let EventKind::ApprovalRequest { approval_id, call_id, tool, input, .. } = events(&items)[0] else { panic!() };
        assert_eq!((call_id.as_str(), tool.as_str()), ("c1", "command"));
        assert_eq!(input["command"], "touch x");
        assert_eq!(approval_id, &req.approval_id);
    }

    #[test]
    fn string_request_ids_are_kept_as_they_came() {
        let mut m = Mapper::new();
        let items = feed(&mut m, json!({"method": "item/fileChange/requestApproval", "id": "req-7", "params": {"itemId": "f1", "reason": "extra write access"}}));
        let Item::Request(req) = &items[0] else { panic!() };
        assert_eq!((req.rpc_id.clone(), req.kind), (json!("req-7"), RequestKind::FileChange));
        let EventKind::ApprovalRequest { tool, input, .. } = events(&items)[0] else { panic!() };
        assert_eq!(tool, "file_change");
        assert_eq!(input["reason"], "extra write access");
    }

    #[test]
    fn mcp_elicitations_are_shown_as_tool_approvals_for_the_server() {
        let mut m = Mapper::new();
        let items = feed(&mut m, json!({"method": "mcpServer/elicitation/request", "id": 3, "params": {"serverName": "discard-memory", "threadId": "t", "message": "Allow memory_append?", "_meta": {"tool_name": "memory_append"}}}));
        let Item::Request(req) = &items[0] else { panic!() };
        assert_eq!(req.kind, RequestKind::McpElicitation);
        let EventKind::ApprovalRequest { tool, .. } = events(&items)[0] else { panic!() };
        assert_eq!(tool, "mcp__discard-memory__memory_append");
        let items = feed(&mut m, json!({"method": "mcpServer/elicitation/request", "id": 4, "params": {"serverName": "other", "threadId": "t"}}));
        let EventKind::ApprovalRequest { tool, .. } = events(&items)[0] else { panic!() };
        assert_eq!(tool, "mcp__other__elicitation");
    }

    #[test]
    fn unsupported_server_requests_are_rejected_so_codex_does_not_wait() {
        let mut m = Mapper::new();
        for method in ["item/permissions/requestApproval", "item/tool/requestUserInput", "item/tool/call", "attestation/generate", "applyPatchApproval", "execCommandApproval", "account/chatgptAuthTokens/refresh"] {
            let items = feed(&mut m, json!({"method": method, "id": 11, "params": {}}));
            assert!(matches!(&items[0], Item::RejectRequest { rpc_id, .. } if *rpc_id == json!(11)), "{method}: {items:?}");
            assert!(events(&items).iter().any(|k| matches!(k, EventKind::Error { .. })), "{method} must be visible to the user");
        }
    }

    #[test]
    fn decisions_map_to_the_words_codex_accepts() {
        let all: Vec<String> = ["accept", "acceptForSession", "decline", "cancel"].map(String::from).into();
        let r = |k, d, a: &[String]| approval_response(k, d, a);
        assert_eq!(r(RequestKind::Command, Decision::Approve, &all), json!({"decision": "accept"}));
        assert_eq!(r(RequestKind::Command, Decision::AlwaysAllow, &all), json!({"decision": "acceptForSession"}));
        assert_eq!(r(RequestKind::Command, Decision::Deny, &all), json!({"decision": "decline"}));
        assert_eq!(r(RequestKind::FileChange, Decision::Deny, &all), json!({"decision": "decline"}));
    }

    #[test]
    fn decisions_fall_back_to_what_the_server_offered() {
        let offered: Vec<String> = ["accept", "cancel"].map(String::from).into();
        assert_eq!(approval_response(RequestKind::Command, Decision::Deny, &offered), json!({"decision": "cancel"}));
        assert_eq!(approval_response(RequestKind::Command, Decision::AlwaysAllow, &offered), json!({"decision": "accept"}));
        assert_eq!(approval_response(RequestKind::Command, Decision::Approve, &offered), json!({"decision": "accept"}));
        assert_eq!(approval_response(RequestKind::Command, Decision::Deny, &[]), json!({"decision": "decline"}), "unknown offers: use the standard word");
    }

    #[test]
    fn elicitations_use_the_action_form() {
        assert_eq!(approval_response(RequestKind::McpElicitation, Decision::Approve, &[]), json!({"action": "accept", "content": null}));
        assert_eq!(approval_response(RequestKind::McpElicitation, Decision::AlwaysAllow, &[]), json!({"action": "accept", "content": null}));
        assert_eq!(approval_response(RequestKind::McpElicitation, Decision::Deny, &[]), json!({"action": "decline"}));
    }

    #[test]
    fn recorded_transcript_maps_to_the_expected_event_order() {
        let text = include_str!("../../fixtures/codex-approval-turn.jsonl");
        let mut m = Mapper::new();
        let mut names: Vec<&str> = Vec::new();
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            for item in m.on_message(&v) {
                if let Item::Event(e) = item {
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
            }
        }
        assert_eq!(names, vec!["working", "text", "tool_start", "approval", "tool_result", "text", "usage", "idle"]);
    }
}
