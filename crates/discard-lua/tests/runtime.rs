//! The Luau runtime: loading, rendering, sandboxing, limits, and isolation.

use discard_lua::element::{Node, Prop, Tag};
use discard_lua::snapshot::Snapshot;
use discard_lua::{PluginError, PluginHandle, PluginLimits, Status, Violation};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn manifest(caps: &str) -> String {
    format!("name = \"t\"\nversion = \"1\"\napi = \"1\"\n{caps}")
}

const UI: &str = "[capabilities]\nui = true\n";

fn snapshot(caps: &str, files: &[(&str, &str)]) -> Snapshot {
    let mut map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    map.insert("plugin.toml".into(), manifest(caps).into_bytes());
    for (p, b) in files {
        map.insert((*p).into(), b.as_bytes().to_vec());
    }
    Snapshot::from_files(map)
}

fn limits() -> PluginLimits {
    PluginLimits { call_budget: Duration::from_millis(150), memory_bytes: 16 * 1024 * 1024, queue_capacity: 8 }
}

fn load(caps: &str, init: &str) -> Result<(PluginHandle, discard_lua::LoadedInfo), PluginError> {
    PluginHandle::spawn(snapshot(caps, &[("init.lua", init)]), limits())
}

fn ok(caps: &str, init: &str) -> (PluginHandle, discard_lua::LoadedInfo) {
    load(caps, init).unwrap_or_else(|e| panic!("load failed: {e}"))
}

#[test]
fn menu_item_callback_receives_only_kind_and_id_and_items_keep_order() {
    let (h, info) = ok(UI, r#"
        local keys = ""
        discard.menu_item("message", "Pin", function(target)
            local k = {} for key in target do table.insert(k, key) end table.sort(k)
            keys = table.concat(k, ",") .. ":" .. target.kind .. ":" .. target.id
        end)
        discard.menu_item("app", "Other", function() end)
        discard.command("keys", function() return keys end)
    "#);
    assert_eq!(info.menu_items.iter().map(|item| item.label.as_str()).collect::<Vec<_>>(), ["Pin", "Other"]);
    h.run_menu_item(0, "message", "m1").unwrap();
    assert_eq!(h.run_command("keys", "").unwrap().as_deref(), Some("id,kind:message:m1"));
}

#[test]
fn menu_item_accepts_the_four_new_targets() {
    let (_, info) = load(UI, "for _, target in {'diff_block', 'tool_call', 'agent', 'server'} do discard.menu_item(target, 'Run', function() end) end").unwrap();
    assert_eq!(info.menu_items.len(), 4);
}

#[test]
fn invalid_menu_items_fail_to_load() {
    for (source, expected) in [
        ("discard.menu_item('diff', 'ok', function() end)".to_string(), "unknown menu target `diff`"),
        ("discard.menu_item('app', 'bad\\nlabel', function() end)".to_string(), "menu label must be 1 to 40 characters"),
        (format!("discard.menu_item('app', '{}', function() end)", "x".repeat(41)), "menu label must be 1 to 40 characters"),
        ("for i = 1, 9 do discard.menu_item('app', 'ok', function() end) end".to_string(), "too many menu items (max 8)"),
    ] {
        let Err(PluginError::Load(message)) = load(UI, &source) else { panic!("expected load error") };
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn menu_item_needs_the_ui_capability() {
    let Err(PluginError::Load(message)) = load("", "discard.menu_item('app', 'ok', function() end)") else { panic!("expected load error") };
    assert!(message.contains("ui"), "{message}");
}

#[test]
fn loading_reports_what_the_plugin_registered() {
    let (_h, info) = ok(
        UI,
        r##"
        discard.theme({ primary = "#112233", radius = 6 })
        discard.slot("channel_header", function(s) return { "text", "x" } end)
        discard.command("hello", function(args) return "hi" end)
        discard.action("go", function() end)
        discard.on("message_sent", function(e) end)
        "##,
    );
    assert_eq!(info.manifest.name, "t");
    assert_eq!(info.slots, ["channel_header"]);
    assert_eq!(info.commands, ["hello"]);
    assert_eq!(info.actions, ["go"]);
    assert_eq!(info.events, ["message_sent"]);
    assert_eq!(info.theme.color("primary").unwrap().to_hex(), "#112233");
    assert_eq!(info.theme.radius(), Some(6.0));
    assert_eq!(info.hash.len(), 64);
}

#[test]
fn render_passes_state_in_and_returns_a_validated_element() {
    let (h, _) = ok(
        UI,
        r##"discard.slot("header", function(state)
            return { "row", gap = 4, { "text", state.channel.name }, { "badge", count = #state.items } }
        end)"##,
    );
    let e = h.render("header", &json!({"channel": {"name": "general"}, "items": [1, 2, 3]})).unwrap();
    assert_eq!(e.tag, Tag::Row);
    assert_eq!(e.props["gap"], Prop::Number(4.0));
    let Node::Element(text) = &e.children[0] else { panic!() };
    assert_eq!(text.children, vec![Node::Text("general".into())]);
    let Node::Element(badge) = &e.children[1] else { panic!() };
    assert_eq!(badge.props["count"], Prop::Number(3.0));
}

#[test]
fn rendering_an_unknown_slot_is_a_clear_error() {
    let (h, _) = ok(UI, "");
    assert_eq!(h.render("nope", &json!({})), Err(PluginError::NotRegistered { kind: "slot", name: "nope".into() }));
}

#[test]
fn an_invalid_element_is_an_error_but_the_plugin_keeps_running() {
    let (h, _) = ok(UI, r##"discard.slot("s", function() return { "script" } end) discard.slot("t", function() return { "text", "fine" } end)"##);
    assert!(matches!(h.render("s", &json!({})), Err(PluginError::Element(_))));
    assert_eq!(h.status(), Status::Running);
    assert!(h.render("t", &json!({})).is_ok());
}

#[test]
fn a_lua_error_is_reported_and_the_plugin_keeps_running() {
    let (h, _) = ok(UI, r##"discard.slot("s", function() error("boom") end) discard.slot("t", function() return { "text", "fine" } end)"##);
    let Err(PluginError::Runtime(msg)) = h.render("s", &json!({})) else { panic!() };
    assert!(msg.contains("boom"), "{msg}");
    assert_eq!(h.status(), Status::Running);
    assert!(h.render("t", &json!({})).is_ok());
}

#[test]
fn an_infinite_loop_is_killed_by_the_cpu_budget_and_the_plugin_is_terminated() {
    let (h, _) = ok(UI, r##"discard.slot("s", function() while true do end end)"##);
    let started = Instant::now();
    assert_eq!(h.render("s", &json!({})), Err(PluginError::Violation(Violation::Cpu)));
    assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
    assert!(matches!(h.status(), Status::Terminated(_)));
    assert!(matches!(h.render("s", &json!({})), Err(PluginError::Terminated(_))));
}

#[test]
fn a_memory_hog_is_terminated() {
    let (h, _) = ok(UI, r##"discard.slot("s", function() local t = {} for i = 1, 1e9 do t[i] = string.rep("x", 1024) .. i end return { "text" } end)"##);
    assert_eq!(h.render("s", &json!({})), Err(PluginError::Violation(Violation::Memory)));
    assert!(matches!(h.status(), Status::Terminated(_)));
}

#[test]
fn an_infinite_loop_while_loading_fails_the_load() {
    let r = load(UI, "while true do end");
    assert!(matches!(r, Err(PluginError::Violation(Violation::Cpu))), "{r:?}");
}

#[test]
fn a_syntax_error_fails_the_load_with_a_message() {
    let Err(PluginError::Load(msg)) = load(UI, "this is not lua") else { panic!() };
    assert!(!msg.is_empty());
}

#[test]
fn ui_functions_need_the_ui_capability() {
    let Err(PluginError::Load(msg)) = load("", r##"discard.slot("s", function() return { "text" } end)"##) else { panic!("expected a load error") };
    assert!(msg.contains("ui"), "{msg}");
    let Err(PluginError::Load(msg)) = load("", r##"discard.theme({ primary = "#000000" })"##) else { panic!() };
    assert!(msg.contains("ui"), "{msg}");
}

#[test]
fn a_bad_theme_token_fails_the_load() {
    let Err(PluginError::Load(msg)) = load(UI, r##"discard.theme({ sparkle = "#000000" })"##) else { panic!() };
    assert!(msg.contains("sparkle"), "{msg}");
    let Err(PluginError::Load(msg)) = load(UI, r##"discard.theme({ primary = "blue" })"##) else { panic!() };
    assert!(msg.contains("blue"), "{msg}");
}

#[test]
fn the_sandbox_has_no_io_process_or_package_access() {
    let (h, _) = ok(
        UI,
        r##"discard.slot("s", function()
            local present = {}
            for _, name in ipairs({ "io", "package", "dofile", "loadfile", "require" }) do
                if _G[name] ~= nil then present[#present + 1] = name end
            end
            local has_exec = os ~= nil and (os.execute ~= nil or os.getenv ~= nil or os.remove ~= nil)
            return { "text", table.concat(present, ",") .. (has_exec and " os-exec" or "") }
        end)"##,
    );
    let e = h.render("s", &json!({})).unwrap();
    // `require` is the snapshot loader. Everything else must be absent.
    assert_eq!(e.children, vec![Node::Text("require".into())]);
}

#[test]
fn builtins_cannot_be_modified() {
    let r = load(UI, "string.rep = nil");
    assert!(matches!(r, Err(PluginError::Load(_))), "{:?}", r.map(|_| ()));
    let r = load(UI, "discard.slot = nil");
    assert!(matches!(r, Err(PluginError::Load(_))));
}

#[test]
fn require_loads_modules_from_the_snapshot_only() {
    let snap = snapshot(
        UI,
        &[
            ("init.lua", r##"local u = require("lib.util") discard.slot("s", function() return { "text", u.name } end)"##),
            ("lib/util.lua", r##"return { name = "from-snapshot" }"##),
        ],
    );
    let (h, _) = PluginHandle::spawn(snap, limits()).unwrap();
    assert_eq!(h.render("s", &json!({})).unwrap().children, vec![Node::Text("from-snapshot".into())]);
}

#[test]
fn require_rejects_escapes_missing_modules_and_cycles() {
    for src in [r##"require("../secret")"##, r##"require("/etc/passwd")"##, r##"require("missing")"##] {
        assert!(matches!(load(UI, src), Err(PluginError::Load(_))), "{src}");
    }
    let snap = snapshot(UI, &[("init.lua", r##"require("a")"##), ("a.lua", r##"require("b")"##), ("b.lua", r##"require("a")"##)]);
    let Err(PluginError::Load(msg)) = PluginHandle::spawn(snap, limits()) else { panic!() };
    assert!(msg.contains("cyclic"), "{msg}");
}

#[test]
fn modules_are_cached() {
    let snap = snapshot(
        UI,
        &[
            ("init.lua", r##"local a = require("m") local b = require("m") assert(a == b, "not cached")"##),
            ("m.lua", "return {}"),
        ],
    );
    assert!(PluginHandle::spawn(snap, limits()).is_ok());
}

#[test]
fn commands_return_text_and_unknown_commands_error() {
    let (h, _) = ok(UI, r##"discard.command("echo", function(args) return "you said " .. args end) discard.command("quiet", function() end)"##);
    assert_eq!(h.run_command("echo", "hi").unwrap().as_deref(), Some("you said hi"));
    assert_eq!(h.run_command("quiet", "").unwrap(), None);
    assert_eq!(h.run_command("zzz", ""), Err(PluginError::NotRegistered { kind: "command", name: "zzz".into() }));
}

#[test]
fn actions_run_and_keep_plugin_state_between_calls() {
    let (h, _) = ok(
        UI,
        r##"local n = 0
        discard.action("inc", function() n = n + 1 end)
        discard.slot("s", function() return { "text", n } end)"##,
    );
    h.run_action("inc").unwrap();
    h.run_action("inc").unwrap();
    assert_eq!(h.render("s", &json!({})).unwrap().children, vec![Node::Text("2".into())]);
    assert_eq!(h.run_action("zzz"), Err(PluginError::NotRegistered { kind: "action", name: "zzz".into() }));
}

#[test]
fn hooks_can_block_and_the_first_blocker_wins() {
    let (h, _) = ok(
        UI,
        r##"discard.on("tool_call", function(e) if e.tool == "Bash" then return { block = true, reason = "no shell" } end end)
           discard.on("tool_call", function(e) return { block = true, reason = "second" } end)"##,
    );
    let blocked = h.hook("tool_call", &json!({"tool": "Bash"})).unwrap();
    assert_eq!((blocked.block, blocked.reason.as_deref()), (true, Some("no shell")));
    let other = h.hook("tool_call", &json!({"tool": "Read"})).unwrap();
    assert_eq!(other.reason.as_deref(), Some("second"));
}

#[test]
fn hooks_with_no_handler_or_no_result_do_not_block() {
    let (h, _) = ok(UI, r##"discard.on("x", function() end)"##);
    assert!(!h.hook("x", &json!({})).unwrap().block);
    assert!(!h.hook("unregistered", &json!({})).unwrap().block);
}

#[test]
fn a_hook_that_errors_returns_an_error_so_the_caller_can_fail_closed() {
    let (h, _) = ok(UI, r##"discard.on("x", function() error("bad hook") end)"##);
    assert!(matches!(h.hook("x", &json!({})), Err(PluginError::Runtime(_))));
}

#[test]
fn emit_never_blocks_and_drops_old_events_for_a_slow_plugin() {
    let (h, _) = ok(
        UI,
        r##"local seen = 0
        discard.on("tick", function() local t = os.clock() while os.clock() - t < 0.02 do end seen = seen + 1 end)
        discard.command("seen", function() return tostring(seen) end)"##,
    );
    let started = Instant::now();
    for i in 0..500 {
        h.emit("tick", &json!({"i": i}));
    }
    assert!(started.elapsed() < Duration::from_millis(500), "emit blocked for {:?}", started.elapsed());
    std::thread::sleep(Duration::from_millis(400));
    let seen: usize = h.run_command("seen", "").unwrap().unwrap().parse().unwrap();
    assert!(seen < 500, "all events were processed: {seen}");
    assert_eq!(h.status(), Status::Running);
}

#[test]
fn a_looping_plugin_does_not_block_another_plugin() {
    let (bad, _) = ok(UI, r##"discard.slot("s", function() while true do end end)"##);
    let (good, _) = ok(UI, r##"discard.slot("s", function() return { "text", "ok" } end)"##);
    let t = std::thread::spawn(move || bad.render("s", &json!({})));
    let started = Instant::now();
    assert!(good.render("s", &json!({})).is_ok());
    assert!(started.elapsed() < Duration::from_millis(100), "good plugin was delayed: {:?}", started.elapsed());
    let _ = t.join().unwrap();
}

#[test]
fn plugins_do_not_share_globals() {
    let (a, _) = ok(UI, r##"shared = "a" discard.slot("s", function() return { "text", tostring(shared) } end)"##);
    let (b, _) = ok(UI, r##"discard.slot("s", function() return { "text", tostring(shared) } end)"##);
    assert_eq!(a.render("s", &json!({})).unwrap().children, vec![Node::Text("a".into())]);
    assert_eq!(b.render("s", &json!({})).unwrap().children, vec![Node::Text("nil".into())]);
}

#[test]
fn shutdown_stops_the_plugin_thread() {
    let (h, _) = ok(UI, "");
    h.shutdown();
}

#[test]
fn the_loaded_bytes_come_from_the_snapshot_not_the_disk() {
    let snap = snapshot(UI, &[("init.lua", r##"discard.slot("s", function() return { "text", "v1" } end)"##)]);
    let hash = snap.hash();
    let (h, info) = PluginHandle::spawn(snap, limits()).unwrap();
    assert_eq!(info.hash, hash);
    assert_eq!(h.render("s", &json!({})).unwrap().children, vec![Node::Text("v1".into())]);
}
