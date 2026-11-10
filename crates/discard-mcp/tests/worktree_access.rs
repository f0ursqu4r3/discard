use discard_core::{agent::AgentConfig, capability::ApprovalPolicy, engine::MemoryService, memory::MemoryStore, model::{Channel, ChannelId, WorkspaceMode}, store::{Store, WorktreeRecord}, worktree_access::WorktreeAccess};
use discard_mcp::MemoryRuntime;
use serde_json::{json, Value};
use std::{path::Path, sync::{Arc, Mutex}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream};
fn git(path: &Path, args: &[&str]) {
    let output = discard_core::process::spawn_std(std::process::Command::new("git").args(["-c","user.name=Test","-c","user.email=test@example.com","-c","commit.gpgsign=false","-c","core.hooksPath=/dev/null"]).args(args).current_dir(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
async fn call(config: &Value, name: &str) -> Value {
    call_args(config, name, json!({})).await
}
async fn call_args(config: &Value, name: &str, arguments: Value) -> Value {
    rpc(config, "tools/call", json!({"name":name,"arguments":arguments})).await
}
async fn rpc(config: &Value, method: &str, params: Value) -> Value {
    let server = &config["mcpServers"]["discard-memory"];
    let addr = server["url"].as_str().unwrap().trim_start_matches("http://").trim_end_matches("/mcp");
    let auth = server["headers"]["Authorization"].as_str().unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string();
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    let mut response = String::new(); stream.read_to_string(&mut response).await.unwrap();
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}
#[tokio::test]
async fn scoped_http_tool_requests_removal_but_only_app_approval_executes_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    git(&repo,&["init","-q","-b","main"]); git(&repo,&["commit","--allow-empty","-m","initial"]);
    let path = dir.path().join("linked"); git(&repo,&["worktree","add","--detach",path.to_str().unwrap()]);
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let channel = Channel {id:ChannelId::new(),project_id:Some(project.id.clone()),name:"linked".into(),agent_id:agent.clone(),workspace_mode:WorkspaceMode::Worktree};
    store.create_named_repository_channel(&channel,"",None,Some(&WorktreeRecord {channel_id:channel.id.clone(),path:path.to_string_lossy().into(),repo:repo.to_string_lossy().into(),branch:"detached".into(),base_commit:String::new(),owned:false})).unwrap();
    let access = Arc::new(WorktreeAccess::new(Arc::new(Mutex::new(store))));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start_with_services(memory,&dir.path().join("runtime"),None,Some(access.clone())).await.unwrap();
    let prepared = runtime.prepare_turn(&agent,Some(&project.id),&channel.id,None,false,"remove this worktree").unwrap();
    assert!(prepared.system_prompt.contains("worktree_request_removal"));
    let config:Value = serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
    assert_eq!(call(&config,"worktree_inspect").await["result"]["isError"],false);
    let reply = call(&config,"worktree_request_removal").await;
    assert_eq!(reply["result"]["isError"],false,"{reply}");
    assert!(path.exists());
    let pending=access.requests(&channel.id); assert_eq!(pending.len(),1); assert_eq!(pending[0].state,"pending");
    let summary=runtime.prepare_summary(&agent,Some(&project.id),&channel.id).unwrap();
    let summary_config:Value=serde_json::from_slice(&std::fs::read(&summary.mcp_config).unwrap()).unwrap();
    assert!(call(&summary_config,"worktree_request_removal").await["error"].is_object());
    for (name, args) in [("project_git_branches", json!({})), ("project_git_log", json!({})), ("project_git_compare", json!({"base":"HEAD"}))] {
        assert_eq!(call_args(&config, name, args.clone()).await["result"]["isError"], false);
        assert!(call_args(&summary_config, name, args).await["error"].is_object());
    }
    assert!(path.exists());
    let completed=access.decide(&channel.id,&pending[0].id,true).unwrap();
    assert_eq!(completed.state,"completed","{completed:?}"); assert!(!path.exists());
    let next=runtime.prepare_turn(&agent,Some(&project.id),&channel.id,None,false,"what happened?").unwrap();
    assert!(next.system_prompt.contains("Removed worktree"));
    for token in [&prepared.token,&summary.token,&next.token] {runtime.finish(token);}
    runtime.shutdown().await;
}

#[tokio::test]
async fn general_channel_runtime_exposes_project_files_without_worktree_tools() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("project"); std::fs::create_dir(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "Project overview").unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let channel = store.ensure_project_general_channel(&project.id, &agent).unwrap();
    let access = Arc::new(WorktreeAccess::new(Arc::new(Mutex::new(store))));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start_with_services(memory, &dir.path().join("runtime"), None, Some(access)).await.unwrap();
    let prepared = runtime.prepare_turn(&agent, Some(&project.id), &channel.id, None, false, "What is this project?").unwrap();
    assert!(prepared.system_prompt.contains("project_read_file"));
    assert!(!prepared.system_prompt.contains("Worktree access:"));
    let config: Value = serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
    let listing = call(&config, "project_list_files").await;
    assert_eq!(listing["result"]["isError"], false, "{listing}");
    assert!(listing.to_string().contains("README.md"));
    let summary = runtime.prepare_summary(&agent, Some(&project.id), &channel.id).unwrap();
    let summary_config: Value = serde_json::from_slice(&std::fs::read(&summary.mcp_config).unwrap()).unwrap();
    assert!(call(&summary_config, "project_list_files").await["error"].is_object());
    for token in [&prepared.token, &summary.token] { runtime.finish(token); }
    runtime.shutdown().await;
}

fn data(reply: &Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn project_git_and_search_options_use_scoped_http_tools() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("project"); std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "Project 12\nproject [abc]\n").unwrap();
    git(&repo, &["add", "."]); git(&repo, &["commit", "-qm", "initial"]);
    git(&repo, &["branch", "base"]);
    std::fs::write(repo.join("other.txt"), "new\n").unwrap();
    git(&repo, &["add", "."]); git(&repo, &["commit", "-qm", "second"]);
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let channel = store.ensure_project_general_channel(&project.id, &agent).unwrap();
    let access = Arc::new(WorktreeAccess::new(Arc::new(Mutex::new(store))));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start_with_services(memory, &dir.path().join("runtime"), None, Some(access)).await.unwrap();
    let prepared = runtime.prepare_turn(&agent, Some(&project.id), &channel.id, None, false, "Inspect branches").unwrap();
    let config: Value = serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
    let listed = rpc(&config, "tools/list", json!({})).await;
    for name in ["project_git_branches", "project_git_log", "project_git_compare"] {
        let tool = listed["result"]["tools"].as_array().unwrap().iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert!(prepared.system_prompt.contains(name));
    }
    let branches = data(&call(&config, "project_git_branches").await);
    assert_eq!(branches["default_branch"], "main");
    assert_eq!(branches["branches"].as_array().unwrap().len(), 2);
    let log = data(&call_args(&config, "project_git_log", json!({"limit":1})).await);
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    assert_eq!(log["commits"][0]["subject"], "second");
    let log = data(&call_args(&config, "project_git_log", json!({"path":"README.md"})).await);
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    assert_eq!(log["commits"][0]["subject"], "initial");
    let compare = data(&call_args(&config, "project_git_compare", json!({"base":"base"})).await);
    assert_eq!(compare["ahead"], 1);
    assert_eq!(compare["files"][0]["path"], "other.txt");
    let compare = data(&call_args(&config, "project_git_compare", json!({"base":"base","view":"diff"})).await);
    assert!(compare["diff"].as_str().unwrap().contains("+new"));
    for (args, count) in [
        (json!({"query":"project","ignore_case":true}), 2),
        (json!({"query":"Project [0-9]+","regex":true}), 1),
        (json!({"query":"project [0-9]+","regex":true,"ignore_case":true}), 1),
    ] {
        assert_eq!(data(&call_args(&config, "project_search", args).await)["matches"].as_array().unwrap().len(), count);
    }
    for (name, args) in [
        ("project_git_branches", json!({"path":"/tmp"})),
        ("project_git_log", json!({"command":"status"})),
        ("project_git_log", json!({"ref":"--help"})),
        ("project_git_log", json!({"ref":"HEAD\n"})),
        ("project_git_log", json!({"ref":"missing"})),
        ("project_git_log", json!({"ref":3})),
        ("project_git_log", json!({"path":"../outside"})),
        ("project_git_log", json!({"limit":0})),
        ("project_git_log", json!({"limit":31})),
        ("project_git_log", json!({"limit":1.5})),
        ("project_git_compare", json!({})),
        ("project_git_compare", json!({"base":"main","head":"--help"})),
        ("project_git_compare", json!({"base":"main","view":"invalid"})),
        ("project_git_compare", json!({"base":"main","unknown":true})),
        ("project_search", json!({"query":"[","regex":true})),
        ("project_search", json!({"query":"Project","ignore_case":"true"})),
        ("project_search", json!({"query":"Project","regex":1})),
    ] {
        let reply = call_args(&config, name, args).await;
        assert_eq!(reply["result"]["isError"], true, "{name}: {reply}");
        assert!(!reply.to_string().contains(dir.path().to_str().unwrap()), "{reply}");
    }
    runtime.finish(&prepared.token);
    runtime.shutdown().await;
}
