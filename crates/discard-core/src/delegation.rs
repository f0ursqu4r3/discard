//! Delegation: an agent starts read-only sub-agents in threads through the Discard `delegate` tool.
//!
//! Everything here needs no app state. The app implements `Delegator`; the MCP crate validates a call with `plan`,
//! hands the tasks to the `Delegator` it finds in the `DelegatorSlot`, and renders the answer with `format_results`.

use crate::model::{AgentId, Block, ChannelId, ProjectId, ThreadState};
use crate::store::{Store, sub_agent_thread_name};
use serde_json::json;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// A `delegate` call carries 1 to this many tasks. They run at the same time.
pub const MAX_TASKS: usize = 4;
/// A sub-agent turn stops after this long, and its task returns "timed out".
pub const SUB_AGENT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Added to `SUB_AGENT_TIMEOUT` for the provider's own tool-call timeout, so Discard reports the timeout first.
pub const PROVIDER_TIMEOUT_MARGIN: Duration = Duration::from_secs(60);
/// Characters of an agent's personality prompt that `agents_list` shows.
pub const PERSONALITY_PREVIEW_CHARS: usize = 200;
/// The brief becomes a message and the sub-agent's request, so it has a size limit.
pub const MAX_BRIEF_BYTES: usize = 32 * 1024;

/// The tool-call timeout to give the provider for the Discard MCP server.
pub const fn provider_tool_timeout() -> Duration {
    Duration::from_secs(SUB_AGENT_TIMEOUT.as_secs() + PROVIDER_TIMEOUT_MARGIN.as_secs())
}

/// One task as the tool call states it. `agent` is a name, not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInput {
    pub title: String,
    pub brief: String,
    pub agent: Option<String>,
}

/// One validated task. The sub-agent is resolved: the calling agent when the call named none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateTask {
    /// The thread name: the title cut to about 40 characters, or the start of the brief.
    pub name: String,
    pub brief: String,
    pub agent: AgentId,
}

/// How one task ended. `text` is the sub-agent's final message when the state is `Done`, and a detail for the
/// agent otherwise. `steer_note` is `steer_note(..)` for a task whose sub-agent got messages from the user. The result
/// for the agent starts with it, in every state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateResult {
    pub thread_channel_id: ChannelId,
    pub name: String,
    pub state: ThreadState,
    pub text: String,
    pub steer_note: Option<String>,
}

/// The note is cut to this many characters of the latest user message.
pub const MAX_STEER_NOTE_CHARS: usize = 500;

/// The note that tells the parent agent the user steered a sub-agent: `count` is the number of user messages saved in
/// the thread during the task, `latest` the text of the last one. `None` when the user sent none. The text is cut to
/// `MAX_STEER_NOTE_CHARS` characters (with `…`), and its `"` become `'` so the quotes of the note stay clear.
pub fn steer_note(count: usize, latest: &str) -> Option<String> {
    if count == 0 { return None; }
    let mut quoted: String = latest.chars().take(MAX_STEER_NOTE_CHARS).collect();
    if latest.chars().nth(MAX_STEER_NOTE_CHARS).is_some() { quoted.push('…'); }
    let quoted = quoted.replace('"', "'");
    let messages = if count == 1 { "1 message".to_owned() } else { format!("{count} messages") };
    Some(format!("Note: the user sent this sub-agent {messages} during the task. The result follows the user's instructions, which can differ from your brief. Latest user message: \"{quoted}\""))
}

/// Runs the tasks of one `delegate` call. The app implements it.
///
/// `delegate` blocks until every task has a final state. It is called from a worker thread, never from an async
/// task, and it must not hold a lock that other calls need. The tasks are valid (`plan`). For each task it must:
/// 1. create the thread with `Store::create_sub_agent_thread(parent, &task.name, &task.agent)` and start its session
///    (the engine saves the brief and sets the state),
/// 2. send exactly one `Command::SendBrief { text: task.brief, author }` to the thread's engine, where `author` is the
///    calling agent as a `MessageAuthor`,
/// 3. wait for the thread's `Update::SubAgentFinished { state, text }`; `state` is the final state in storage and
///    `text` the final message. Keep the `Update::Error` and `Update::SetupNeeded` texts of the turn as the detail
///    for a task that is not `Done`,
/// 4. stop the turn after `SUB_AGENT_TIMEOUT`: call `Store::finish_sub_agent_thread(thread, TimedOut)` first, then send
///    `Command::Cancel` (the first final state wins, so the engine's `Cancelled` does not replace it),
/// 5. stop every running task, as `Cancelled`, when the parent turn is stopped or the call is dropped. Stopping one
///    sub-agent from its thread ends only that task,
/// 6. return one `DelegateResult` for each task, in task order.
///
/// The parent turn is blocked in this call, so its response does not exist yet: the engine anchors the threads when
/// the parent turn ends and sends `Update::ThreadsAnchored` on the parent channel.
pub trait Delegator: Send + Sync {
    fn delegate(&self, parent_channel: &ChannelId, calling_agent: &AgentId, tasks: Vec<DelegateTask>) -> Result<Vec<DelegateResult>, String>;
}

/// Holds the `Delegator` once the app has one. The MCP server starts before the app's channels exist, so the slot
/// starts empty and the app fills it later. `get` clones the handle and releases the lock, so a long `delegate`
/// call never blocks another call or `install`.
#[derive(Default)]
pub struct DelegatorSlot(RwLock<Option<Arc<dyn Delegator>>>);

impl DelegatorSlot {
    pub fn install(&self, delegator: Arc<dyn Delegator>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(delegator);
    }
    pub fn get(&self) -> Option<Arc<dyn Delegator>> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// What the MCP server needs to serve the delegation tools. Without it, the tools are not listed.
#[derive(Clone)]
pub struct DelegationServices {
    pub store: Arc<Mutex<Store>>,
    pub requests: Arc<crate::agent_requests::AgentRequests>,
    pub delegator: Arc<DelegatorSlot>,
}

/// True when `agent` is a member of `project`: a project agent, or its main agent.
pub fn is_member(store: &Store, project: &ProjectId, agent: &AgentId) -> Result<bool, String> {
    let main = store.project_main_agent(project).map_err(|e| e.to_string())?;
    Ok(main.as_ref() == Some(agent) || store.project_agents(project).map_err(|e| e.to_string())?.contains(agent))
}

fn named<'a>(agents: &'a [(AgentId, crate::agent::AgentConfig)], name: &str) -> Result<Option<&'a (AgentId, crate::agent::AgentConfig)>, String> {
    let mut found = agents.iter().filter(|(_, config)| config.name.to_lowercase() == name.trim().to_lowercase());
    let first = found.next();
    if found.next().is_some() { return Err(format!("More than one active agent is named {name}. Ask the user to give them distinct names.")); }
    Ok(first)
}

/// Check a whole `delegate` call before anything starts, and resolve its agents. One bad task fails the call.
///
/// The calling agent needs `delegation`, and a thread cannot delegate. There are 1 to `MAX_TASKS` tasks, each with a
/// non-empty brief. A named agent must be active with a model. In a project channel it must be a project member, and
/// the error for a non-member names `agent_request_add`. In a direct message only the calling agent is permitted.
pub fn plan(store: &Store, channel: &ChannelId, caller: &AgentId, tasks: &[TaskInput]) -> Result<Vec<DelegateTask>, String> {
    let channel = store.load_channel(channel).map_err(|e| e.to_string())?;
    if store.thread_info(&channel.id).map_err(|e| e.to_string())?.is_some() { return Err("A sub-agent cannot delegate. Do the work yourself and return the result.".into()); }
    let config = store.load_agent(caller).map_err(|e| e.to_string())?;
    if !config.delegation { return Err("Delegation is off for this agent. The user turns it on in the agent settings.".into()); }
    if tasks.is_empty() { return Err("Give 1 to 4 tasks.".into()); }
    if tasks.len() > MAX_TASKS { return Err(format!("A call takes at most {MAX_TASKS} tasks. Split the work into several calls.")); }
    let agents = store.list_agents().map_err(|e| e.to_string())?;
    let mut planned = Vec::new();
    for (index, task) in tasks.iter().enumerate() {
        let number = index + 1;
        if task.brief.trim().is_empty() { return Err(format!("Task {number}: the brief is empty. Write a self-contained brief.")); }
        if task.brief.len() > MAX_BRIEF_BYTES { return Err(format!("Task {number}: the brief is longer than {} KB. Shorten it.", MAX_BRIEF_BYTES / 1024)); }
        let agent = match task.agent.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            None => caller.clone(),
            Some(name) => {
                let Some((id, found)) = named(&agents, name)? else {
                    let known = store.all_agents().map_err(|e| e.to_string())?.iter().any(|(_, c, archived)| *archived && c.name.to_lowercase() == name.to_lowercase());
                    return Err(if known { format!("Task {number}: the agent {name} is archived. Choose another agent from agents_list.") } else { format!("Task {number}: no active agent is named {name}. Call agents_list to see the agents.") });
                };
                if *id != *caller {
                    match &channel.project_id {
                        None => return Err(format!("Task {number}: a direct message can start sub-agents only with your own configuration. Leave out `agent`.")),
                        Some(project) if !is_member(store, project, id)? => return Err(format!("Task {number}: {} is not a member of this project. Call agent_request_add to ask the user to add it. No sub-agent started.", found.name)),
                        Some(_) => {}
                    }
                }
                id.clone()
            }
        };
        let model = if agent == *caller { config.model.clone() } else { store.load_agent(&agent).map_err(|e| e.to_string())?.model };
        if model.is_none_or(|m| m.trim().is_empty()) { return Err(format!("Task {number}: that agent has no model selected. Choose another agent.")); }
        planned.push(DelegateTask { name: sub_agent_thread_name(&task.title, &task.brief), brief: task.brief.clone(), agent });
    }
    Ok(planned)
}

/// One task's result is cut at this size. The full text stays in the thread.
pub const MAX_RESULT_BYTES: usize = 32 * 1024;

/// Cut `text` to `MAX_RESULT_BYTES` on a character boundary and say where the full text is.
fn cap_result(text: String) -> String {
    if text.len() <= MAX_RESULT_BYTES { return text; }
    let mut end = MAX_RESULT_BYTES;
    while !text.is_char_boundary(end) { end -= 1; }
    format!("{}\n\n[This result was cut at {} KB. The full text is in the sub-agent thread.]", text[..end].trim_end(), MAX_RESULT_BYTES / 1024)
}

/// The text of the final saved agent message of a sub-agent: all its text blocks in order, joined with blank lines.
/// Tool calls are left out, so findings written before a tool call (such as a memory save) are kept. Code and diff
/// blocks are part of the text. A result longer than `MAX_RESULT_BYTES` is cut.
pub fn final_text(blocks: &[Block]) -> String {
    let text = blocks.iter().filter_map(|b| match b {
        Block::Markdown { text } | Block::Code { text, .. } | Block::Diff { text } => Some(text.trim_matches('\n')),
        _ => None,
    }).collect::<Vec<_>>().join("\n\n");
    cap_result(text.trim().to_owned())
}

/// The text of a task that did not finish. It states which way it ended.
pub fn error_text(state: ThreadState, detail: &str) -> String {
    let head = match state {
        ThreadState::Failed => "The sub-agent task failed.".to_owned(),
        ThreadState::Cancelled => "The sub-agent task was cancelled.".to_owned(),
        ThreadState::TimedOut => format!("The sub-agent task timed out after {} minutes.", SUB_AGENT_TIMEOUT.as_secs() / 60),
        ThreadState::Running => "The sub-agent task is still running.".to_owned(),
        ThreadState::Done => return if detail.trim().is_empty() { "The sub-agent finished without a text reply.".to_owned() } else { detail.to_owned() },
    };
    if detail.trim().is_empty() { head } else { format!("{head} {}", detail.trim()) }
}

/// The tool result for the agent: one entry for each task, in task order. A finished task has `result`; any other
/// has `error`, which states failed, cancelled, or timed out.
pub fn format_results(results: &[DelegateResult]) -> String {
    let entries = results.iter().enumerate().map(|(index, r)| {
        let mut entry = json!({"task": index + 1, "thread": r.name, "state": r.state.as_str()});
        let body = error_text(r.state, &r.text);
        let body = match &r.steer_note { Some(note) => format!("{note}\n\n{body}"), None => body };
        entry[if r.state == ThreadState::Done { "result" } else { "error" }] = json!(body);
        entry
    }).collect::<Vec<_>>();
    json!({"results": entries}).to_string()
}

/// One active agent as `agents_list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSummary {
    pub name: String,
    pub provider: String,
    pub model: Option<String>,
    pub member: bool,
    pub personality: String,
}

/// The first `PERSONALITY_PREVIEW_CHARS` characters of a personality prompt, cut on a character boundary.
pub fn personality_preview(prompt: &str) -> String {
    prompt.chars().take(PERSONALITY_PREVIEW_CHARS).collect()
}

/// Every active agent with its project membership. Outside a project every agent counts as not a member, and the
/// calling agent is always listed as a member of its own conversation.
pub fn agent_summaries(store: &Store, project: Option<&ProjectId>, caller: &AgentId) -> Result<Vec<AgentSummary>, String> {
    store.list_agents().map_err(|e| e.to_string())?.into_iter().map(|(id, config)| {
        let member = id == *caller || match project { Some(project) => is_member(store, project, &id)?, None => false };
        Ok(AgentSummary { name: config.name, provider: config.provider, model: config.model, member, personality: personality_preview(&config.system_prompt) })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::capability::ApprovalPolicy;
    use crate::model::{Channel, Message, MessageId, Project, Role};

    struct World { store: Store, project: ProjectId, channel: Channel, caller: AgentId }

    fn config(name: &str, delegation: bool, model: Option<&str>) -> AgentConfig {
        AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation, reasoning_effort: None, name: name.into(), avatar: None, provider: "fake".into(), model: model.map(Into::into), system_prompt: format!("{name} personality"), policy: ApprovalPolicy::Ask, plugin_write: false }
    }

    fn world() -> World {
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "demo".into(), path: "/tmp/demo".into() };
        store.insert_project(&project).unwrap();
        let caller = store.create_agent(&config("Main", true, Some("m"))).unwrap();
        store.set_project_main_agent(&project.id, &caller).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: caller.clone(), workspace_mode: crate::model::WorkspaceMode::MainCheckout };
        store.insert_channel(&channel).unwrap();
        World { store, project: project.id, channel, caller }
    }

    fn task(title: &str, brief: &str, agent: Option<&str>) -> TaskInput { TaskInput { title: title.into(), brief: brief.into(), agent: agent.map(Into::into) } }

    #[test]
    fn the_provider_timeout_is_the_limit_plus_a_margin() {
        assert_eq!(SUB_AGENT_TIMEOUT, Duration::from_secs(600));
        assert_eq!(provider_tool_timeout(), Duration::from_secs(660));
        assert!(provider_tool_timeout() > SUB_AGENT_TIMEOUT);
    }

    #[test]
    fn a_valid_call_resolves_the_default_and_named_agents_and_names_the_threads() {
        let w = world();
        let helper = w.store.create_agent(&config("Helper", false, Some("m"))).unwrap();
        w.store.invite_agent_to_channel(&w.project, &w.channel.id, &helper).unwrap();
        let tasks = plan(&w.store, &w.channel.id, &w.caller, &[task("Find callers", "Search for callers of foo.", None), task("", "Read the build scripts and list the steps in order please", Some(" helper "))]).unwrap();
        assert_eq!(tasks, vec![
            DelegateTask { name: "Find callers".into(), brief: "Search for callers of foo.".into(), agent: w.caller.clone() },
            DelegateTask { name: "Read the build scripts and list the step".into(), brief: "Read the build scripts and list the steps in order please".into(), agent: helper },
        ]);
        plan(&w.store, &w.channel.id, &w.caller, &[task("t", "b", Some("main"))]).unwrap();
    }

    #[test]
    fn the_task_count_and_briefs_are_checked() {
        let w = world();
        let run = |tasks: Vec<TaskInput>| plan(&w.store, &w.channel.id, &w.caller, &tasks).unwrap_err();
        assert!(run(vec![]).contains("1 to 4"));
        assert!(run((0..5).map(|_| task("t", "b", None)).collect()).contains("at most 4"));
        assert!(plan(&w.store, &w.channel.id, &w.caller, &(0..4).map(|_| task("t", "b", None)).collect::<Vec<_>>()).is_ok());
        assert!(run(vec![task("t", "ok", None), task("t", "  \n ", None)]).starts_with("Task 2: the brief is empty"));
        assert!(run(vec![task("t", &"x".repeat(MAX_BRIEF_BYTES + 1), None)]).contains("Shorten"));
    }

    #[test]
    fn one_invalid_agent_fails_the_whole_call_and_a_non_member_error_names_agent_request_add() {
        let w = world();
        let outsider = w.store.create_agent(&config("Outsider", false, Some("m"))).unwrap();
        let archived = w.store.create_agent(&config("Gone", false, Some("m"))).unwrap();
        w.store.invite_agent_to_channel(&w.project, &w.channel.id, &archived).unwrap();
        w.store.set_agent_archived(&archived, true).unwrap();
        let no_model = AgentId::new();
        w.store.insert_agent(&no_model, &config("Blank", false, None)).unwrap();
        w.store.invite_agent_to_channel(&w.project, &w.channel.id, &no_model).unwrap();
        let err = |name: &str| plan(&w.store, &w.channel.id, &w.caller, &[task("fine", "b", None), task("bad", "b", Some(name))]).unwrap_err();
        let not_member = err("Outsider");
        assert!(not_member.starts_with("Task 2:") && not_member.contains("not a member") && not_member.contains("agent_request_add"), "{not_member}");
        assert!(err("Nobody").contains("agents_list"));
        assert!(err("Gone").contains("archived"));
        assert!(err("Blank").contains("no model"));
        let _ = outsider;
    }

    #[test]
    fn a_direct_message_allows_only_the_calling_agent() {
        let w = world();
        let other = w.store.create_agent(&config("Other", false, Some("m"))).unwrap();
        let direct = w.store.create_conversation(&w.caller, None, "chat").unwrap();
        assert_eq!(plan(&w.store, &direct.id, &w.caller, &[task("t", "b", None)]).unwrap()[0].agent, w.caller);
        assert_eq!(plan(&w.store, &direct.id, &w.caller, &[task("t", "b", Some("Main"))]).unwrap()[0].agent, w.caller, "naming the default is the default");
        let err = plan(&w.store, &direct.id, &w.caller, &[task("t", "b", Some("Other"))]).unwrap_err();
        assert!(err.contains("direct message") && err.contains("your own configuration"), "{err}");
        let _ = other;
    }

    #[test]
    fn the_caller_needs_delegation_and_must_not_be_in_a_thread() {
        let w = world();
        let plain = w.store.create_agent(&config("Plain", false, Some("m"))).unwrap();
        assert!(plan(&w.store, &w.channel.id, &plain, &[task("t", "b", None)]).unwrap_err().contains("Delegation is off"));
        let user = Message { id: MessageId::new(), channel_id: w.channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "go".into() }] };
        w.store.insert_message(&user).unwrap();
        let thread = w.store.create_sub_agent_thread(&w.channel.id, "t", &w.caller).unwrap();
        assert!(plan(&w.store, &thread.id, &w.caller, &[task("t", "b", None)]).unwrap_err().contains("cannot delegate"));
        let question = w.store.create_thread(&w.channel.id, &user.id, "q", &w.caller).unwrap();
        assert!(plan(&w.store, &question.id, &w.caller, &[task("t", "b", None)]).unwrap_err().contains("cannot delegate"));
    }

    #[test]
    fn agents_list_shows_membership_and_a_character_safe_personality_preview() {
        let w = world();
        let mut long = config("Long", false, Some("m"));
        long.system_prompt = "é".repeat(300);
        let long_id = w.store.create_agent(&long).unwrap();
        let outsider = AgentId::new();
        w.store.insert_agent(&outsider, &config("Outsider", false, None)).unwrap();
        w.store.invite_agent_to_channel(&w.project, &w.channel.id, &long_id).unwrap();
        let list = agent_summaries(&w.store, Some(&w.project), &w.caller).unwrap();
        let by = |name: &str| list.iter().find(|a| a.name == name).unwrap().clone();
        assert!(by("Main").member && by("Long").member && !by("Outsider").member);
        assert_eq!(by("Long").personality, "é".repeat(PERSONALITY_PREVIEW_CHARS));
        assert_eq!(by("Main").personality, "Main personality");
        assert_eq!((by("Outsider").provider.as_str(), by("Outsider").model), ("fake", None));
        let direct = agent_summaries(&w.store, None, &w.caller).unwrap();
        assert!(direct.iter().find(|a| a.name == "Main").unwrap().member && !direct.iter().find(|a| a.name == "Long").unwrap().member);
        w.store.set_agent_archived(&long_id, true).unwrap();
        assert!(!agent_summaries(&w.store, Some(&w.project), &w.caller).unwrap().iter().any(|a| a.name == "Long"), "archived agents are not listed");
        let _ = outsider;
    }

    #[test]
    fn the_final_text_is_every_text_block_of_the_message_in_order() {
        let md = |t: &str| Block::Markdown { text: t.into() };
        let call = Block::ToolCall { call_id: "c".into(), name: "x".into(), input: json!({}), output: None, is_error: None, sandboxed: None };
        assert_eq!(final_text(&[md("I found it."), call.clone(), md("Saved.")]), "I found it.\n\nSaved.", "findings before a tool call are kept");
        assert_eq!(final_text(&[md("I will look."), call.clone(), md("Found it.\n"), md("Done.")]), "I will look.\n\nFound it.\n\nDone.");
        assert_eq!(final_text(&[md("Only text.")]), "Only text.");
        assert_eq!(final_text(&[md("Before the call."), call.clone()]), "Before the call.");
        assert_eq!(final_text(&[call]), "");
        assert_eq!(final_text(&[]), "");
    }

    #[test]
    fn a_result_is_cut_at_32_kb_on_a_character_boundary_with_a_note() {
        let long = "é".repeat(MAX_RESULT_BYTES);
        let cut = final_text(&[Block::Markdown { text: long.clone() }]);
        let (head, note) = cut.split_once("\n\n[").expect("a note follows the text");
        assert!(head.len() <= MAX_RESULT_BYTES && head.chars().all(|c| c == 'é'), "cut on a character boundary");
        assert!(MAX_RESULT_BYTES - head.len() < 4, "as much as fits is kept");
        assert!(note.contains("32 KB") && note.contains("thread"), "{note}");
        let exact = "a".repeat(MAX_RESULT_BYTES);
        assert_eq!(final_text(&[Block::Markdown { text: exact.clone() }]), exact, "a result at the limit is not cut");
    }

    #[test]
    fn results_come_back_in_task_order_and_each_failure_states_how_it_ended() {
        let result = |name: &str, state, text: &str| DelegateResult { thread_channel_id: ChannelId::new(), name: name.into(), state, text: text.into(), steer_note: None };
        let text = format_results(&[
            result("a", ThreadState::Done, "The answer."),
            result("b", ThreadState::Failed, "Provider error."),
            result("c", ThreadState::Cancelled, ""),
            result("d", ThreadState::TimedOut, ""),
            result("e", ThreadState::Done, "  "),
        ]);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rows = value["results"].as_array().unwrap();
        assert_eq!(rows.iter().map(|r| r["task"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        assert_eq!(rows[4]["result"], "The sub-agent finished without a text reply.", "an empty final message is still a result");
        assert_eq!((rows[0]["thread"].as_str(), rows[0]["state"].as_str(), rows[0]["result"].as_str()), (Some("a"), Some("done"), Some("The answer.")));
        assert!(rows[0].get("error").is_none());
        assert_eq!(rows[1]["error"], "The sub-agent task failed. Provider error.");
        assert_eq!(rows[2]["error"], "The sub-agent task was cancelled.");
        assert_eq!(rows[3]["error"], "The sub-agent task timed out after 10 minutes.");
        assert!(rows[1..4].iter().all(|r| r.get("result").is_none()));
    }

    #[test]
    fn a_steer_note_counts_the_messages_and_quotes_the_latest() {
        assert_eq!(steer_note(0, "ignored"), None, "no steer, no note");
        assert_eq!(
            steer_note(1, "Only count the .rs files.").unwrap(),
            "Note: the user sent this sub-agent 1 message during the task. The result follows the user's instructions, which can differ from your brief. Latest user message: \"Only count the .rs files.\""
        );
        assert!(steer_note(2, "x").unwrap().starts_with("Note: the user sent this sub-agent 2 messages during the task."));
        assert!(steer_note(11, "x").unwrap().contains("11 messages"));
    }

    #[test]
    fn a_steer_note_cuts_the_message_at_500_characters_and_swaps_quotes() {
        let at_limit = "é".repeat(MAX_STEER_NOTE_CHARS);
        assert!(steer_note(1, &at_limit).unwrap().ends_with(&format!("\"{at_limit}\"")), "a message at the limit is not cut");
        let cut = steer_note(1, &"é".repeat(MAX_STEER_NOTE_CHARS + 1)).unwrap();
        assert!(cut.ends_with(&format!("\"{}…\"", "é".repeat(MAX_STEER_NOTE_CHARS))), "cut on a character: {cut}");
        assert!(steer_note(1, "Say \"hi\" now").unwrap().ends_with("Latest user message: \"Say 'hi' now\""));
    }

    #[test]
    fn the_note_leads_the_result_text_in_every_state() {
        let result = |state, text: &str| DelegateResult { thread_channel_id: ChannelId::new(), name: "a".into(), state, text: text.into(), steer_note: steer_note(1, "Stop.") };
        let value: serde_json::Value = serde_json::from_str(&format_results(&[result(ThreadState::Done, "Answer."), result(ThreadState::Done, ""), result(ThreadState::Cancelled, "")])).unwrap();
        let rows = value["results"].as_array().unwrap();
        let note = steer_note(1, "Stop.").unwrap();
        assert_eq!(rows[0]["result"], format!("{note}\n\nAnswer.").as_str());
        assert_eq!(rows[1]["result"], format!("{note}\n\nThe sub-agent finished without a text reply.").as_str());
        assert_eq!(rows[2]["error"], format!("{note}\n\nThe sub-agent task was cancelled.").as_str());
    }

    #[test]
    fn the_slot_starts_empty_and_hands_out_the_installed_delegator() {
        struct Fixed;
        impl Delegator for Fixed {
            fn delegate(&self, _: &ChannelId, _: &AgentId, tasks: Vec<DelegateTask>) -> Result<Vec<DelegateResult>, String> {
                Ok(tasks.into_iter().map(|t| DelegateResult { thread_channel_id: ChannelId::new(), name: t.name, state: ThreadState::Done, text: "ok".into(), steer_note: None }).collect())
            }
        }
        let slot = DelegatorSlot::default();
        assert!(slot.get().is_none());
        slot.install(Arc::new(Fixed));
        let delegator = slot.get().expect("installed");
        slot.install(Arc::new(Fixed));
        assert_eq!(delegator.delegate(&ChannelId::new(), &AgentId::new(), vec![DelegateTask { name: "n".into(), brief: "b".into(), agent: AgentId::new() }]).unwrap().len(), 1);
    }
}
