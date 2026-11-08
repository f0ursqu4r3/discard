//! The plugin-authoring MCP tools, against a real PluginService.

use discard_core::memory::MemoryStore;
use discard_core::model::{AgentId, ProjectId};
use discard_lua::keystore::StaticKeyStore;
use discard_lua::ledger::Ledger;
use discard_lua::{PluginLimits, PluginManager, PluginService};
use discard_mcp::identity::{Identity, ScopeKindOrd, Surface};
use discard_mcp::tools::McpHandler;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Rig {
    handler: McpHandler,
    svc: Arc<PluginService>,
    dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let ledger = Ledger::open(&dir.path().join("plugins.db"), Arc::new(StaticKeyStore([3; 32]))).unwrap();
    let manager = Arc::new(Mutex::new(PluginManager::new(PluginLimits { call_budget: Duration::from_millis(200), ..PluginLimits::default() })));
    let svc = Arc::new(PluginService::new(dir.path(), ledger, manager, false).unwrap());
    Rig { handler: McpHandler::new(store).with_plugins(Arc::clone(&svc)), svc, dir }
}

fn who(surface: Surface, can_write: bool) -> Identity {
    Identity { agent: AgentId("a1".into()), project: Some(ProjectId("p1".into())), source_conversation: Some(discard_core::model::ChannelId("chat".into())), write_scopes: [ScopeKindOrd::AgentProject].into(), surface, scopes: [ScopeKindOrd::Agent].into(), can_write }
}

fn writer() -> Identity {
    who(Surface::Plugins, true)
}

fn call(r: &Rig, who: &Identity, tool: &str, args: Value) -> Value {
    r.handler.handle(who, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})).unwrap()
}

fn text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn is_error(reply: &Value) -> bool {
    reply["result"]["isError"].as_bool().unwrap_or(false)
}

const MANIFEST: &str = "name = \"warm\"\nversion = \"1\"\napi = \"1\"\n[capabilities]\nui = true\n";
const INIT: &str = "discard.slot(\"channel_header\", function(s) return { \"text\", \"hello\" } end)";

fn create_warm(r: &Rig) {
    let reply = call(r, &writer(), "plugin_create", json!({"name": "warm", "files": {"plugin.toml": MANIFEST, "init.lua": INIT}}));
    assert!(!is_error(&reply), "{reply}");
}

fn tool_names(r: &Rig, who: &Identity) -> Vec<String> {
    let reply = r.handler.handle(who, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_writer_sees_all_eight_tools_and_none_of_them_can_approve() {
    let names = tool_names(&rig(), &writer());
    assert_eq!(names, ["plugin_api_docs", "plugin_list", "ui_inspect", "plugin_create", "plugin_edit", "plugin_test", "plugin_reload", "plugin_rollback"]);
    assert!(!names.iter().any(|n| n.contains("approve") || n.contains("install") || n.contains("trust")), "approval belongs to the user only");
}

#[test]
fn a_session_without_plugin_write_sees_only_the_read_tools() {
    let r = rig();
    assert_eq!(tool_names(&r, &who(Surface::Plugins, false)), ["plugin_api_docs", "plugin_list", "ui_inspect"]);
    for tool in ["plugin_create", "plugin_edit", "plugin_test", "plugin_reload", "plugin_rollback"] {
        let reply = call(&r, &who(Surface::Plugins, false), tool, json!({"name": "x"}));
        assert!(is_error(&reply), "{tool} must be refused without plugin.write");
    }
    assert!(r.svc.drafts().list().unwrap().is_empty());
}

#[test]
fn the_memory_surface_cannot_use_plugin_tools_and_without_a_service_there_are_none() {
    let r = rig();
    let reply = call(&r, &who(Surface::Memory, true), "plugin_create", json!({"name": "x", "files": {}}));
    assert_eq!(reply["error"]["code"], -32602);
    assert!(tool_names(&r, &who(Surface::Memory, true)).iter().all(|n| n.starts_with("memory_")));
    let dir = tempfile::tempdir().unwrap();
    let bare = McpHandler::new(Arc::new(MemoryStore::open_in_memory_index(dir.path()).unwrap()));
    let reply = bare.handle(&writer(), &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    assert!(reply["result"]["tools"].as_array().unwrap().is_empty());
}

#[test]
fn api_docs_include_the_reference_the_types_and_the_guide() {
    let r = rig();
    let t = text(&call(&r, &writer(), "plugin_api_docs", json!({})));
    for needle in ["discard.slot", "declare discard:", "Writing Discard plugins", "plugin_reload"] {
        assert!(t.contains(needle), "missing {needle}");
    }
}

#[test]
fn create_makes_a_draft_and_points_at_the_next_step() {
    let r = rig();
    let reply = call(&r, &writer(), "plugin_create", json!({"name": "warm", "files": {"plugin.toml": MANIFEST, "init.lua": INIT}}));
    assert!(!is_error(&reply));
    assert!(text(&reply).contains("plugin_test"), "{}", text(&reply));
    assert_eq!(r.svc.drafts().list().unwrap(), ["warm"]);
    assert!(r.svc.manager().lock().unwrap().names().is_empty(), "a draft must not run");
}

#[test]
fn create_rejects_bad_names_paths_and_argument_shapes() {
    let r = rig();
    for args in [
        json!({"name": "Bad Name", "files": {"init.lua": "x"}}),
        json!({"name": "ok", "files": {"../evil.lua": "x"}}),
        json!({"name": "ok", "files": {"init.sh": "x"}}),
        json!({"name": "ok", "files": "not an object"}),
        json!({"name": "ok", "files": {"init.lua": 5}}),
        json!({"files": {}}),
        json!({"name": 5, "files": {}}),
    ] {
        assert!(is_error(&call(&r, &writer(), "plugin_create", args.clone())), "{args}");
    }
    assert!(r.svc.drafts().list().unwrap().is_empty());
}

#[test]
fn create_can_start_from_an_installed_plugin() {
    let r = rig();
    create_warm(&r);
    let review = r.svc.submit("warm").unwrap();
    r.svc.approve(&review.id, false).unwrap();
    r.svc.drafts().discard("warm").unwrap();
    let reply = call(&r, &writer(), "plugin_create", json!({"name": "warm", "from_installed": true}));
    assert!(!is_error(&reply), "{reply}");
    assert_eq!(r.svc.drafts().read_file("warm", "init.lua").unwrap(), INIT);
    assert!(is_error(&call(&r, &writer(), "plugin_create", json!({"name": "nothing-installed", "from_installed": true}))));
}

#[test]
fn edit_can_write_replace_and_delete_files() {
    let r = rig();
    create_warm(&r);
    assert!(!is_error(&call(&r, &writer(), "plugin_edit", json!({"name": "warm", "path": "lib/util.lua", "content": "return 1"}))));
    assert_eq!(r.svc.drafts().read_file("warm", "lib/util.lua").unwrap(), "return 1");
    assert!(!is_error(&call(&r, &writer(), "plugin_edit", json!({"name": "warm", "path": "init.lua", "old": "hello", "new": "howdy"}))));
    assert!(r.svc.drafts().read_file("warm", "init.lua").unwrap().contains("howdy"));
    assert!(!is_error(&call(&r, &writer(), "plugin_edit", json!({"name": "warm", "path": "lib/util.lua", "delete": true}))));
    assert!(r.svc.drafts().read_file("warm", "lib/util.lua").is_err());
}

#[test]
fn edit_rejects_ambiguous_requests_bad_paths_and_missing_drafts() {
    let r = rig();
    create_warm(&r);
    for args in [
        json!({"name": "warm", "path": "init.lua"}),
        json!({"name": "warm", "path": "init.lua", "content": "x", "old": "a", "new": "b"}),
        json!({"name": "warm", "path": "init.lua", "old": "zzz", "new": "b"}),
        json!({"name": "warm", "path": "../x.lua", "content": "x"}),
        json!({"name": "nope", "path": "init.lua", "content": "x"}),
        json!({"name": "warm", "path": "init.lua", "old": "a"}),
    ] {
        assert!(is_error(&call(&r, &writer(), "plugin_edit", args.clone())), "{args}");
    }
    assert_eq!(r.svc.drafts().read_file("warm", "init.lua").unwrap(), INIT, "failed edits change nothing");
}

#[test]
fn test_reports_what_the_draft_would_do_without_installing() {
    let r = rig();
    create_warm(&r);
    let reply = call(&r, &writer(), "plugin_test", json!({"name": "warm"}));
    assert!(!is_error(&reply));
    let t = text(&reply);
    assert!(t.contains("loaded") && t.contains("channel_header") && t.contains("ui"), "{t}");
    assert!(r.svc.manager().lock().unwrap().names().is_empty());
}

#[test]
fn test_supplies_the_documented_agent_id_to_plugin_slots() {
    let r = rig();
    let init = r#"discard.slot("member_list", function(state)
        assert(type(state.agent.id) == "string" and #state.agent.id > 0, "missing agent ID")
        return { "column", agent = state.agent.id,
            { "avatar", agent = state.agent.id, name = state.agent.name },
            { "text", state.agent.name, color = "agent", variant = "mono" },
            { "spinner", label = "Syncing" },
        }
    end)"#;
    let created = call(&r, &writer(), "plugin_create", json!({"name": "warm", "files": {"plugin.toml": MANIFEST, "init.lua": init}}));
    assert!(!is_error(&created), "{created}");
    let report = text(&call(&r, &writer(), "plugin_test", json!({"name": "warm"})));
    assert!(report.contains("Slot `member_list`: rendered OK"), "{report}");
}

#[test]
fn test_shows_load_and_render_errors_so_the_agent_can_fix_them() {
    let r = rig();
    call(&r, &writer(), "plugin_create", json!({"name": "broken", "files": {"plugin.toml": MANIFEST.replace("warm", "broken"), "init.lua": "this is not lua"}}));
    let t = text(&call(&r, &writer(), "plugin_test", json!({"name": "broken"})));
    assert!(t.contains("did not load"), "{t}");
    call(&r, &writer(), "plugin_create", json!({"name": "slow", "files": {"plugin.toml": MANIFEST.replace("warm", "slow"), "init.lua": "discard.slot(\"channel_header\", function() error(\"kaboom\") end)"}}));
    let t = text(&call(&r, &writer(), "plugin_test", json!({"name": "slow"})));
    assert!(t.contains("kaboom"), "{t}");
}

#[test]
fn reload_queues_a_review_for_the_user_and_nothing_goes_live() {
    let r = rig();
    create_warm(&r);
    let reply = call(&r, &writer(), "plugin_reload", json!({"name": "warm"}));
    assert!(!is_error(&reply), "{reply}");
    let t = text(&reply);
    assert!(t.contains("review") && t.contains("user"), "{t}");
    assert_eq!(r.svc.pending().len(), 1);
    assert!(r.svc.manager().lock().unwrap().names().is_empty(), "submitting must not load anything");
    assert!(!r.dir.path().join("plugins/warm").exists());
}

#[test]
fn reload_refuses_drafts_that_would_waste_the_users_time() {
    let r = rig();
    call(&r, &writer(), "plugin_create", json!({"name": "bad", "files": {"plugin.toml": MANIFEST.replace("warm", "bad"), "init.lua": "this is not lua"}}));
    assert!(is_error(&call(&r, &writer(), "plugin_reload", json!({"name": "bad"}))));
    assert!(r.svc.pending().is_empty());
    assert!(is_error(&call(&r, &writer(), "plugin_reload", json!({"name": "missing"}))));
}

#[test]
fn rollback_reverts_an_installed_plugin_and_errors_for_unknown_ones() {
    let r = rig();
    create_warm(&r);
    let review = r.svc.submit("warm").unwrap();
    r.svc.approve(&review.id, false).unwrap();
    assert!(!is_error(&call(&r, &writer(), "plugin_rollback", json!({"name": "warm"}))));
    assert!(is_error(&call(&r, &writer(), "plugin_rollback", json!({"name": "warm"}))));
}

#[test]
fn list_shows_installed_plugins_drafts_and_pending_reviews() {
    let r = rig();
    create_warm(&r);
    call(&r, &writer(), "plugin_reload", json!({"name": "warm"}));
    call(&r, &writer(), "plugin_create", json!({"name": "other", "files": {"plugin.toml": MANIFEST.replace("warm", "other"), "init.lua": INIT}}));
    let t = text(&call(&r, &writer(), "plugin_list", json!({})));
    assert!(t.contains("drafts") && t.contains("other") && t.contains("warm"), "{t}");
    assert!(t.contains("awaiting review"), "{t}");
    let review = r.svc.pending().remove(0);
    r.svc.approve(&review.id, false).unwrap();
    assert!(text(&call(&r, &writer(), "plugin_list", json!({}))).contains("installed"));
}

#[test]
fn ui_inspect_returns_the_current_slot_trees_as_json() {
    let r = rig();
    create_warm(&r);
    let review = r.svc.submit("warm").unwrap();
    r.svc.approve(&review.id, false).unwrap();
    r.svc.set_ui_state(json!({"channel": "general"}));
    let v: Value = serde_json::from_str(&text(&call(&r, &who(Surface::Plugins, false), "ui_inspect", json!({})))).expect("JSON");
    assert_eq!(v["slots"]["channel_header"]["plugin"], "warm");
}

#[test]
fn error_messages_do_not_reveal_filesystem_paths() {
    let r = rig();
    let msg = text(&call(&r, &writer(), "plugin_test", json!({"name": "missing"})));
    assert!(!msg.contains(r.dir.path().to_str().unwrap()), "{msg}");
    let msg = text(&call(&r, &writer(), "plugin_edit", json!({"name": "missing", "path": "init.lua", "content": "x"})));
    assert!(!msg.contains(r.dir.path().to_str().unwrap()), "{msg}");
}

#[test]
fn arguments_must_be_an_object() {
    let r = rig();
    for args in [json!(null), json!("x"), json!([])] {
        assert!(is_error(&call(&r, &writer(), "plugin_create", args)));
    }
}
