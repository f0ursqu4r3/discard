//! Requests from an agent to add another agent to the project (`agent_request_add`). Like `write_requests`, a
//! request is ephemeral and only the user decides it: no pending request survives an app restart.
//!
//! The tool returns "approval required" at once. The app lists the requests of the parent channel as approval
//! cards and calls `decide`. The decision returns a `FollowUp`, which the app sends to the parent agent through
//! `Command::Continue` (added) or `Command::ContinueDeclined` (declined).
use crate::model::{AgentId, ChannelId, MessageId};
use crate::store::Store;
use serde::Serialize;
use std::{collections::HashMap, sync::Mutex, time::{Duration, Instant}};

const TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ROWS: usize = 100;
/// Characters of the reason that the card shows.
pub const REASON_CHARS: usize = 500;

/// Request states: `pending`, `added`, `denied`, `failed`, `expired`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAddRequest {
    pub id: String,
    pub channel_id: String,
    pub requesting_agent_id: String,
    pub requesting_agent_name: String,
    pub agent_id: String,
    pub agent_name: String,
    pub provider: String,
    pub model: Option<String>,
    pub reason: String,
    pub state: String,
    pub result: Option<String>,
    /// When the request was made, in Unix milliseconds.
    pub created_at: u64,
}

/// What the app tells the parent agent after the user decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUp {
    /// The agent that asked.
    pub agent: AgentId,
    pub note: String,
    /// The user declined. Send `Command::ContinueDeclined`; otherwise send `Command::Continue`.
    pub declined: bool,
}

/// The result of `decide`. `follow_up` is `None` when adding the agent failed: the card shows the failure and the
/// parent agent is not continued.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentDecision {
    pub request: AgentAddRequest,
    pub follow_up: Option<FollowUp>,
}

struct Pending {
    request: AgentAddRequest,
    created: Instant,
    /// For a denied request: the user's latest message when it was denied. The same agent cannot be requested
    /// again in the channel until the user writes another message.
    denied_after: Option<Option<MessageId>>,
}

#[derive(Default)]
pub struct AgentRequests {
    inner: Mutex<HashMap<String, Pending>>,
}

impl AgentRequests {
    fn expire(rows: &mut HashMap<String, Pending>) {
        for pending in rows.values_mut() {
            if pending.request.state == "pending" && pending.created.elapsed() >= TTL {
                pending.request.state = "expired".into();
                pending.request.result = Some("The request expired. Ask again.".into());
            }
        }
    }

    /// Record a request from `requester` in `channel` to add the agent named `name`. Returns the pending request,
    /// or the existing one when the same agent is already pending in the channel.
    ///
    /// It is an error in a direct message or a thread, from an agent without `delegation`, for an unknown or archived
    /// agent, for an agent that is already a member, with an empty reason, and for an agent the user already declined
    /// since the user's latest message.
    pub fn request(&self, store: &Store, channel: &ChannelId, requester: &AgentId, name: &str, reason: &str) -> Result<AgentAddRequest, String> {
        let parent = store.load_channel(channel).map_err(|e| e.to_string())?;
        if store.thread_info(channel).map_err(|e| e.to_string())?.is_some() { return Err("A sub-agent cannot ask to add agents.".into()); }
        let Some(project) = parent.project_id else { return Err("agent_request_add is unavailable in a direct message. A direct message can start sub-agents only with your own configuration.".into()); };
        let requester_config = store.load_agent(requester).map_err(|e| e.to_string())?;
        if !requester_config.delegation { return Err("Delegation is off for this agent.".into()); }
        let reason = reason.trim();
        if reason.is_empty() { return Err("Give a reason. The user reads it on the approval card.".into()); }
        let reason: String = reason.chars().take(REASON_CHARS).collect();
        let wanted = name.trim().to_lowercase();
        let all = store.all_agents().map_err(|e| e.to_string())?;
        let matching: Vec<_> = all.iter().filter(|(_, config, _)| config.name.to_lowercase() == wanted).collect();
        let active: Vec<_> = matching.iter().filter(|(_, _, archived)| !archived).collect();
        let (agent_id, config) = match active.as_slice() {
            [(id, config, _)] => (id.clone(), config.clone()),
            [] if matching.is_empty() => return Err(format!("No agent is named {}. Call agents_list to see the agents.", name.trim())),
            [] => return Err(format!("The agent {} is archived. Choose another agent from agents_list.", name.trim())),
            _ => return Err(format!("More than one agent is named {}. Ask the user to give them distinct names.", name.trim())),
        };
        if crate::delegation::is_member(store, &project, &agent_id)? { return Err(format!("{} is already a member of this project. Use it in delegate.", config.name)); }
        let last_user = store.last_user_message_id(channel).map_err(|e| e.to_string())?;
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let same = |p: &&Pending| p.request.channel_id == channel.0 && p.request.agent_id == agent_id.0;
        if rows.values().filter(same).any(|p| p.denied_after.as_ref() == Some(&last_user)) {
            return Err(format!("The user declined adding {}. Do not ask again until the user sends another message.", config.name));
        }
        if let Some(existing) = rows.values().filter(same).find(|p| p.request.state == "pending") { return Ok(existing.request.clone()); }
        if rows.len() >= MAX_ROWS {
            let oldest = rows.iter().filter(|(_, p)| p.request.state != "pending").min_by_key(|(_, p)| p.created).map(|(id, _)| id.clone());
            match oldest { Some(id) => { rows.remove(&id); } None => return Err("Too many pending requests.".into()) }
        }
        let request = AgentAddRequest {
            id: uuid::Uuid::new_v4().to_string(), channel_id: channel.0.clone(), requesting_agent_id: requester.0.clone(), requesting_agent_name: requester_config.name,
            agent_id: agent_id.0.clone(), agent_name: config.name, provider: config.provider, model: config.model, reason, state: "pending".into(), result: None,
            created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
        };
        rows.insert(request.id.clone(), Pending { request: request.clone(), created: Instant::now(), denied_after: None });
        Ok(request)
    }

    /// The requests of a channel, oldest first.
    pub fn list(&self, channel: &ChannelId) -> Vec<AgentAddRequest> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let mut rows: Vec<_> = rows.values().filter(|p| p.request.channel_id == channel.0).collect();
        rows.sort_by_key(|p| p.created);
        rows.into_iter().map(|p| p.request.clone()).collect()
    }

    /// Decide a pending request of `channel`. Approving adds the agent to the project and to the channel, through
    /// `Store::invite_agent_to_channel`, with no end date. Denying records the decision. A request is decided once.
    pub fn decide(&self, store: &Store, channel: &ChannelId, id: &str, approve: bool) -> Result<AgentDecision, String> {
        let request = {
            let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            Self::expire(&mut rows);
            let row = rows.get_mut(id).filter(|p| p.request.channel_id == channel.0).ok_or("This request does not belong to this channel.")?;
            if row.request.state != "pending" { return Err("This request is no longer pending.".into()); }
            // Claim the request before storage work, so a second decision finds it taken.
            row.request.state = if approve { "added" } else { "denied" }.into();
            row.request.clone()
        };
        let requester = AgentId(request.requesting_agent_id.clone());
        let outcome = if approve {
            let parent = store.load_channel(channel).map_err(|e| e.to_string());
            parent.and_then(|parent| parent.project_id.ok_or_else(|| "This channel has no project.".to_string()))
                .and_then(|project| store.invite_agent_to_channel(&project, channel, &AgentId(request.agent_id.clone())).map_err(|e| e.to_string()))
        } else { Ok(()) };
        let denied_after = if approve { None } else { Some(store.last_user_message_id(channel).map_err(|e| e.to_string())?) };
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let row = rows.get_mut(id).expect("a claimed request stays until it is decided");
        row.denied_after = denied_after;
        let follow_up = match (approve, outcome) {
            (true, Ok(())) => {
                row.request.result = Some(format!("{} is now a project member.", request.agent_name));
                Some(FollowUp { agent: requester, note: format!("The user added {} to this project. Use it in delegate now.", request.agent_name), declined: false })
            }
            (true, Err(error)) => {
                row.request.state = "failed".into();
                row.request.result = Some(format!("Adding {} failed: {error}", request.agent_name));
                None
            }
            (false, _) => {
                row.request.result = Some(format!("The request to add {} was declined.", request.agent_name));
                Some(FollowUp { agent: requester, note: format!("The user declined to add {} to this project. Do not ask for it again unless the user asks you to.", request.agent_name), declined: true })
            }
        };
        Ok(AgentDecision { request: row.request.clone(), follow_up })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::capability::ApprovalPolicy;
    use crate::model::{Block, Channel, Message, Project, ProjectId, Role, WorkspaceMode};

    struct World { store: Store, project: ProjectId, channel: Channel, caller: AgentId, outsider: AgentId, requests: AgentRequests }

    fn config(name: &str, delegation: bool) -> AgentConfig {
        AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation, reasoning_effort: None, name: name.into(), avatar: None, provider: "codex".into(), model: Some("gpt".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }
    }

    fn world() -> World {
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "demo".into(), path: "/tmp/demo".into() };
        store.insert_project(&project).unwrap();
        let caller = store.create_agent(&config("Main", true)).unwrap();
        store.set_project_main_agent(&project.id, &caller).unwrap();
        let outsider = store.create_agent(&config("Reviewer", false)).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: caller.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        store.insert_channel(&channel).unwrap();
        World { store, project: project.id, channel, caller, outsider, requests: AgentRequests::default() }
    }

    fn user_says(w: &World, text: &str) {
        w.store.insert_message(&Message { id: MessageId::new(), channel_id: w.channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: text.into() }] }).unwrap();
    }

    fn ask(w: &World, name: &str) -> Result<AgentAddRequest, String> { w.requests.request(&w.store, &w.channel.id, &w.caller, name, "Needs a second opinion.") }

    #[test]
    fn a_request_is_pending_with_the_details_for_the_card_and_repeats_return_the_same_one() {
        let w = world();
        user_says(&w, "review this");
        let request = ask(&w, " reviewer ").unwrap();
        assert_eq!((request.state.as_str(), request.agent_name.as_str(), request.provider.as_str(), request.model.as_deref()), ("pending", "Reviewer", "codex", Some("gpt")));
        assert_eq!((request.requesting_agent_name.as_str(), request.reason.as_str(), request.agent_id.as_str()), ("Main", "Needs a second opinion.", w.outsider.0.as_str()));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        assert!(request.created_at > 0 && request.created_at <= now && now - request.created_at < 60_000, "the card shows when it was asked: {}", request.created_at);
        assert_eq!(ask(&w, "Reviewer").unwrap().id, request.id, "one pending request for each agent and channel");
        assert_eq!(w.requests.list(&w.channel.id), vec![request]);
        assert!(w.requests.list(&ChannelId::new()).is_empty());
        assert!(!crate::delegation::is_member(&w.store, &w.project, &w.outsider).unwrap(), "nothing changes before the user decides");
    }

    #[test]
    fn invalid_requests_are_refused() {
        let w = world();
        user_says(&w, "go");
        assert!(ask(&w, "Nobody").unwrap_err().contains("agents_list"));
        assert!(ask(&w, "Main").unwrap_err().contains("already a member"));
        assert!(w.requests.request(&w.store, &w.channel.id, &w.caller, "Reviewer", "  ").unwrap_err().contains("reason"));
        let long = w.requests.request(&w.store, &w.channel.id, &w.caller, "Reviewer", &"é".repeat(900)).unwrap();
        assert_eq!(long.reason.chars().count(), REASON_CHARS);
        let direct = w.store.create_conversation(&w.caller, None, "chat").unwrap();
        assert!(w.requests.request(&w.store, &direct.id, &w.caller, "Reviewer", "why").unwrap_err().contains("direct message"));
        let plain = w.store.create_agent(&config("Plain", false)).unwrap();
        assert!(w.requests.request(&w.store, &w.channel.id, &plain, "Reviewer", "why").unwrap_err().contains("Delegation is off"));
        let thread = w.store.create_sub_agent_thread(&w.channel.id, "t", &w.caller).unwrap();
        assert!(w.requests.request(&w.store, &thread.id, &w.caller, "Reviewer", "why").unwrap_err().contains("sub-agent"));
        w.store.set_agent_archived(&w.outsider, true).unwrap();
        assert!(ask(&w, "Reviewer").unwrap_err().contains("archived"));
    }

    #[test]
    fn approving_adds_the_agent_to_the_project_and_continues_the_parent() {
        let w = world();
        user_says(&w, "go");
        let request = ask(&w, "Reviewer").unwrap();
        let decision = w.requests.decide(&w.store, &w.channel.id, &request.id, true).unwrap();
        assert_eq!(decision.request.state, "added");
        assert_eq!(decision.follow_up, Some(FollowUp { agent: w.caller.clone(), note: "The user added Reviewer to this project. Use it in delegate now.".into(), declined: false }));
        assert!(crate::delegation::is_member(&w.store, &w.project, &w.outsider).unwrap());
        assert!(w.store.channel_agents(&w.channel.id).unwrap().contains(&w.outsider));
        assert!(w.requests.decide(&w.store, &w.channel.id, &request.id, true).unwrap_err().contains("no longer pending"), "decided once");
        assert!(ask(&w, "Reviewer").unwrap_err().contains("already a member"));
    }

    #[test]
    fn denying_continues_with_a_declined_note_and_blocks_a_repeat_until_the_next_user_message() {
        let w = world();
        user_says(&w, "go");
        let request = ask(&w, "Reviewer").unwrap();
        let decision = w.requests.decide(&w.store, &w.channel.id, &request.id, false).unwrap();
        assert_eq!(decision.request.state, "denied");
        let follow_up = decision.follow_up.unwrap();
        assert!(follow_up.declined && follow_up.note.contains("declined") && follow_up.agent == w.caller);
        assert!(!crate::delegation::is_member(&w.store, &w.project, &w.outsider).unwrap());
        assert!(ask(&w, "Reviewer").unwrap_err().contains("declined"), "refused in the same turn");
        user_says(&w, "ok, try again");
        let again = ask(&w, "Reviewer").unwrap();
        assert_ne!(again.id, request.id);
        assert_eq!(again.state, "pending", "the next user message lifts the refusal");
        let other = w.store.create_agent(&config("Other", false)).unwrap();
        assert!(w.requests.request(&w.store, &w.channel.id, &w.caller, "Other", "x").is_ok());
        let _ = other;
    }

    #[test]
    fn decisions_are_scoped_to_the_channel_and_expired_requests_cannot_be_decided() {
        let w = world();
        user_says(&w, "go");
        let request = ask(&w, "Reviewer").unwrap();
        assert!(w.requests.decide(&w.store, &ChannelId::new(), &request.id, true).unwrap_err().contains("does not belong"));
        assert!(w.requests.decide(&w.store, &w.channel.id, "unknown", true).is_err());
        w.requests.inner.lock().unwrap().get_mut(&request.id).unwrap().created = Instant::now() - TTL;
        assert!(w.requests.decide(&w.store, &w.channel.id, &request.id, true).unwrap_err().contains("no longer pending"));
        assert_eq!(w.requests.list(&w.channel.id)[0].state, "expired");
        assert_eq!(ask(&w, "Reviewer").unwrap().state, "pending", "an expired request does not block a new one");
    }

    #[test]
    fn a_failed_add_shows_the_failure_and_does_not_continue_the_parent() {
        let w = world();
        user_says(&w, "go");
        let request = ask(&w, "Reviewer").unwrap();
        w.store.set_agent_archived(&w.outsider, true).unwrap();
        let decision = w.requests.decide(&w.store, &w.channel.id, &request.id, true).unwrap();
        assert_eq!(decision.request.state, "failed");
        assert!(decision.request.result.unwrap().contains("Restore the agent"));
        assert_eq!(decision.follow_up, None);
    }
}
