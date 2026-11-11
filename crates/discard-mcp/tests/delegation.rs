//! The delegation tools: `delegate`, `agents_list`, and `agent_request_add`, with a fake `Delegator`.

use discard_core::agent::AgentConfig;
use discard_core::capability::ApprovalPolicy;
use discard_core::delegation::{DelegateResult, DelegateTask, DelegationServices, Delegator, DelegatorSlot};
use discard_core::agent_requests::AgentRequests;
use discard_core::engine::{MemoryOptions, MemoryService};
use discard_core::memory::MemoryStore;
use discard_core::model::{AgentId, Block, Channel, ChannelId, Message, MessageId, Project, ProjectId, Role, ThreadState, WorkspaceMode};
use discard_core::store::Store;
use discard_mcp::identity::{Identity, ScopeKindOrd, Surface};
use discard_mcp::tools::McpHandler;
use discard_mcp::MemoryRuntime;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

type Call = (ChannelId, AgentId, Vec<DelegateTask>);

/// Records its calls and answers each task as done with a fixed text. With `rendezvous`, a call waits until that many
/// calls are inside `delegate` at once, so a test can show that calls are not serialized.
#[derive(Default)]
struct FakeDelegator {
    calls: Mutex<Vec<Call>>,
    inside: AtomicUsize,
    rendezvous: usize,
    fail: Option<String>,
}

impl Delegator for FakeDelegator {
    fn delegate(&self, parent: &ChannelId, caller: &AgentId, tasks: Vec<DelegateTask>) -> Result<Vec<DelegateResult>, String> {
        self.calls.lock().unwrap().push((parent.clone(), caller.clone(), tasks.clone()));
        if let Some(error) = &self.fail { return Err(error.clone()); }
        if self.rendezvous > 0 {
            self.inside.fetch_add(1, Ordering::SeqCst);
            let start = Instant::now();
            while self.inside.load(Ordering::SeqCst) < self.rendezvous {
                if start.elapsed() > Duration::from_secs(5) { return Err("the calls were serialized".into()); }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(tasks.iter().enumerate().map(|(i, t)| DelegateResult {
            thread_channel_id: ChannelId(format!("thread-{i}")), name: t.name.clone(),
            state: if t.brief.contains("fail") { ThreadState::Failed } else if t.brief.contains("slow") { ThreadState::TimedOut } else { ThreadState::Done },
            text: format!("answer to {}", t.brief), steer_note: None,
        }).collect())
    }
}

struct World {
    core: Arc<Mutex<Store>>,
    project: ProjectId,
    channel: ChannelId,
    caller: AgentId,
    helper: AgentId,
    outsider: AgentId,
    requests: Arc<AgentRequests>,
    slot: Arc<DelegatorSlot>,
    handler: Arc<McpHandler>,
    _dir: tempfile::TempDir,
}

fn config(name: &str, delegation: bool, prompt: &str) -> AgentConfig {
    AgentConfig { workspace_write: false, sandbox_autorun: false, delegation, reasoning_effort: None, name: name.into(), avatar: None, provider: "fake".into(), model: Some("test".into()), system_prompt: prompt.into(), policy: ApprovalPolicy::Ask, plugin_write: false }
}

fn services(core: &Arc<Mutex<Store>>) -> DelegationServices {
    DelegationServices { store: core.clone(), requests: Arc::default(), delegator: Arc::default() }
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let project = Project { id: ProjectId::new(), name: "demo".into(), path: "/tmp/demo".into() };
    store.insert_project(&project).unwrap();
    let caller = store.create_agent(&config("Main", true, "Lead.")).unwrap();
    store.set_project_main_agent(&project.id, &caller).unwrap();
    let helper = store.create_agent(&config("Helper", false, &"é".repeat(300))).unwrap();
    let outsider = store.create_agent(&config("Outsider", false, "Reviews.")).unwrap();
    let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: caller.clone(), workspace_mode: WorkspaceMode::MainCheckout };
    store.insert_channel(&channel).unwrap();
    store.invite_agent_to_channel(&project.id, &channel.id, &helper).unwrap();
    store.insert_message(&Message { id: MessageId::new(), channel_id: channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "go".into() }] }).unwrap();
    let core = Arc::new(Mutex::new(store));
    let services = services(&core);
    let (requests, slot) = (services.requests.clone(), services.delegator.clone());
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let handler = Arc::new(McpHandler::new(memory).with_delegation(services));
    World { core, project: project.id, channel: channel.id, caller, helper, outsider, requests, slot, handler, _dir: dir }
}

impl World {
    fn who(&self) -> Identity {
        Identity {
            main_checkout_requests: false, main_checkout_grant: false, workspace_root: None, agent: self.caller.clone(), project: Some(self.project.clone()),
            source_conversation: Some(self.channel.clone()), write_scopes: [ScopeKindOrd::AgentProject].into(), surface: Surface::Memory, scopes: [ScopeKindOrd::Agent].into(), can_write: true, delegation: true,
        }
    }
    fn call(&self, who: &Identity, tool: &str, args: Value) -> Value {
        self.handler.handle(who, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})).unwrap()
    }
    fn tools(&self, who: &Identity) -> Vec<String> {
        let reply = self.handler.handle(who, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).unwrap();
        reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect()
    }
    fn install(&self, delegator: FakeDelegator) -> Arc<FakeDelegator> { let d = Arc::new(delegator); self.slot.install(d.clone()); d }
}

fn text(reply: &Value) -> String { reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned() }
fn is_error(reply: &Value) -> bool { reply["result"]["isError"].as_bool().unwrap_or(false) }
fn ok(reply: &Value) -> Value { assert!(!is_error(reply), "{reply}"); serde_json::from_str(&text(reply)).unwrap() }
const DELEGATION: [&str; 3] = ["delegate", "agents_list", "agent_request_add"];

#[test]
fn the_tools_are_listed_only_when_the_identity_may_delegate() {
    let w = world();
    let listed = w.tools(&w.who());
    for tool in DELEGATION { assert!(listed.iter().any(|t| t == tool), "{tool} in {listed:?}"); }
    let mut off = w.who(); off.delegation = false;
    let mut summary = w.who(); summary.can_write = false;
    let mut plugins = w.who(); plugins.surface = Surface::Plugins;
    for (label, who) in [("setting off or a thread", off), ("summarization", summary)] {
        assert!(!w.tools(&who).iter().any(|t| DELEGATION.contains(&t.as_str())), "{label}");
    }
    assert!(w.tools(&plugins).is_empty());
    // Without the services in the handler nothing is listed, even for an identity that may delegate.
    let bare = McpHandler::new(Arc::new(MemoryStore::open_in_memory_index(w._dir.path()).unwrap()));
    let reply = bare.handle(&w.who(), &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).unwrap();
    assert!(!reply.to_string().contains("\"delegate\""));
}

#[test]
fn the_descriptions_tell_the_agent_how_to_brief_sub_agents() {
    let w = world();
    let reply = w.handler.handle(&w.who(), &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).unwrap();
    let tool = |name: &str| reply["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap().clone();
    let delegate = tool("delegate")["description"].as_str().unwrap().to_lowercase();
    for needle in ["read-only", "cannot see this conversation", "self-contained", "1 to 4", "at the same time", "agents_list", "agent_request_add", "in task order", "results"] {
        assert!(delegate.contains(needle), "{needle}: {delegate}");
    }
    assert_eq!(tool("delegate")["inputSchema"]["properties"]["tasks"]["maxItems"], 4);
    assert!(tool("agent_request_add")["description"].as_str().unwrap().contains("approvalRequired"));
}

#[test]
fn delegate_hands_the_resolved_tasks_to_the_delegator_and_returns_results_in_task_order() {
    let w = world();
    let delegator = w.install(FakeDelegator::default());
    let reply = w.call(&w.who(), "delegate", json!({"tasks": [
        {"title": "Find callers", "brief": "search the code"},
        {"title": "", "brief": "fail on purpose", "agent": " helper "},
        {"title": "Slow one", "brief": "slow work", "agent": "Main"},
    ]}));
    let value = ok(&reply);
    let rows = value["results"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| r["task"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!((rows[0]["thread"].as_str(), rows[0]["state"].as_str(), rows[0]["result"].as_str()), (Some("Find callers"), Some("done"), Some("answer to search the code")));
    assert_eq!((rows[1]["thread"].as_str(), rows[1]["state"].as_str()), (Some("fail on purpose"), Some("failed")), "an empty title uses the brief");
    assert!(rows[1]["error"].as_str().unwrap().starts_with("The sub-agent task failed."), "{}", rows[1]);
    assert!(rows[2]["error"].as_str().unwrap().contains("timed out"));
    let calls = delegator.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let (channel, caller, tasks) = &calls[0];
    assert_eq!((channel, caller), (&w.channel, &w.caller));
    assert_eq!(tasks.iter().map(|t| t.agent.clone()).collect::<Vec<_>>(), [w.caller.clone(), w.helper.clone(), w.caller.clone()], "no agent means the caller; names resolve to ids");
    assert_eq!(tasks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Find callers", "fail on purpose", "Slow one"]);
}

#[test]
fn an_invalid_task_fails_the_whole_call_before_any_sub_agent_starts() {
    let w = world();
    let delegator = w.install(FakeDelegator::default());
    let task = |brief: &str, agent: Option<&str>| match agent { Some(a) => json!({"title": "t", "brief": brief, "agent": a}), None => json!({"title": "t", "brief": brief}) };
    let cases: Vec<(Value, &str)> = vec![
        (json!({"tasks": []}), "1 to 4"),
        (json!({"tasks": [task("b", None), task("b", None), task("b", None), task("b", None), task("b", None)]}), "at most 4"),
        (json!({"tasks": [task("fine", None), task("  ", None)]}), "Task 2: the brief is empty"),
        (json!({"tasks": [task("fine", None), task("b", Some("Nobody"))]}), "agents_list"),
        (json!({"tasks": [task("fine", None), task("b", Some("Outsider"))]}), "agent_request_add"),
        (json!({"tasks": "nope"}), "`tasks` is required"),
        (json!({}), "`tasks` is required"),
        (json!({"tasks": [{"title": 3, "brief": "b"}]}), "must be a string"),
        (json!({"tasks": [{"brief": "b", "model": "x"}]}), "Task 1"),
        (json!({"tasks": [task("b", None)], "extra": 1}), "Only `tasks`"),
    ];
    for (args, needle) in cases {
        let reply = w.call(&w.who(), "delegate", args.clone());
        assert!(is_error(&reply) && text(&reply).contains(needle), "{args}: {}", text(&reply));
    }
    assert!(delegator.calls.lock().unwrap().is_empty(), "no task started");
}

#[test]
fn a_direct_message_allows_only_the_default_agent() {
    let w = world();
    let delegator = w.install(FakeDelegator::default());
    let direct = {
        let store = w.core.lock().unwrap();
        store.create_conversation(&w.caller, None, "chat").unwrap().id
    };
    let mut who = w.who(); who.project = None; who.source_conversation = Some(direct.clone());
    assert!(!is_error(&w.call(&who, "delegate", json!({"tasks": [{"title": "t", "brief": "b"}]}))));
    let reply = w.call(&who, "delegate", json!({"tasks": [{"title": "t", "brief": "b", "agent": "Helper"}]}));
    assert!(is_error(&reply) && text(&reply).contains("direct message"), "{}", text(&reply));
    let request = w.call(&who, "agent_request_add", json!({"name": "Helper", "reason": "why"}));
    assert!(is_error(&request) && text(&request).contains("direct message"), "agent_request_add errors in a direct message: {}", text(&request));
    assert_eq!(delegator.calls.lock().unwrap().len(), 1);
}

#[test]
fn delegate_fails_clearly_without_a_delegator_and_passes_its_errors_on() {
    let w = world();
    let args = json!({"tasks": [{"title": "t", "brief": "b"}]});
    let reply = w.call(&w.who(), "delegate", args.clone());
    assert!(is_error(&reply) && text(&reply).contains("not ready"), "{}", text(&reply));
    w.install(FakeDelegator { fail: Some("The app is closing.".into()), ..Default::default() });
    let reply = w.call(&w.who(), "delegate", args);
    assert!(is_error(&reply) && text(&reply) == "The app is closing.");
}

#[test]
fn a_session_that_may_not_delegate_cannot_call_the_tools() {
    let w = world();
    w.install(FakeDelegator::default());
    let mut off = w.who(); off.delegation = false;
    let mut summary = w.who(); summary.can_write = false;
    for who in [off, summary] {
        for tool in DELEGATION {
            let reply = w.call(&who, tool, json!({"tasks": [{"title": "t", "brief": "b"}], "name": "Helper", "reason": "r"}));
            assert!(reply["error"].is_object(), "{tool}: {reply}");
        }
    }
    // A thread agent has the setting but the identity says no, and the core check refuses a thread channel as well.
    let thread = { let store = w.core.lock().unwrap(); store.create_sub_agent_thread(&w.channel, "t", &w.helper).unwrap().id };
    let mut in_thread = w.who(); in_thread.source_conversation = Some(thread);
    let reply = w.call(&in_thread, "delegate", json!({"tasks": [{"title": "t", "brief": "b"}]}));
    assert!(is_error(&reply) && text(&reply).contains("cannot delegate"), "{}", text(&reply));
}

#[test]
fn agents_list_shows_name_provider_model_membership_and_a_short_personality() {
    let w = world();
    let value = ok(&w.call(&w.who(), "agents_list", json!({})));
    let agents = value["agents"].as_array().unwrap();
    let by = |name: &str| agents.iter().find(|a| a["name"] == name).unwrap().clone();
    assert_eq!((by("Main")["member"].as_bool(), by("Helper")["member"].as_bool(), by("Outsider")["member"].as_bool()), (Some(true), Some(true), Some(false)));
    assert_eq!((by("Helper")["provider"].as_str(), by("Helper")["model"].as_str()), (Some("fake"), Some("test")));
    assert_eq!(by("Helper")["personality"].as_str().unwrap().chars().count(), 200, "the first 200 characters, cut on a character boundary");
    assert_eq!(by("Outsider")["personality"], "Reviews.");
    assert!(is_error(&w.call(&w.who(), "agents_list", json!({"x": 1}))));
    w.core.lock().unwrap().set_agent_archived(&w.outsider, true).unwrap();
    assert!(!ok(&w.call(&w.who(), "agents_list", json!({})))["agents"].as_array().unwrap().iter().any(|a| a["name"] == "Outsider"));
}

#[test]
fn agent_request_add_returns_at_once_with_a_pending_request_the_app_can_decide() {
    let w = world();
    let reply = w.call(&w.who(), "agent_request_add", json!({"name": "outsider", "reason": "Needs a security review."}));
    let value = ok(&reply);
    assert_eq!(value["approvalRequired"], true);
    assert_eq!((value["request"]["agentName"].as_str(), value["request"]["state"].as_str(), value["request"]["reason"].as_str()), (Some("Outsider"), Some("pending"), Some("Needs a security review.")));
    assert!(value["message"].as_str().unwrap().contains("cannot approve"));
    let pending = w.requests.list(&w.channel);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, value["request"]["id"].as_str().unwrap());
    // The user declines: the same agent cannot be asked again until the next user message.
    let decision = w.requests.decide(&w.core.lock().unwrap(), &w.channel, &pending[0].id, false).unwrap();
    assert!(decision.follow_up.unwrap().declined);
    let again = w.call(&w.who(), "agent_request_add", json!({"name": "Outsider", "reason": "Please."}));
    assert!(is_error(&again) && text(&again).contains("declined"), "{}", text(&again));
    w.core.lock().unwrap().insert_message(&Message { id: MessageId::new(), channel_id: w.channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "ok try".into() }] }).unwrap();
    let id = ok(&w.call(&w.who(), "agent_request_add", json!({"name": "Outsider", "reason": "Please."})))["request"]["id"].as_str().unwrap().to_owned();
    // Approval adds the agent, and a later delegate call may name it.
    w.requests.decide(&w.core.lock().unwrap(), &w.channel, &id, true).unwrap();
    w.install(FakeDelegator::default());
    assert!(!is_error(&w.call(&w.who(), "delegate", json!({"tasks": [{"title": "t", "brief": "review", "agent": "Outsider"}]}))));
}

#[test]
fn agent_request_add_checks_its_arguments_and_the_agent() {
    let w = world();
    for (args, needle) in [
        (json!({"name": "Nobody", "reason": "r"}), "agents_list"),
        (json!({"name": "Helper", "reason": "r"}), "already a member"),
        (json!({"name": "Outsider", "reason": " "}), "reason"),
        (json!({"name": "Outsider"}), "`reason` is required"),
        (json!({"reason": "r"}), "`name` is required"),
        (json!({"name": "Outsider", "reason": "r", "x": 1}), "Only `name` and `reason`"),
    ] {
        let reply = w.call(&w.who(), "agent_request_add", args.clone());
        assert!(is_error(&reply) && text(&reply).contains(needle), "{args}: {}", text(&reply));
    }
    assert!(w.requests.list(&w.channel).is_empty());
}

#[test]
fn concurrent_delegate_calls_run_at_the_same_time_without_blocking_other_tools() {
    let w = Arc::new(world());
    let delegator = w.install(FakeDelegator { rendezvous: 2, ..Default::default() });
    let started = Instant::now();
    let calls: Vec<_> = (0..2).map(|i| { let w = w.clone(); std::thread::spawn(move || w.call(&w.who(), "delegate", json!({"tasks": [{"title": format!("t{i}"), "brief": format!("b{i}")}]}))) }).collect();
    // While both calls wait in the delegator, other tools and the store still answer.
    while delegator.inside.load(Ordering::SeqCst) == 0 && started.elapsed() < Duration::from_secs(5) { std::thread::sleep(Duration::from_millis(2)); }
    assert!(!is_error(&w.call(&w.who(), "agents_list", json!({}))));
    assert!(!is_error(&w.call(&w.who(), "memory_list", json!({"scope": "agent"}))), "memory tools answer while delegate calls wait");
    for call in calls { let reply = call.join().unwrap(); assert!(!is_error(&reply), "{}", text(&reply)); }
    assert!(started.elapsed() < Duration::from_secs(4), "the calls did not wait for each other");
    assert_eq!(delegator.calls.lock().unwrap().len(), 2);
}

// ---- The runtime: identity, config file, and HTTP ----

async fn runtime(w: &World, with_services: bool) -> Arc<MemoryRuntime> {
    let memory = Arc::new(MemoryStore::open_in_memory_index(&w._dir.path().join("rt-memory")).unwrap());
    let delegation = with_services.then(|| DelegationServices { store: w.core.clone(), requests: w.requests.clone(), delegator: w.slot.clone() });
    MemoryRuntime::start_with_delegation(memory, &w._dir.path().join("run"), None, None, None, delegation).await.unwrap()
}

fn options(delegation: bool) -> MemoryOptions<'static> { MemoryOptions { query: "q", workspace: None, main_checkout_requests: false, delegation } }

fn server_entry(path: &std::path::Path) -> Value {
    serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["mcpServers"]["discard-memory"].clone()
}

#[tokio::test]
async fn a_session_that_may_delegate_gets_the_identity_the_timeout_and_a_prompt_note() {
    let w = world();
    let rt = runtime(&w, true).await;
    let on = rt.prepare_response(&w.caller, Some(&w.project), &w.channel, Some("Base."), false, options(true)).unwrap();
    let identity = rt.registry().lookup(&on.token).unwrap();
    assert!(identity.delegation && identity.can_write);
    assert!(on.system_prompt.contains("Delegation: delegate starts read-only sub-agents"), "{}", on.system_prompt);
    let entry = server_entry(&on.mcp_config);
    assert_eq!(entry["timeout"], 660_000, "10 minutes plus a 60 second margin, in milliseconds");
    assert_eq!(entry["timeout"].as_u64().unwrap() as u128, discard_core::delegation::provider_tool_timeout().as_millis());
    let off = rt.prepare_response(&w.caller, Some(&w.project), &w.channel, Some("Base."), false, options(false)).unwrap();
    assert!(!rt.registry().lookup(&off.token).unwrap().delegation);
    assert!(server_entry(&off.mcp_config).get("timeout").is_none() && !off.system_prompt.contains("Delegation:"));
    let summary = rt.prepare_summary(&w.caller, Some(&w.project), &w.channel).unwrap();
    assert!(!rt.registry().lookup(&summary.token).unwrap().delegation, "never during summarization");
    assert!(server_entry(&summary.mcp_config).get("timeout").is_none());
    let plain = runtime(&w, false).await;
    let none = plain.prepare_response(&w.caller, Some(&w.project), &w.channel, None, false, options(true)).unwrap();
    assert!(!plain.registry().lookup(&none.token).unwrap().delegation, "a runtime without the services never delegates");
    assert!(server_entry(&none.mcp_config).get("timeout").is_none());
    for (rt, token) in [(&rt, &on.token), (&rt, &off.token), (&rt, &summary.token), (&plain, &none.token)] { rt.finish(token); }
    rt.shutdown().await; plain.shutdown().await;
}

async fn rpc(config: &Value, method: &str, params: Value) -> Value {
    let addr = config["url"].as_str().unwrap().trim_start_matches("http://").trim_end_matches("/mcp").to_owned();
    let auth = config["headers"]["Authorization"].as_str().unwrap().to_owned();
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string();
    let mut stream = TcpStream::connect(&addr).await.unwrap();
    stream.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut response = String::new(); stream.read_to_string(&mut response).await.unwrap();
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_server_answers_other_calls_while_two_delegate_calls_block() {
    let w = world();
    let rt = runtime(&w, true).await;
    let a = rt.prepare_response(&w.caller, Some(&w.project), &w.channel, None, false, options(true)).unwrap();
    let b = rt.prepare_response(&w.caller, Some(&w.project), &w.channel, None, false, options(true)).unwrap();
    let delegator = w.install(FakeDelegator { rendezvous: 2, ..Default::default() });
    let (ea, eb) = (server_entry(&a.mcp_config), server_entry(&b.mcp_config));
    let delegate = |entry: Value, n: u32| tokio::spawn(async move { rpc(&entry, "tools/call", json!({"name": "delegate", "arguments": {"tasks": [{"title": format!("t{n}"), "brief": format!("b{n}")}]}})).await });
    let (first, second) = (delegate(ea.clone(), 1), delegate(eb, 2));
    let start = Instant::now();
    while delegator.inside.load(Ordering::SeqCst) < 1 && start.elapsed() < Duration::from_secs(5) { tokio::time::sleep(Duration::from_millis(5)).await; }
    // A third call, on a third connection, is served while the first two are inside `delegate`.
    let listed = rpc(&ea, "tools/list", json!({})).await;
    assert!(listed["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "delegate"));
    for reply in [first.await.unwrap(), second.await.unwrap()] {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert_eq!(serde_json::from_str::<Value>(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()["results"][0]["state"], "done");
    }
    assert!(start.elapsed() < Duration::from_secs(4));
    rt.finish(&a.token); rt.finish(&b.token);
    rt.shutdown().await;
}
