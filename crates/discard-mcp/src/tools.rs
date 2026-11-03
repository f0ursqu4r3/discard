//! MCP JSON-RPC handling and the memory tools. Pure: no network, no global state.

use crate::identity::{Identity, ScopeKindOrd, Surface};
use discard_core::memory::{MemoryError, MemoryStore, Scope};
use serde_json::{Value, json};
use std::sync::Arc;

pub const SERVER_NAME: &str = "discard-memory";
pub const LATEST_PROTOCOL: &str = "2025-03-26";
/// Claude names MCP tools `mcp__<server>__<tool>`. Discard auto-approves its own memory tools by this prefix.
pub const TOOL_PREFIX: &str = "mcp__discard-memory__";

pub struct McpHandler {
    store: Arc<MemoryStore>,
}

impl McpHandler {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }

    /// Handle one JSON-RPC message. `None` means no reply (a notification).
    pub fn handle(&self, identity: &Identity, request: &Value) -> Option<Value> {
        let Some(object) = request.as_object() else {
            let why = if request.is_array() { "batch requests are not supported" } else { "request must be a JSON object" };
            return Some(error(&Value::Null, -32600, why));
        };
        let id = object.get("id").cloned();
        let reply_id = id.clone().unwrap_or(Value::Null);
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(error(&reply_id, -32600, "request needs a string `method`"));
        };
        // A request without an id is a notification: nothing to answer, whatever it is.
        id.as_ref()?;
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let wanted = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
                let version = if SUPPORTED.contains(&wanted) { wanted } else { LATEST_PROTOCOL };
                result(
                    &reply_id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
                    }),
                )
            }
            "ping" => result(&reply_id, json!({})),
            "tools/list" => result(&reply_id, json!({"tools": tool_list(identity)})),
            "tools/call" => self.call(identity, &reply_id, &params),
            other if other.starts_with("notifications/") => return None,
            _ => error(&reply_id, -32601, "method not found"),
        })
    }

    fn call(&self, identity: &Identity, id: &Value, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        let known = identity.surface == Surface::Memory && ALL_TOOLS.contains(&name);
        if !known {
            return error(id, -32602, "unknown tool");
        }
        let outcome = self.run_tool(identity, name, params.get("arguments").unwrap_or(&Value::Null));
        match outcome {
            Ok(text) => result(id, json!({"content": [{"type": "text", "text": text}], "isError": false})),
            Err(message) => result(id, json!({"content": [{"type": "text", "text": message}], "isError": true})),
        }
    }

    fn run_tool(&self, identity: &Identity, name: &str, args: &Value) -> Result<String, String> {
        if !args.is_object() {
            return Err("arguments must be an object".to_string());
        }
        let writes = matches!(name, "memory_append" | "memory_edit");
        if writes && !identity.can_write {
            return Err("this session may not change memory".to_string());
        }
        match name {
            "memory_search" => {
                let query = str_arg(args, "query")?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10).clamp(1, 25) as usize;
                let scopes = match args.get("scope") {
                    None | Some(Value::Null) => {
                        [ScopeKindOrd::Agent, ScopeKindOrd::Project, ScopeKindOrd::Global]
                            .into_iter()
                            .filter(|k| identity.scopes.contains(k))
                            .map(|k| scope_for(identity, k))
                            .collect()
                    }
                    Some(_) => vec![resolve_scope(identity, args)?],
                };
                let hits = self.store.search(&scopes, query, limit).map_err(describe)?;
                if hits.is_empty() {
                    return Ok("no matches".to_string());
                }
                Ok(hits
                    .iter()
                    .map(|h| format!("[{}] {}: {}", h.scope.split(':').next().unwrap_or("?"), h.path, h.snippet.replace('\n', " ")))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "memory_list" => {
                let scope = resolve_scope(identity, args)?;
                let files = self.store.list(&scope).map_err(describe)?;
                if files.is_empty() {
                    return Ok("no files in this scope".to_string());
                }
                Ok(files.iter().map(|f| format!("{} ({} bytes, version {})", f.path, f.bytes, f.version)).collect::<Vec<_>>().join("\n"))
            }
            "memory_read" => {
                let scope = resolve_scope(identity, args)?;
                let content = self.store.read(&scope, str_arg(args, "path")?).map_err(describe)?;
                Ok(format!("version: {}\n\n{}", content.version, content.text))
            }
            "memory_append" => {
                let scope = resolve_scope(identity, args)?;
                let path = str_arg(args, "path")?;
                let version = self.store.append(&scope, path, str_arg(args, "text")?).map_err(describe)?;
                Ok(format!("Saved {path} (version {version})"))
            }
            "memory_edit" => {
                let scope = resolve_scope(identity, args)?;
                let path = str_arg(args, "path")?;
                let version = self
                    .store
                    .edit(&scope, path, str_arg(args, "old")?, str_arg(args, "new")?, str_arg(args, "version")?)
                    .map_err(describe)?;
                Ok(format!("Edited {path} (version {version})"))
            }
            _ => Err("unknown tool".to_string()),
        }
    }
}

const SUPPORTED: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
const READ_TOOLS: &[&str] = &["memory_list", "memory_read", "memory_search"];
const WRITE_TOOLS: &[&str] = &["memory_append", "memory_edit"];
const ALL_TOOLS: &[&str] = &["memory_list", "memory_read", "memory_search", "memory_append", "memory_edit"];

fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("`{key}` is required and must be a string"))
}

fn scope_for(identity: &Identity, kind: ScopeKindOrd) -> Scope {
    match kind {
        ScopeKindOrd::Agent => Scope::Agent(identity.agent.clone()),
        ScopeKindOrd::Project => Scope::Project(identity.project.clone()),
        ScopeKindOrd::Global => Scope::Global,
    }
}

/// The `scope` argument, checked against what this session may use.
fn resolve_scope(identity: &Identity, args: &Value) -> Result<Scope, String> {
    let name = str_arg(args, "scope")?;
    let kind = match name {
        "agent" => ScopeKindOrd::Agent,
        "project" => ScopeKindOrd::Project,
        "global" => ScopeKindOrd::Global,
        other => return Err(format!("unknown scope `{other}`: use agent, project, or global")),
    };
    if !identity.scopes.contains(&kind) {
        return Err(format!("scope `{name}` is not permitted for this session"));
    }
    Ok(scope_for(identity, kind))
}

/// A message for the agent. Storage details such as absolute paths stay in the logs.
fn describe(e: MemoryError) -> String {
    match e {
        MemoryError::Io(detail) | MemoryError::Index(detail) => {
            tracing::warn!(error = %detail, "memory storage error");
            "memory storage error".to_string()
        }
        other => other.to_string(),
    }
}

fn tool_list(identity: &Identity) -> Vec<Value> {
    if identity.surface != Surface::Memory {
        return Vec::new();
    }
    let scope = json!({"type": "string", "enum": ["agent", "project", "global"], "description": "agent: private to you. project: shared in this project. global: shared by all agents."});
    let defs = |name: &str| -> Value {
        match name {
            "memory_list" => json!({"name": name, "description": "List the memory files in a scope, with sizes and versions.",
                "inputSchema": {"type": "object", "properties": {"scope": scope}, "required": ["scope"]}}),
            "memory_read" => json!({"name": name, "description": "Read a memory file. The first line of the result is its version; pass it to memory_edit.",
                "inputSchema": {"type": "object", "properties": {"scope": scope, "path": {"type": "string", "description": "Relative path ending in .md, like core/persona.md"}}, "required": ["scope", "path"]}}),
            "memory_search" => json!({"name": name, "description": "Search memory files by words. Searches every scope you can use unless `scope` is given.",
                "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "scope": scope, "limit": {"type": "integer", "minimum": 1, "maximum": 25}}, "required": ["query"]}}),
            "memory_append" => json!({"name": name, "description": "Add a note at the end of a memory file. Creates the file if it does not exist. Files under core/ are shown to you at the start of every session.",
                "inputSchema": {"type": "object", "properties": {"scope": scope, "path": {"type": "string"}, "text": {"type": "string"}}, "required": ["scope", "path", "text"]}}),
            _ => json!({"name": name, "description": "Replace one piece of text in a memory file. `old` must appear exactly once. `version` comes from memory_read; a stale version is refused.",
                "inputSchema": {"type": "object", "properties": {"scope": scope, "path": {"type": "string"}, "old": {"type": "string"}, "new": {"type": "string"}, "version": {"type": "string"}}, "required": ["scope", "path", "old", "new", "version"]}}),
        }
    };
    let mut names: Vec<&str> = READ_TOOLS.to_vec();
    if identity.can_write {
        names.extend(WRITE_TOOLS);
    }
    names.into_iter().map(defs).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::model::{AgentId, ProjectId};

    struct Rig {
        handler: McpHandler,
        store: Arc<MemoryStore>,
        _dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open_in_memory_index(dir.path()).unwrap());
        Rig { handler: McpHandler::new(Arc::clone(&store)), store, _dir: dir }
    }

    fn id(scopes: &[ScopeKindOrd], can_write: bool) -> Identity {
        Identity {
            agent: AgentId("a1".into()),
            project: ProjectId("p1".into()),
            surface: Surface::Memory,
            scopes: scopes.iter().copied().collect(),
            can_write,
        }
    }

    fn full() -> Identity {
        id(&[ScopeKindOrd::Agent, ScopeKindOrd::Project], true)
    }

    fn rpc(method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
    }

    fn call(r: &Rig, who: &Identity, tool: &str, args: Value) -> Value {
        r.handler.handle(who, &rpc("tools/call", json!({"name": tool, "arguments": args}))).expect("a reply")
    }

    fn text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    fn is_error(reply: &Value) -> bool {
        reply["result"]["isError"].as_bool().unwrap_or(false)
    }

    #[test]
    fn initialize_reports_server_info_and_the_tools_capability() {
        let r = rig();
        let reply = r.handler.handle(&full(), &rpc("initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}))).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
    }

    #[test]
    fn an_unknown_protocol_version_gets_the_latest_one() {
        let r = rig();
        let reply = r.handler.handle(&full(), &rpc("initialize", json!({"protocolVersion": "1999-01-01"}))).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], LATEST_PROTOCOL);
    }

    #[test]
    fn notifications_get_no_reply_and_ping_gets_an_empty_result() {
        let r = rig();
        assert_eq!(r.handler.handle(&full(), &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})), None);
        assert_eq!(r.handler.handle(&full(), &rpc("ping", json!({}))).unwrap()["result"], json!({}));
    }

    #[test]
    fn tools_list_describes_the_five_memory_tools_with_schemas() {
        let r = rig();
        let reply = r.handler.handle(&full(), &rpc("tools/list", json!({}))).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["memory_list", "memory_read", "memory_search", "memory_append", "memory_edit"]);
        for t in tools {
            assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn read_only_sessions_only_see_read_tools() {
        let r = rig();
        let who = id(&[ScopeKindOrd::Agent], false);
        let reply = r.handler.handle(&who, &rpc("tools/list", json!({}))).unwrap();
        let names: Vec<_> = reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["memory_list", "memory_read", "memory_search"]);
    }

    #[test]
    fn the_plugin_surface_has_no_memory_tools() {
        let r = rig();
        let mut who = full();
        who.surface = Surface::Plugins;
        let reply = r.handler.handle(&who, &rpc("tools/list", json!({}))).unwrap();
        assert!(reply["result"]["tools"].as_array().unwrap().iter().all(|t| !t["name"].as_str().unwrap().starts_with("memory_")));
        let denied = r.handler.handle(&who, &rpc("tools/call", json!({"name": "memory_read", "arguments": {"scope": "agent", "path": "x.md"}}))).unwrap();
        assert!(denied["error"].is_object(), "{denied}");
    }

    #[test]
    fn append_writes_into_the_callers_own_agent_scope() {
        let r = rig();
        let reply = call(&r, &full(), "memory_append", json!({"scope": "agent", "path": "core/persona.md", "text": "I am Scout"}));
        assert!(!is_error(&reply), "{reply}");
        assert_eq!(r.store.read(&Scope::Agent(AgentId("a1".into())), "core/persona.md").unwrap().text, "I am Scout");
        assert!(r.store.read(&Scope::Agent(AgentId("other".into())), "core/persona.md").is_err());
    }

    #[test]
    fn project_scope_maps_to_the_callers_project() {
        let r = rig();
        call(&r, &full(), "memory_append", json!({"scope": "project", "path": "notes.md", "text": "shared"}));
        assert_eq!(r.store.read(&Scope::Project(ProjectId("p1".into())), "notes.md").unwrap().text, "shared");
    }

    #[test]
    fn a_scope_the_session_may_not_use_is_denied_and_nothing_is_written() {
        let r = rig();
        let reply = call(&r, &full(), "memory_append", json!({"scope": "global", "path": "x.md", "text": "nope"}));
        assert!(is_error(&reply));
        assert!(text(&reply).contains("not permitted"), "{}", text(&reply));
        assert!(r.store.read(&Scope::Global, "x.md").is_err());
        assert!(is_error(&call(&r, &full(), "memory_read", json!({"scope": "global", "path": "x.md"}))));
    }

    #[test]
    fn read_only_sessions_cannot_write() {
        let r = rig();
        let who = id(&[ScopeKindOrd::Agent], false);
        let reply = call(&r, &who, "memory_append", json!({"scope": "agent", "path": "x.md", "text": "no"}));
        assert!(is_error(&reply));
        assert!(r.store.read(&Scope::Agent(AgentId("a1".into())), "x.md").is_err());
    }

    #[test]
    fn read_returns_the_text_and_version_and_edit_uses_that_version() {
        let r = rig();
        call(&r, &full(), "memory_append", json!({"scope": "agent", "path": "x.md", "text": "alpha beta"}));
        let read = text(&call(&r, &full(), "memory_read", json!({"scope": "agent", "path": "x.md"})));
        let version = read.lines().next().unwrap().strip_prefix("version: ").unwrap().to_string();
        assert!(read.ends_with("alpha beta"), "{read}");
        let edit = call(&r, &full(), "memory_edit", json!({"scope": "agent", "path": "x.md", "old": "beta", "new": "gamma", "version": version}));
        assert!(!is_error(&edit), "{edit}");
        assert_eq!(r.store.read(&Scope::Agent(AgentId("a1".into())), "x.md").unwrap().text, "alpha gamma");
    }

    #[test]
    fn a_stale_edit_is_an_error_that_tells_the_agent_to_read_again() {
        let r = rig();
        call(&r, &full(), "memory_append", json!({"scope": "agent", "path": "x.md", "text": "one"}));
        let reply = call(&r, &full(), "memory_edit", json!({"scope": "agent", "path": "x.md", "old": "one", "new": "two", "version": "0000000000000000"}));
        assert!(is_error(&reply));
        assert!(text(&reply).contains("changed since"), "{}", text(&reply));
    }

    #[test]
    fn list_shows_files_with_sizes_and_versions() {
        let r = rig();
        call(&r, &full(), "memory_append", json!({"scope": "agent", "path": "a.md", "text": "12345"}));
        let out = text(&call(&r, &full(), "memory_list", json!({"scope": "agent"})));
        assert!(out.contains("a.md") && out.contains("5 bytes"), "{out}");
        assert!(text(&call(&r, &full(), "memory_list", json!({"scope": "project"}))).contains("no files"));
    }

    #[test]
    fn search_covers_permitted_scopes_by_default_and_one_scope_when_named() {
        let r = rig();
        call(&r, &full(), "memory_append", json!({"scope": "agent", "path": "a.md", "text": "needle in agent"}));
        call(&r, &full(), "memory_append", json!({"scope": "project", "path": "p.md", "text": "needle in project"}));
        r.store.create(&Scope::Global, "g.md", "needle in global").unwrap();
        let all = text(&call(&r, &full(), "memory_search", json!({"query": "needle"})));
        assert!(all.contains("a.md") && all.contains("p.md"), "{all}");
        assert!(!all.contains("g.md"), "global is not permitted: {all}");
        let one = text(&call(&r, &full(), "memory_search", json!({"query": "needle", "scope": "project"})));
        assert!(one.contains("p.md") && !one.contains("a.md"), "{one}");
        assert!(text(&call(&r, &full(), "memory_search", json!({"query": "zzzz"}))).contains("no matches"));
    }

    #[test]
    fn bad_arguments_are_tool_errors_not_crashes() {
        let r = rig();
        for args in [json!({}), json!({"scope": "agent"}), json!({"scope": 5, "path": "x.md"}), json!({"scope": "galaxy", "path": "x.md"}), json!({"scope": "agent", "path": "../x.md"}), json!("string"), json!(null)] {
            let reply = call(&r, &full(), "memory_read", args.clone());
            assert!(is_error(&reply), "{args}: {reply}");
        }
    }

    #[test]
    fn error_messages_do_not_reveal_filesystem_paths() {
        let r = rig();
        let reply = call(&r, &full(), "memory_read", json!({"scope": "agent", "path": "missing.md"}));
        let msg = text(&reply);
        assert!(!msg.contains(r._dir.path().to_str().unwrap()), "{msg}");
    }

    #[test]
    fn unknown_tools_and_methods_are_protocol_errors_with_the_request_id() {
        let r = rig();
        let reply = r.handler.handle(&full(), &rpc("tools/call", json!({"name": "rm_rf", "arguments": {}}))).unwrap();
        assert_eq!((reply["error"]["code"].as_i64(), reply["id"].clone()), (Some(-32602), json!(7)));
        let reply = r.handler.handle(&full(), &rpc("resources/list", json!({}))).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn malformed_requests_are_rejected() {
        let r = rig();
        for bad in [json!("x"), json!(5), json!([]), json!({"jsonrpc": "2.0", "id": 1}), json!({"jsonrpc": "2.0", "id": 1, "method": 5}), json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}])] {
            let reply = r.handler.handle(&full(), &bad).unwrap();
            assert_eq!(reply["error"]["code"], -32600, "{bad}");
        }
    }
}
