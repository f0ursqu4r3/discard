//! MemoryRuntime: session setup, config files, revocation, and an end-to-end call over HTTP.

use discard_core::engine::MemoryService;
use discard_core::memory::{MemoryStore, Scope};
use discard_core::model::{AgentId, ProjectId};
use discard_mcp::MemoryRuntime;
use discard_mcp::identity::{ScopeKindOrd, Surface};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Rig {
    runtime: Arc<MemoryRuntime>,
    store: Arc<MemoryStore>,
    dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start(Arc::clone(&store), &dir.path().join("run")).await.unwrap();
    Rig { runtime, store, dir }
}

fn ids() -> (AgentId, ProjectId) {
    (AgentId("agent-1".into()), ProjectId("proj-1".into()))
}

fn config(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn prepare_issues_a_token_for_this_agent_and_project() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, false).unwrap();
    let identity = r.runtime.registry().lookup(&prepared.token).expect("token is registered");
    assert_eq!((identity.agent, identity.project, identity.surface, identity.can_write), (a, p, Surface::Memory, true));
    assert!(identity.scopes.contains(&ScopeKindOrd::Agent) && identity.scopes.contains(&ScopeKindOrd::Project) && identity.scopes.contains(&ScopeKindOrd::Global));
}

#[tokio::test]
async fn the_config_file_points_at_the_server_with_the_token_and_is_private() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, false).unwrap();
    let cfg = config(&prepared.mcp_config);
    let server = &cfg["mcpServers"]["discard-memory"];
    assert_eq!(server["type"], "http");
    assert_eq!(server["url"], r.runtime.url());
    assert_eq!(server["headers"]["Authorization"], format!("Bearer {}", prepared.token));
    assert_eq!(mode(&prepared.mcp_config), 0o600);
    assert_eq!(mode(prepared.mcp_config.parent().unwrap()), 0o700);
    assert!(prepared.mcp_config.starts_with(r.dir.path().join("run")));
}

#[tokio::test]
async fn the_token_is_not_in_the_system_prompt_or_the_file_name() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, Some("Be brief."), false).unwrap();
    assert!(!prepared.system_prompt.contains(&prepared.token));
    assert!(!prepared.mcp_config.to_string_lossy().contains(&prepared.token));
}

#[tokio::test]
async fn the_system_prompt_carries_core_blocks_from_all_three_scopes() {
    let r = rig().await;
    let (a, p) = ids();
    r.store.create(&Scope::Agent(a.clone()), "core/persona.md", "I am Scout").unwrap();
    r.store.create(&Scope::Project(p.clone()), "core/project.md", "Uses Rust").unwrap();
    r.store.create(&Scope::Global, "core/user.md", "Kyle").unwrap();
    r.store.create(&Scope::Agent(a.clone()), "notes/skip.md", "not a core block").unwrap();
    let prompt = r.runtime.prepare(&a, &p, Some("Be brief."), false).unwrap().system_prompt;
    assert!(prompt.starts_with("Be brief."));
    for needle in ["I am Scout", "Uses Rust", "Kyle", "memory_search"] {
        assert!(prompt.contains(needle), "missing {needle}: {prompt}");
    }
    assert!(!prompt.contains("not a core block"));
}

#[tokio::test]
async fn other_agents_core_blocks_are_not_included() {
    let r = rig().await;
    let (a, p) = ids();
    r.store.create(&Scope::Agent(AgentId("someone-else".into())), "core/persona.md", "OTHER AGENT SECRET").unwrap();
    assert!(!r.runtime.prepare(&a, &p, None, false).unwrap().system_prompt.contains("OTHER AGENT SECRET"));
}

#[tokio::test]
async fn finish_revokes_the_token_and_deletes_the_file_and_is_safe_to_repeat() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, false).unwrap();
    r.runtime.finish(&prepared.token);
    assert!(r.runtime.registry().lookup(&prepared.token).is_none());
    assert!(!prepared.mcp_config.exists());
    r.runtime.finish(&prepared.token);
    r.runtime.finish("never-issued");
}

#[tokio::test]
async fn each_prepare_gets_its_own_token_and_file() {
    let r = rig().await;
    let (a, p) = ids();
    let one = r.runtime.prepare(&a, &p, None, false).unwrap();
    let two = r.runtime.prepare(&a, &p, None, false).unwrap();
    assert_ne!(one.token, two.token);
    assert_ne!(one.mcp_config, two.mcp_config);
    r.runtime.finish(&one.token);
    assert!(r.runtime.registry().lookup(&two.token).is_some(), "finishing one session leaves the other alone");
    assert!(two.mcp_config.exists());
}

#[tokio::test]
async fn end_to_end_the_config_works_over_http_until_the_session_finishes() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, false).unwrap();
    let cfg = config(&prepared.mcp_config);
    let url = cfg["mcpServers"]["discard-memory"]["url"].as_str().unwrap().to_string();
    let auth = cfg["mcpServers"]["discard-memory"]["headers"]["Authorization"].as_str().unwrap().to_string();
    let addr: std::net::SocketAddr = url.trim_start_matches("http://").trim_end_matches("/mcp").parse().unwrap();

    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "memory_append", "arguments": {"scope": "agent", "path": "core/persona.md", "text": "from http"}}}).to_string();
    let post = |auth: String, body: String| async move {
        let mut s = TcpStream::connect(addr).await.unwrap();
        let req = format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).await.unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).await.unwrap();
        raw.split_whitespace().nth(1).unwrap().to_string()
    };
    assert_eq!(post(auth.clone(), body.clone()).await, "200");
    assert_eq!(r.store.read(&Scope::Agent(a.clone()), "core/persona.md").unwrap().text, "from http");
    r.runtime.finish(&prepared.token);
    assert_eq!(post(auth, body).await, "401");
}

#[tokio::test]
async fn shutdown_closes_the_server() {
    let r = rig().await;
    let addr: std::net::SocketAddr = r.runtime.url().trim_start_matches("http://").trim_end_matches("/mcp").parse().unwrap();
    r.runtime.shutdown().await;
    assert!(TcpStream::connect(addr).await.is_err());
}

// ---- plugin authoring ----

use discard_core::memory::Scope as MemScope;

fn config_servers(path: &Path) -> Value {
    config(path)["mcpServers"].clone()
}

fn bearer_of(server: &Value) -> String {
    server["headers"]["Authorization"].as_str().unwrap().trim_start_matches("Bearer ").to_string()
}

#[tokio::test]
async fn agents_without_plugin_write_get_only_the_memory_server() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, false).unwrap();
    let servers = config_servers(&prepared.mcp_config);
    assert_eq!(servers.as_object().unwrap().keys().collect::<Vec<_>>(), ["discard-memory"]);
    assert!(r.store.read(&MemScope::Agent(a), "core/plugin-guide.md").is_err(), "no guide for agents that cannot write plugins");
}

#[tokio::test]
async fn agents_with_plugin_write_get_a_second_server_with_its_own_token() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, true).unwrap();
    let servers = config_servers(&prepared.mcp_config);
    let (memory, plugins) = (&servers["discard-memory"], &servers["discard-plugins"]);
    assert_eq!(plugins["url"], r.runtime.url());
    let (memory_token, plugin_token) = (bearer_of(memory), bearer_of(plugins));
    assert_ne!(memory_token, plugin_token, "separate connections need separate tokens");
    assert_eq!(memory_token, prepared.token);
    let (mi, pi) = (r.runtime.registry().lookup(&memory_token).unwrap(), r.runtime.registry().lookup(&plugin_token).unwrap());
    assert_eq!((mi.surface, pi.surface), (Surface::Memory, Surface::Plugins));
    assert!(pi.can_write && pi.agent == a);
}

#[tokio::test]
async fn finishing_a_session_revokes_both_tokens_and_deletes_the_file() {
    let r = rig().await;
    let (a, p) = ids();
    let prepared = r.runtime.prepare(&a, &p, None, true).unwrap();
    let plugin_token = bearer_of(&config_servers(&prepared.mcp_config)["discard-plugins"]);
    r.runtime.finish(&prepared.token);
    assert!(r.runtime.registry().lookup(&prepared.token).is_none());
    assert!(r.runtime.registry().lookup(&plugin_token).is_none(), "the plugin token must not outlive the session");
    assert!(!prepared.mcp_config.exists());
}

#[tokio::test]
async fn the_authoring_guide_is_kept_in_the_agents_core_memory_and_refreshed() {
    let r = rig().await;
    let (a, p) = ids();
    let prompt = r.runtime.prepare(&a, &p, None, true).unwrap().system_prompt;
    let guide = discard_lua::api_spec::guide();
    let stored = r.store.read(&MemScope::Agent(a.clone()), "core/plugin-guide.md").unwrap();
    assert_eq!(stored.text, guide);
    assert!(prompt.contains("Writing Discard plugins"), "the guide is a core block, so it is in the prompt");
    // A stale copy is replaced.
    r.store.replace(&MemScope::Agent(a.clone()), "core/plugin-guide.md", "OLD GUIDE", &stored.version).unwrap();
    r.runtime.prepare(&a, &p, None, true).unwrap();
    assert_eq!(r.store.read(&MemScope::Agent(a), "core/plugin-guide.md").unwrap().text, guide);
}

#[tokio::test]
async fn the_plugin_tools_work_over_http_with_the_plugin_token_and_not_with_the_memory_token() {
    use discard_lua::keystore::StaticKeyStore;
    use discard_lua::ledger::Ledger;
    use discard_lua::{PluginLimits, PluginManager, PluginService};
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let ledger = Ledger::open(&dir.path().join("plugins.db"), Arc::new(StaticKeyStore([3; 32]))).unwrap();
    let manager = Arc::new(std::sync::Mutex::new(PluginManager::new(PluginLimits::default())));
    let plugins = Arc::new(PluginService::new(dir.path(), ledger, manager, false).unwrap());
    let runtime = MemoryRuntime::start_with_plugins(Arc::clone(&store), &dir.path().join("run"), Some(plugins)).await.unwrap();

    let (a, p) = ids();
    let prepared = runtime.prepare(&a, &p, None, true).unwrap();
    let servers = config_servers(&prepared.mcp_config);
    let addr: std::net::SocketAddr = runtime.url().trim_start_matches("http://").trim_end_matches("/mcp").parse().unwrap();
    let post = |token: String, tool: &'static str| async move {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": {}}}).to_string();
        let mut s = TcpStream::connect(addr).await.unwrap();
        let req = format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).await.unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).await.unwrap();
        serde_json::from_str::<Value>(raw.split("\r\n\r\n").nth(1).unwrap_or("{}")).unwrap_or(Value::Null)
    };
    let ok = post(bearer_of(&servers["discard-plugins"]), "plugin_api_docs").await;
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    let denied = post(bearer_of(&servers["discard-memory"]), "plugin_api_docs").await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
}
