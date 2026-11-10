//! The MCP server over real HTTP on loopback.

use discard_core::memory::{MemoryStore, Scope};
use discard_core::model::AgentId;
use discard_mcp::identity::{Identity, ScopeKindOrd, Surface, TokenRegistry};
use discard_mcp::tools::McpHandler;
use discard_mcp::McpServer;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Rig {
    server: McpServer,
    registry: Arc<TokenRegistry>,
    store: Arc<MemoryStore>,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::open_in_memory_index(dir.path()).unwrap());
    let registry = Arc::new(TokenRegistry::new());
    let server = McpServer::start(Arc::new(McpHandler::new(Arc::clone(&store))), Arc::clone(&registry)).await.unwrap();
    Rig { server, registry, store, _dir: dir }
}

fn identity(agent: &str) -> Identity {
    Identity {
            main_checkout_requests: false,
            main_checkout_grant: false,
            workspace_root: None,
        agent: AgentId(agent.into()),
        project: None, source_conversation: Some(discard_core::model::ChannelId("chat".into())), write_scopes: [ScopeKindOrd::Agent].into(),
        surface: Surface::Memory,
        scopes: [ScopeKindOrd::Agent, ScopeKindOrd::Project].into(),
        can_write: true,
    }
}

impl Rig {
    fn token(&self, agent: &str) -> String {
        self.registry.issue(identity(agent), Duration::from_secs(60)).unwrap()
    }
}

struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {:?}", self.body))
    }
}

async fn http(addr: std::net::SocketAddr, method: &str, auth: Option<&str>, host: Option<&str>, body: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let host = host.map_or_else(|| addr.to_string(), str::to_string);
    let mut head = format!("{method} /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len());
    if let Some(a) = auth {
        head.push_str(&format!("Authorization: {a}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw)).await;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Reply { status, body }
}

fn rpc(method: &str, params: Value) -> Vec<u8> {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string().into_bytes()
}

#[tokio::test]
async fn a_valid_token_gets_a_json_rpc_reply() {
    let r = rig().await;
    let t = r.token("a1");
    let reply = http(r.server.addr(), "POST", Some(&format!("Bearer {t}")), None, &rpc("ping", json!({}))).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["result"], json!({}));
}

#[tokio::test]
async fn the_server_listens_on_loopback_only() {
    let r = rig().await;
    assert!(r.server.addr().ip().is_loopback());
    assert!(r.server.url().starts_with("http://127.0.0.1:"));
}

#[tokio::test]
async fn missing_wrong_and_malformed_credentials_are_refused_without_running_anything() {
    let r = rig().await;
    let t = r.token("a1");
    let append = rpc("tools/call", json!({"name": "memory_save", "arguments": {"scope": "agent", "path": "x.md", "text": "nope"}}));
    for auth in [None, Some("Bearer wrong"), Some("Bearer "), Some("Bearer"), Some(&*format!("Basic {t}")), Some(&*t)] {
        let reply = http(r.server.addr(), "POST", auth, None, &append).await;
        assert_eq!(reply.status, 401, "{auth:?}");
    }
    assert!(r.store.read(&Scope::Agent(AgentId("a1".into())), "x.md").is_err(), "an unauthorized call must not write");
}

#[tokio::test]
async fn a_revoked_token_is_refused() {
    let r = rig().await;
    let t = r.token("a1");
    let auth = format!("Bearer {t}");
    assert_eq!(http(r.server.addr(), "POST", Some(&auth), None, &rpc("ping", json!({}))).await.status, 200);
    r.registry.revoke(&t);
    assert_eq!(http(r.server.addr(), "POST", Some(&auth), None, &rpc("ping", json!({}))).await.status, 401);
}

#[tokio::test]
async fn notifications_get_202_with_no_body() {
    let r = rig().await;
    let t = r.token("a1");
    let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string().into_bytes();
    let reply = http(r.server.addr(), "POST", Some(&format!("Bearer {t}")), None, &body).await;
    assert_eq!(reply.status, 202);
    assert!(reply.body.trim().is_empty());
}

#[tokio::test]
async fn bad_json_is_a_400_with_a_json_rpc_parse_error() {
    let r = rig().await;
    let t = r.token("a1");
    let reply = http(r.server.addr(), "POST", Some(&format!("Bearer {t}")), None, b"{not json").await;
    assert_eq!(reply.status, 400);
    assert_eq!(reply.json()["error"]["code"], -32700);
}

#[tokio::test]
async fn other_methods_are_refused() {
    let r = rig().await;
    let t = r.token("a1");
    let reply = http(r.server.addr(), "GET", Some(&format!("Bearer {t}")), None, b"").await;
    assert_eq!(reply.status, 405);
}

#[tokio::test]
async fn a_foreign_host_header_is_refused_to_block_dns_rebinding() {
    let r = rig().await;
    let t = r.token("a1");
    let auth = format!("Bearer {t}");
    let reply = http(r.server.addr(), "POST", Some(&auth), Some("evil.example.com"), &rpc("ping", json!({}))).await;
    assert_eq!(reply.status, 403);
    let port = r.server.addr().port();
    let ok = http(r.server.addr(), "POST", Some(&auth), Some(&format!("localhost:{port}")), &rpc("ping", json!({}))).await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let r = rig().await;
    let t = r.token("a1");
    let big = vec![b' '; discard_mcp::server::MAX_BODY_BYTES + 10];
    let reply = http(r.server.addr(), "POST", Some(&format!("Bearer {t}")), None, &big).await;
    assert_eq!(reply.status, 413);
}

#[tokio::test]
async fn sessions_for_different_agents_cannot_read_each_others_private_memory() {
    let r = rig().await;
    let (a, b) = (r.token("agent-a"), r.token("agent-b"));
    let write = rpc("tools/call", json!({"name": "memory_save", "arguments": {"scope": "agent", "path": "secret.md", "text": "a's secret"}}));
    assert_eq!(http(r.server.addr(), "POST", Some(&format!("Bearer {a}")), None, &write).await.status, 200);
    let item=r.store.list_items(&[Scope::Agent(AgentId("agent-a".into()))]).unwrap().remove(0);
    let read = rpc("tools/call", json!({"name": "memory_read", "arguments": {"scope": "agent", "path": item.path()}}));
    let as_b = http(r.server.addr(), "POST", Some(&format!("Bearer {b}")), None, &read).await.json();
    assert_eq!(as_b["result"]["isError"], true);
    let as_a = http(r.server.addr(), "POST", Some(&format!("Bearer {a}")), None, &read).await.json();
    assert_eq!(as_a["result"]["isError"], false);
    assert!(as_a["result"]["content"][0]["text"].as_str().unwrap().contains("a's secret"));
}

#[tokio::test]
async fn shutdown_stops_the_listener() {
    let r = rig().await;
    let addr = r.server.addr();
    r.server.shutdown().await;
    assert!(TcpStream::connect(addr).await.is_err());
}
