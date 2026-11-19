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
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
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
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
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
    let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, name:"Agent".into(),provider:"fake".into(),model:Some("test".into()),avatar:None,reasoning_effort:None,system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false }).unwrap();
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

fn failed(reply: &Value) -> bool { reply["error"].is_object() || reply["result"]["isError"] == true }

fn tool_names(listed: &Value) -> Vec<String> {
    listed["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn a_thread_of_a_worktree_channel_reads_the_parents_worktree_and_gets_no_write_or_worktree_tools() {
    use discard_core::{model::{Block, Message, MessageId, Role}, workspace::GitWorkspace, write_requests::WriteRequests};
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "main checkout\n").unwrap();
    git(&repo, &["add", "."]); git(&repo, &["commit", "-qm", "initial"]);
    std::fs::write(repo.join("ONLY_MAIN.md"), "only in the main checkout\n").unwrap();
    git(&repo, &["branch", "no-worktree"]);
    let linked = dir.path().join("linked");
    git(&repo, &["worktree", "add", "-q", "-b", "feature", linked.to_str().unwrap()]);
    std::fs::write(linked.join("NOTE.md"), "uncommitted in the worktree\n").unwrap();
    let store = Store::open_in_memory().unwrap();
    let agent = store.create_agent(&AgentConfig { workspace_write: true, sandbox_autorun: false, native_resume: false, delegation: false, name: "Agent".into(), provider: "fake".into(), model: Some("test".into()), avatar: None, reasoning_effort: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    let bound = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "feature".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_repository_channel(&bound, "", Some("feature"), Some(&WorktreeRecord { channel_id: bound.id.clone(), path: linked.to_string_lossy().into(), repo: repo.to_string_lossy().into(), branch: "feature".into(), base_commit: String::new(), owned: false })).unwrap();
    let unbound = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "no-worktree".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.create_named_repository_channel(&unbound, "", Some("no-worktree"), None).unwrap();
    let say = |channel: &ChannelId, role| { let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: "x".into() }] }; store.insert_message(&message).unwrap(); message.id };
    say(&bound.id, Role::User); let answer = say(&bound.id, Role::Agent); say(&unbound.id, Role::User);
    let sub_agent = store.create_sub_agent_thread(&bound.id, "research", &agent).unwrap();
    let question = store.create_thread(&bound.id, &answer, "why?", &agent).unwrap();
    let unbound_thread = store.create_sub_agent_thread(&unbound.id, "research", &agent).unwrap();
    let store = Arc::new(Mutex::new(store));
    let access = Arc::new(WorktreeAccess::new(store.clone()));
    let workspace = Arc::new(GitWorkspace::new(&dir.path().join("worktrees"), store.clone()));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start_with_write_services(memory, &dir.path().join("runtime"), None, Some(access), Some((Arc::new(WriteRequests::default()), workspace))).await.unwrap();

    // Control: the bound channel itself has the worktree tools. The unbound branch channel may ask for a worktree.
    let own = runtime.prepare_turn(&agent, Some(&project.id), &bound.id, None, false, "q").unwrap();
    let own_config: Value = serde_json::from_slice(&std::fs::read(&own.mcp_config).unwrap()).unwrap();
    let names = tool_names(&rpc(&own_config, "tools/list", json!({})).await);
    assert!(names.contains(&"worktree_inspect".into()) && names.contains(&"worktree_request_removal".into()), "{names:?}");
    let unbound_turn = runtime.prepare_turn(&agent, Some(&project.id), &unbound.id, None, false, "q").unwrap();
    let unbound_config: Value = serde_json::from_slice(&std::fs::read(&unbound_turn.mcp_config).unwrap()).unwrap();
    assert!(tool_names(&rpc(&unbound_config, "tools/list", json!({})).await).contains(&"worktree_request_creation".into()), "the control can ask for a worktree");

    for thread in [&sub_agent.id, &question.id] {
        let prepared = runtime.prepare_turn(&agent, Some(&project.id), thread, None, false, "research").unwrap();
        let config: Value = serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
        let names = tool_names(&rpc(&config, "tools/list", json!({})).await);
        for read in ["project_list_files", "project_read_file", "project_search", "project_git_branches", "project_git_log", "project_git_compare"] {
            assert!(names.contains(&read.to_owned()), "{read} in {names:?}");
        }
        for write in ["worktree_inspect", "worktree_request_removal", "worktree_commit", "worktree_request_creation", "main_checkout_request_write"] {
            assert!(!names.contains(&write.to_owned()), "{write} must stay unavailable to a thread: {names:?}");
            assert!(failed(&call_args(&config, write, json!({})).await), "{write} cannot be called either");
        }
        assert!(!prepared.system_prompt.contains("Worktree access:") && !prepared.system_prompt.contains("worktree_request_creation"), "{}", prepared.system_prompt);
        // The project tools read the parent's worktree, with its uncommitted file, and not the main checkout.
        let listing = data(&call(&config, "project_list_files").await).to_string();
        assert!(listing.contains("NOTE.md") && !listing.contains("ONLY_MAIN.md"), "{listing}");
        let note = data(&call_args(&config, "project_read_file", json!({"path": "NOTE.md"})).await);
        assert_eq!(note["text"], "uncommitted in the worktree\n");
        assert!(failed(&call_args(&config, "project_read_file", json!({"path": "ONLY_MAIN.md"})).await), "the main checkout is not read");
        let found = data(&call_args(&config, "project_search", json!({"query": "uncommitted"})).await);
        assert_eq!(found["matches"][0]["path"], "NOTE.md");
        let branches = data(&call(&config, "project_git_branches").await).to_string();
        assert!(branches.contains("feature"), "{branches}");
        runtime.finish(&prepared.token);
    }

    // A thread of a channel with no worktree reads the project folder, as that channel does.
    let prepared = runtime.prepare_turn(&agent, Some(&project.id), &unbound_thread.id, None, false, "research").unwrap();
    let config: Value = serde_json::from_slice(&std::fs::read(&prepared.mcp_config).unwrap()).unwrap();
    let names = tool_names(&rpc(&config, "tools/list", json!({})).await);
    assert!(names.contains(&"project_read_file".into()) && !names.contains(&"worktree_request_creation".into()) && !names.contains(&"worktree_inspect".into()), "{names:?}");
    assert_eq!(data(&call_args(&config, "project_read_file", json!({"path": "ONLY_MAIN.md"})).await)["text"], "only in the main checkout\n");
    for token in [&own.token, &unbound_turn.token, &prepared.token] { runtime.finish(token); }
    runtime.shutdown().await;
}

#[tokio::test]
async fn moderator_prompt_names_the_move_tool() {
    use discard_core::{workspace::GitWorkspace, write_requests::WriteRequests};
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]); git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
    let store = Store::open_in_memory().unwrap();
    let config = |name: &str| AgentConfig { workspace_write: true, sandbox_autorun: false, native_resume: false, delegation: false, name: name.into(), provider: "fake".into(), model: Some("test".into()), avatar: None, reasoning_effort: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false };
    let main = store.create_agent(&config("Main")).unwrap();
    let other = store.create_agent(&config("Other")).unwrap();
    let project = store.import_project_folder(&repo).unwrap();
    store.set_project_main_agent(&project.id, &main).unwrap();
    let channel = store.ensure_project_general_channel(&project.id, &main).unwrap();
    let direct = Channel { id: ChannelId::new(), project_id: None, name: "Main".into(), agent_id: main.clone(), workspace_mode: WorkspaceMode::Worktree };
    store.insert_channel(&direct).unwrap();
    let store = Arc::new(Mutex::new(store));
    let access = Arc::new(WorktreeAccess::new(store.clone()));
    let workspace = Arc::new(GitWorkspace::new(&dir.path().join("worktrees"), store.clone()));
    let memory = Arc::new(MemoryStore::open_in_memory_index(&dir.path().join("memory")).unwrap());
    let runtime = MemoryRuntime::start_with_write_services(memory, &dir.path().join("runtime"), None, Some(access), Some((Arc::new(WriteRequests::default()), workspace))).await.unwrap();
    let line = "\nYou are this project's Primary / Moderator agent. When the user asks to move this conversation to another channel or to a new branch, call conversation_request_move with the target and a handoff. Do not save the handoff to memory instead. Other agents cannot move a conversation.\n";
    let moderator = runtime.prepare_turn(&main, Some(&project.id), &channel.id, None, false, "move this").unwrap();
    assert!(moderator.system_prompt.contains(line), "{}", moderator.system_prompt);
    let member = runtime.prepare_turn(&other, Some(&project.id), &channel.id, None, false, "move this").unwrap();
    assert!(!member.system_prompt.contains("conversation_request_move"), "{}", member.system_prompt);
    let chat = runtime.prepare_turn(&main, None, &direct.id, None, false, "move this").unwrap();
    assert!(!chat.system_prompt.contains("conversation_request_move"), "{}", chat.system_prompt);
    for token in [&moderator.token, &member.token, &chat.token] { runtime.finish(token); }
    runtime.shutdown().await;
}
