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
    let server = &config["mcpServers"]["discard-memory"];
    let addr = server["url"].as_str().unwrap().trim_start_matches("http://").trim_end_matches("/mcp");
    let auth = server["headers"]["Authorization"].as_str().unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}).to_string();
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
    let agent = store.create_agent(&AgentConfig { workspace_write: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
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
    let agent = store.create_agent(&AgentConfig { workspace_write: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
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
