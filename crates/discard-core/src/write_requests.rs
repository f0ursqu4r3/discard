//! Ephemeral, user-only requests. No pending approval survives an app restart.
use crate::model::{AgentId, ChannelId};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(10 * 60);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRequestKind {
    WorktreeCreate,
    MainCheckoutWrite,
    ConversationMove,
    BranchCreate,
}
const HANDOFF_MAX_BYTES: usize = 4096;
const LIST_FIELDS: [&str; 3] = ["constraints", "decisions", "open_questions"];
/// What the next conversation needs to continue the work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handoff {
    pub goal: String,
    pub constraints: Vec<String>,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
}
impl Handoff {
    pub fn from_args(args: &serde_json::Map<String, serde_json::Value>) -> Result<Handoff, String> {
        let goal = args.get("goal").and_then(|v| v.as_str()).map(str::trim).filter(|g| !g.is_empty()).ok_or("`goal` is required.")?;
        if goal.chars().count() > 600 {
            return Err("Keep `goal` under 600 characters.".into());
        }
        let list = |name: &str| -> Result<Vec<String>, String> {
            let items = match args.get(name) {
                None | Some(serde_json::Value::Null) => return Ok(vec![]),
                Some(serde_json::Value::Array(items)) => items,
                Some(_) => return Err(format!("`{name}` must be a list of strings.")),
            };
            let items: Vec<&str> = items.iter().map(|v| v.as_str()).collect::<Option<_>>().ok_or_else(|| format!("`{name}` must be a list of strings."))?;
            if items.len() > 12 {
                return Err(format!("Give at most 12 items in `{name}`."));
            }
            items
                .into_iter()
                .map(|item| {
                    let item = item.trim();
                    if item.is_empty() || item.chars().count() > 400 {
                        return Err(format!("Each item in `{name}` needs 1 to 400 characters."));
                    }
                    Ok(item.to_string())
                })
                .collect()
        };
        let [constraints, decisions, open_questions] = LIST_FIELDS.map(list);
        let handoff = Handoff { goal: goal.into(), constraints: constraints?, decisions: decisions?, open_questions: open_questions? };
        if handoff.markdown().len() > HANDOFF_MAX_BYTES {
            return Err(format!("The handoff is too long. Keep it under {HANDOFF_MAX_BYTES} bytes."));
        }
        Ok(handoff)
    }
    pub fn markdown(&self) -> String {
        let mut parts = vec![format!("**Goal:** {}", self.goal)];
        for (title, items) in [("Constraints", &self.constraints), ("Decisions", &self.decisions), ("Open questions", &self.open_questions)] {
            if !items.is_empty() {
                parts.push(format!("**{title}**\n{}", items.iter().map(|item| format!("- {item}")).collect::<Vec<_>>().join("\n")));
            }
        }
        parts.join("\n\n")
    }
}
/// Where a conversation moves to. `repository` is the repository ID of the project channel metadata (empty for the project root). `repo_path` is its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum MoveTarget {
    Channel { channel_id: String, name: String },
    NewBranch { repository: String, repo_path: String, branch: String, base: String, base_commit: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveDetails {
    pub target: MoveTarget,
    pub handoff: Handoff,
    pub source_message_id: String,
    /// Attachments of the source that the handoff carries to the target. Approval loads each one again by ID.
    #[serde(default)]
    pub attachments: Vec<MoveAttachment>,
}
/// An attachment that a move carries. The card shows the name. The ID finds the file at approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveAttachment {
    pub id: String,
    pub name: String,
}
/// The attachments in the messages of a channel, each file once, oldest first.
pub fn source_attachments(
    store: &crate::store::Store,
    channel: &ChannelId,
) -> Result<Vec<MoveAttachment>, String> {
    Ok(attachments_of(store, channel)?
        .into_iter()
        .map(|a| MoveAttachment {
            id: a.id,
            name: a.name,
        })
        .collect())
}
/// The saved attachments of a channel with these IDs, in the order of `ids`. A missing ID is an error.
pub fn load_source_attachments(
    store: &crate::store::Store,
    channel: &ChannelId,
    ids: &[MoveAttachment],
) -> Result<Vec<crate::attachments::Attachment>, String> {
    let held = attachments_of(store, channel)?;
    ids.iter()
        .map(|wanted| {
            held.iter()
                .find(|a| a.id == wanted.id)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "The attachment {} is no longer in the channel. Ask again.",
                        wanted.name
                    )
                })
        })
        .collect()
}
fn attachments_of(
    store: &crate::store::Store,
    channel: &ChannelId,
) -> Result<Vec<crate::attachments::Attachment>, String> {
    let mut seen = std::collections::HashSet::new();
    Ok(store
        .messages_for_channel(channel)
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|m| m.blocks)
        .filter_map(|b| match b {
            crate::model::Block::Attachment { attachment } => Some(attachment),
            _ => None,
        })
        .filter(|a| seen.insert(a.id.clone()))
        .collect())
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteRequest {
    pub id: String,
    pub kind: WriteRequestKind,
    pub channel_id: String,
    pub agent_id: String,
    pub branch: String,
    pub path: String,
    pub state: String,
    pub result: Option<String>,
    #[serde(rename = "move", skip_serializing_if = "Option::is_none")]
    pub details: Option<MoveDetails>,
    /// The commit a branch-creation request names. `None` for other kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}
struct Pending {
    request: WriteRequest,
    created: Instant,
}
#[derive(Default)]
pub struct WriteRequests {
    inner: Mutex<HashMap<String, Pending>>,
}
impl WriteRequests {
    fn expire(rows: &mut HashMap<String, Pending>) {
        for pending in rows.values_mut() {
            if pending.request.state == "pending" && pending.created.elapsed() >= TTL {
                pending.request.state = "expired".into();
                pending.request.result = Some("Approval expired. Ask for a new request.".into());
            }
        }
    }
    pub fn request(
        &self,
        kind: WriteRequestKind,
        channel: &ChannelId,
        agent: &AgentId,
        branch: String,
        path: String,
    ) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        if let Some(existing) = rows.values().find(|p| {
            p.request.channel_id == channel.0
                && p.request.kind == kind
                && matches!(p.request.state.as_str(), "pending" | "running")
        }) {
            return Ok(existing.request.clone());
        }
        Self::insert(&mut rows, kind, channel, agent, branch, path, None, None)
    }
    /// Ask to create `branch` at `commit` in the detached worktree at `path`. One request per channel stays pending.
    pub fn request_branch_create(&self, channel: &ChannelId, agent: &AgentId, branch: String, path: String, commit: String) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        if let Some(existing) = rows.values().find(|p| p.request.channel_id == channel.0 && p.request.kind == WriteRequestKind::BranchCreate && matches!(p.request.state.as_str(), "pending" | "running")) {
            return Ok(existing.request.clone());
        }
        Self::insert(&mut rows, WriteRequestKind::BranchCreate, channel, agent, branch, path, None, Some(commit))
    }
    pub fn request_move(&self, channel: &ChannelId, agent: &AgentId, details: MoveDetails) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        if rows.values().any(|p| p.request.channel_id == channel.0 && p.request.kind == WriteRequestKind::ConversationMove && matches!(p.request.state.as_str(), "pending" | "running")) {
            return Err("A move request is already pending in the app. The user must decide it first.".into());
        }
        let branch = match &details.target {
            MoveTarget::NewBranch { branch, .. } => branch.clone(),
            MoveTarget::Channel { .. } => String::new(),
        };
        Self::insert(&mut rows, WriteRequestKind::ConversationMove, channel, agent, branch, String::new(), Some(details), None)
    }
    fn insert(
        rows: &mut HashMap<String, Pending>,
        kind: WriteRequestKind,
        channel: &ChannelId,
        agent: &AgentId,
        branch: String,
        path: String,
        details: Option<MoveDetails>,
        commit: Option<String>,
    ) -> Result<WriteRequest, String> {
        if rows.len() >= 100 {
            let oldest = rows
                .iter()
                .filter(|(_, p)| !matches!(p.request.state.as_str(), "pending" | "running"))
                .min_by_key(|(_, p)| p.created)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                rows.remove(&id);
            } else {
                return Err("Too many pending write requests.".into());
            }
        }
        let request = WriteRequest {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            channel_id: channel.0.clone(),
            agent_id: agent.0.clone(),
            branch,
            path,
            state: "pending".into(),
            result: None,
            details,
            commit,
        };
        rows.insert(
            request.id.clone(),
            Pending {
                request: request.clone(),
                created: Instant::now(),
            },
        );
        Ok(request)
    }
    pub fn list(&self, channel: &ChannelId) -> Vec<WriteRequest> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let mut rows: Vec<_> = rows
            .values()
            .filter(|p| p.request.channel_id == channel.0)
            .collect();
        rows.sort_by_key(|p| p.created);
        rows.into_iter().map(|p| p.request.clone()).collect()
    }
    pub fn begin(
        &self,
        channel: &ChannelId,
        id: &str,
        approve: bool,
    ) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let row = rows
            .get_mut(id)
            .filter(|p| p.request.channel_id == channel.0)
            .ok_or("This request does not belong to this channel.")?;
        if row.request.state != "pending" {
            return Err("This request is no longer pending.".into());
        }
        row.request.state = if approve { "running" } else { "denied" }.into();
        if !approve {
            row.request.result = Some(
                match row.request.kind {
                    WriteRequestKind::WorktreeCreate => "Worktree creation denied.",
                    WriteRequestKind::MainCheckoutWrite => "Main-checkout write access denied.",
                    WriteRequestKind::ConversationMove => "Conversation move denied.",
                    WriteRequestKind::BranchCreate => "Branch creation was denied.",
                }
                .into(),
            );
        }
        Ok(row.request.clone())
    }
    pub fn finish(&self, id: &str, result: Result<String, String>) -> WriteRequest {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let row = rows
            .get_mut(id)
            .expect("begin retains running requests until finish");
        if row.request.state == "running" {
            row.request.state = if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
            .into();
            row.request.result = Some(result.unwrap_or_else(|error| error));
        }
        row.request.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(requests: &WriteRequests, channel: &ChannelId) -> WriteRequest {
        requests
            .request(
                WriteRequestKind::WorktreeCreate,
                channel,
                &AgentId::new(),
                "feature".into(),
                "/target".into(),
            )
            .unwrap()
    }
    #[test]
    fn requests_expire_after_ttl() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        requests
            .inner
            .lock()
            .unwrap()
            .get_mut(&r.id)
            .unwrap()
            .created = Instant::now() - TTL;
        assert!(requests.begin(&channel, &r.id, true).is_err());
        assert_eq!(requests.list(&channel)[0].state, "expired");
    }
    #[test]
    fn a_request_is_decided_once() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert_eq!(
            requests.begin(&channel, &r.id, true).unwrap().state,
            "running"
        );
        assert!(requests.begin(&channel, &r.id, true).is_err());
        assert_eq!(requests.finish(&r.id, Ok("done".into())).state, "completed");
    }
    #[test]
    fn decide_for_another_channel_is_refused() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert!(requests.begin(&ChannelId::new(), &r.id, true).is_err());
        assert_eq!(
            requests.begin(&channel, &r.id, false).unwrap().state,
            "denied"
        );
    }
    #[test]
    fn one_pending_request_per_channel_and_kind() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert_eq!(request(&requests, &channel).id, r.id);
        assert!(
            requests
                .request(
                    WriteRequestKind::MainCheckoutWrite,
                    &channel,
                    &AgentId::new(),
                    "main".into(),
                    "/repo".into()
                )
                .is_ok()
        );
        assert_eq!(requests.list(&channel).len(), 2);
    }
    #[test]
    fn branch_create_request_carries_branch_path_and_commit() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = requests.request_branch_create(&channel, &AgentId::new(), "topic".into(), "/wt".into(), "abc123".into()).unwrap();
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!((json["kind"].as_str(), json["branch"].as_str(), json["path"].as_str(), json["commit"].as_str()), (Some("branch_create"), Some("topic"), Some("/wt"), Some("abc123")));
        assert!(serde_json::to_value(request(&requests, &channel)).unwrap().get("commit").is_none());
    }
    #[test]
    fn one_pending_branch_create_request_per_channel() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let make = |c: &ChannelId| requests.request_branch_create(c, &AgentId::new(), "topic".into(), "/wt".into(), "abc".into()).unwrap();
        let first = make(&channel);
        assert_eq!(make(&channel).id, first.id);
        assert_ne!(make(&ChannelId::new()).id, first.id);
        assert_ne!(request(&requests, &channel).id, first.id, "another kind is separate");
        assert_eq!(requests.begin(&channel, &first.id, false).unwrap().result.as_deref(), Some("Branch creation was denied."));
        assert_ne!(make(&channel).id, first.id, "a decided request frees the slot");
    }
    #[test]
    fn branch_create_requests_expire_after_ttl() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = requests.request_branch_create(&channel, &AgentId::new(), "topic".into(), "/wt".into(), "abc".into()).unwrap();
        requests.inner.lock().unwrap().get_mut(&r.id).unwrap().created = Instant::now() - TTL;
        assert!(requests.begin(&channel, &r.id, true).is_err());
        assert_eq!(requests.list(&channel)[0].state, "expired");
    }
    fn details() -> MoveDetails {
        MoveDetails {
            target: MoveTarget::Channel { channel_id: "c".into(), name: "feature".into() },
            handoff: Handoff { goal: "Ship it".into(), constraints: vec![], decisions: vec![], open_questions: vec!["Which?".into()] },
            source_message_id: "m1".into(),
            attachments: vec![],
        }
    }
    fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }
    #[test]
    fn move_request_carries_target_and_handoff() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = requests.request_move(&channel, &AgentId::new(), details()).unwrap();
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["kind"], "conversation_move");
        assert_eq!(json["move"]["target"]["kind"], "channel");
        assert_eq!(json["move"]["target"]["channelId"], "c");
        assert_eq!(json["move"]["handoff"]["openQuestions"][0], "Which?");
        assert_eq!(json["move"]["sourceMessageId"], "m1");
        assert_eq!(json["branch"], "");
        assert_eq!(json["path"], "");
        let plain = serde_json::to_value(request(&requests, &channel)).unwrap();
        assert!(plain.get("move").is_none());
    }
    #[test]
    fn new_branch_target_serializes_with_camel_case_fields() {
        let target = MoveTarget::NewBranch { repository: "r".into(), repo_path: "/repo".into(), branch: "feat".into(), base: "main".into(), base_commit: "abc".into() };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json, serde_json::json!({"kind":"new_branch","repository":"r","repoPath":"/repo","branch":"feat","base":"main","baseCommit":"abc"}));
        let requests = WriteRequests::default();
        let mut d = details();
        d.target = target;
        assert_eq!(requests.request_move(&ChannelId::new(), &AgentId::new(), d).unwrap().branch, "feat");
    }
    #[test]
    fn a_second_move_request_is_refused_while_one_is_pending() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = requests.request_move(&channel, &AgentId::new(), details()).unwrap();
        assert_eq!(requests.request_move(&channel, &AgentId::new(), details()).unwrap_err(), "A move request is already pending in the app. The user must decide it first.");
        requests.begin(&channel, &r.id, false).unwrap();
        assert!(requests.request_move(&channel, &AgentId::new(), details()).is_ok());
    }
    #[test]
    fn a_denied_move_has_the_denial_text() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = requests.request_move(&channel, &AgentId::new(), details()).unwrap();
        assert_eq!(requests.begin(&channel, &r.id, false).unwrap().result, Some("Conversation move denied.".into()));
    }
    #[test]
    fn handoff_needs_a_goal_and_bounded_text() {
        let err = |value: serde_json::Value| Handoff::from_args(&args(value)).unwrap_err();
        assert_eq!(err(serde_json::json!({})), "`goal` is required.");
        assert_eq!(err(serde_json::json!({"goal": "  "})), "`goal` is required.");
        assert_eq!(err(serde_json::json!({"goal": "x".repeat(601)})), "Keep `goal` under 600 characters.");
        for name in ["constraints", "decisions", "open_questions"] {
            assert_eq!(err(serde_json::json!({"goal": "g", name: "text"})), format!("`{name}` must be a list of strings."));
            assert_eq!(err(serde_json::json!({"goal": "g", name: [1]})), format!("`{name}` must be a list of strings."));
            assert_eq!(err(serde_json::json!({"goal": "g", name: vec!["a"; 13]})), format!("Give at most 12 items in `{name}`."));
            assert_eq!(err(serde_json::json!({"goal": "g", name: ["x".repeat(401)]})), format!("Each item in `{name}` needs 1 to 400 characters."));
            assert_eq!(err(serde_json::json!({"goal": "g", name: [" "]})), format!("Each item in `{name}` needs 1 to 400 characters."));
        }
        let long = "y".repeat(400);
        assert_eq!(err(serde_json::json!({"goal": "g", "constraints": vec![long.clone(); 12], "decisions": vec![long; 12]})), "The handoff is too long. Keep it under 4096 bytes.");
        let ok = Handoff::from_args(&args(serde_json::json!({"goal": " Ship ", "decisions": [" Use CSV "]}))).unwrap();
        assert_eq!(ok.goal, "Ship");
        assert_eq!(ok.decisions, vec!["Use CSV".to_string()]);
        assert!(ok.constraints.is_empty() && ok.open_questions.is_empty());
    }
    #[test]
    fn handoff_markdown_omits_empty_sections() {
        let handoff = Handoff { goal: "Ship the importer".into(), constraints: vec![], decisions: vec!["Use CSV".into()], open_questions: vec!["Which delimiter?".into()] };
        assert_eq!(handoff.markdown(), "**Goal:** Ship the importer\n\n**Decisions**\n- Use CSV\n\n**Open questions**\n- Which delimiter?");
    }
}
