//! The `discard.fs` and `discard.memory` functions: scope enforcement from Lua, and the draft sandbox.

use discard_lua::snapshot::Snapshot;
use discard_lua::{Host, MemoryFile, MemoryHit, MemoryHost, MemoryText, PluginHandle, PluginLimits};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn limits() -> PluginLimits {
    PluginLimits { call_budget: Duration::from_millis(500), ..PluginLimits::default() }
}

fn spawn(caps: &str, init: &str, host: Host) -> PluginHandle {
    let manifest = format!("name = \"t\"\nversion = \"1\"\napi = \"1\"\n{caps}");
    let files: BTreeMap<String, Vec<u8>> =
        [("plugin.toml".to_string(), manifest.into_bytes()), ("init.lua".to_string(), init.as_bytes().to_vec())].into();
    PluginHandle::spawn_with_host(Snapshot::from_files(files), limits(), host).unwrap_or_else(|e| panic!("load failed: {e}")).0
}

fn run(h: &PluginHandle, command: &str) -> String {
    h.run_command(command, "").unwrap().unwrap_or_default()
}

fn fs_caps(dir: &Path, write: bool) -> String {
    format!("[capabilities.fs]\npaths = [\"{}\"]\nwrite = {write}\n", dir.display())
}

/// Wrap a Lua body so a failure comes back as text: `ok` or `err: <message>`.
fn probe(body: &str) -> String {
    format!(r#"discard.command("t", function() local ok, e = pcall(function() {body} end) if ok then return tostring(e) else return "err: " .. tostring(e) end end)"#)
}

#[test]
fn a_plugin_reads_and_writes_files_in_its_scope() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let h = spawn(
        &fs_caps(&root, true),
        &probe(&format!(
            r#"local d = "{}"
            discard.fs.mkdir(d .. "/sub")
            discard.fs.write(d .. "/sub/a.txt", "hello")
            local text = discard.fs.read(d .. "/sub/a.txt")
            local names = table.concat(discard.fs.list(d .. "/sub"), ",")
            local there = discard.fs.exists(d .. "/sub/a.txt")
            discard.fs.remove(d .. "/sub/a.txt")
            return text .. "|" .. names .. "|" .. tostring(there) .. "|" .. tostring(discard.fs.exists(d .. "/sub/a.txt"))"#,
            root.display()
        )),
        Host::live(None),
    );
    assert_eq!(run(&h, "t"), "hello|a.txt|true|false");
}

#[test]
fn a_plugin_cannot_leave_its_scope_even_with_dot_dot() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let scope = root.join("scope");
    std::fs::create_dir_all(&scope).unwrap();
    std::fs::write(root.join("secret.txt"), "secret").unwrap();
    let h = spawn(
        &fs_caps(&scope, true),
        &probe(&format!(r#"return discard.fs.read("{}/../secret.txt")"#, scope.display())),
        Host::live(None),
    );
    assert!(run(&h, "t").starts_with("err:"), "{}", run(&h, "t"));
    let h = spawn(&fs_caps(&scope, true), &probe(&format!(r#"return discard.fs.read("{}")"#, root.join("secret.txt").display())), Host::live(None));
    assert!(run(&h, "t").contains("outside"), "{}", run(&h, "t"));
}

#[test]
fn read_only_fs_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let h = spawn(&fs_caps(&root, false), &probe(&format!(r#"discard.fs.write("{}/x.txt", "x")"#, root.display())), Host::live(None));
    assert!(run(&h, "t").contains("read-only"), "{}", run(&h, "t"));
    assert!(!root.join("x.txt").exists());
}

#[test]
fn without_the_fs_capability_every_fs_call_fails() {
    let h = spawn("[capabilities]\nui = true\n", &probe(r#"return discard.fs.read("/etc/hosts")"#), Host::live(None));
    assert!(run(&h, "t").contains("needs the `fs` capability"), "{}", run(&h, "t"));
}

#[test]
fn a_draft_sandbox_refuses_fs_even_with_the_capability() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("f.txt"), "x").unwrap();
    let h = spawn(&fs_caps(&root, true), &probe(&format!(r#"return discard.fs.read("{}/f.txt")"#, root.display())), Host::default());
    assert!(run(&h, "t").contains("while a draft is being tested"), "{}", run(&h, "t"));
}

#[test]
fn a_failed_fs_call_does_not_stop_the_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let h = spawn(&fs_caps(&root, true), &format!("{}\ndiscard.command(\"ok\", function() return \"alive\" end)", probe(r#"return discard.fs.read("nope")"#)), Host::live(None));
    assert!(run(&h, "t").starts_with("err:"));
    assert_eq!(run(&h, "ok"), "alive");
}

#[derive(Default)]
struct FakeMemory {
    calls: Mutex<Vec<String>>,
}

impl MemoryHost for FakeMemory {
    fn list(&self, scope: &str) -> Result<Vec<MemoryFile>, String> {
        self.calls.lock().unwrap().push(format!("list:{scope}"));
        Ok(vec![MemoryFile { path: "a.md".into(), version: "v1".into(), bytes: 3 }])
    }
    fn read(&self, scope: &str, path: &str) -> Result<MemoryText, String> {
        self.calls.lock().unwrap().push(format!("read:{scope}:{path}"));
        Ok(MemoryText { text: "note".into(), version: "v1".into() })
    }
    fn search(&self, scopes: &[String], query: &str, limit: usize) -> Result<Vec<MemoryHit>, String> {
        self.calls.lock().unwrap().push(format!("search:{}:{query}:{limit}", scopes.join("+")));
        Ok(vec![MemoryHit { scope: "global".into(), path: "a.md".into(), snippet: "s".into() }])
    }
    fn append(&self, scope: &str, path: &str, text: &str) -> Result<String, String> {
        self.calls.lock().unwrap().push(format!("append:{scope}:{path}:{text}"));
        Ok("v2".into())
    }
    fn edit(&self, scope: &str, path: &str, old: &str, new: &str, version: &str) -> Result<String, String> {
        self.calls.lock().unwrap().push(format!("edit:{scope}:{path}:{old}:{new}:{version}"));
        if version == "stale" { Err("changed since you read it".into()) } else { Ok("v3".into()) }
    }
}

fn memory_host() -> (Host, Arc<FakeMemory>) {
    let m = Arc::new(FakeMemory::default());
    (Host::live(Some(m.clone())), m)
}

fn mem_caps(scopes: &str, write: bool) -> String {
    format!("[capabilities.memory]\nscopes = [{scopes}]\nwrite = {write}\n")
}

#[test]
fn a_plugin_reads_lists_searches_and_writes_memory_in_its_scopes() {
    let (host, fake) = memory_host();
    let h = spawn(
        &mem_caps("\"global\", \"project\"", true),
        &probe(
            r#"local files = discard.memory.list("global")
            local note = discard.memory.read("global", "a.md")
            local hits = discard.memory.search("needle", 5)
            local v = discard.memory.append("project", "b.md", "more")
            local v2 = discard.memory.edit("project", "b.md", "more", "less", v)
            return files[1].path .. "|" .. note.text .. "|" .. note.version .. "|" .. hits[1].snippet .. "|" .. v .. "|" .. v2"#,
        ),
        host,
    );
    assert_eq!(run(&h, "t"), "a.md|note|v1|s|v2|v3");
    assert_eq!(
        *fake.calls.lock().unwrap(),
        ["list:global", "read:global:a.md", "search:global+project:needle:5", "append:project:b.md:more", "edit:project:b.md:more:less:v2"]
    );
}

#[test]
fn a_scope_the_manifest_did_not_name_is_refused_and_never_reaches_the_host() {
    let (host, fake) = memory_host();
    let h = spawn(&mem_caps("\"global\"", true), &probe(r#"return discard.memory.read("agent", "a.md")"#), host);
    assert!(run(&h, "t").contains("scope `agent` was not granted"), "{}", run(&h, "t"));
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn read_only_memory_refuses_append_and_edit() {
    let (host, fake) = memory_host();
    let h = spawn(
        &mem_caps("\"global\"", false),
        &format!(
            "{}\n{}",
            probe(r#"return discard.memory.append("global", "a.md", "x")"#),
            r#"discard.command("e", function() local ok, e = pcall(discard.memory.edit, "global", "a.md", "a", "b", "v1") return tostring(e) end)"#
        ),
        host,
    );
    assert!(run(&h, "t").contains("read-only"), "{}", run(&h, "t"));
    assert!(run(&h, "e").contains("read-only"), "{}", run(&h, "e"));
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn without_the_memory_capability_every_memory_call_fails() {
    let (host, fake) = memory_host();
    let h = spawn("[capabilities]\nui = true\n", &probe(r#"return discard.memory.search("x")"#), host);
    assert!(run(&h, "t").contains("needs the `memory` capability"), "{}", run(&h, "t"));
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn a_draft_sandbox_refuses_memory_even_with_the_capability() {
    let h = spawn(&mem_caps("\"global\"", true), &probe(r#"return discard.memory.list("global")"#), Host::default());
    assert!(run(&h, "t").contains("while a draft is being tested"), "{}", run(&h, "t"));
}

#[test]
fn a_stale_edit_comes_back_as_a_catchable_error() {
    let (host, _) = memory_host();
    let h = spawn(&mem_caps("\"global\"", true), &probe(r#"return discard.memory.edit("global", "a.md", "a", "b", "stale")"#), host);
    assert!(run(&h, "t").contains("changed since you read it"), "{}", run(&h, "t"));
}

#[test]
fn the_search_limit_is_clamped() {
    let (host, fake) = memory_host();
    let h = spawn(&mem_caps("\"global\"", false), &probe(r#"discard.memory.search("a", 100000) discard.memory.search("b", -4) discard.memory.search("c") return "ok""#), host);
    assert_eq!(run(&h, "t"), "ok");
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls, ["search:global:a:50", "search:global:b:1", "search:global:c:10"]);
}
