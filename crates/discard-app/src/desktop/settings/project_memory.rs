//! User-facing project memory. This does not expand provider memory access.
use super::memory::{self, MemoryResolution, domain_choices, failure, item_dto, response_records};
use discard_core::{
    memory::{MemoryStore, Scope},
    memory_items::{ItemKind, MemoryItem, ProposalAction},
    model::ProjectId,
    store::Store,
};
use serde_json::{Value, json};

fn project(store: &Store, id: &str) -> Result<ProjectId, String> {
    store
        .list_projects()
        .map_err(failure)?
        .into_iter()
        .find(|p| p.id.0 == id)
        .map(|p| p.id)
        .ok_or_else(|| "Project is unavailable. Choose an existing project.".into())
}

fn includes(scope: &Scope, project: &ProjectId) -> bool {
    match scope {
        Scope::Global => true,
        Scope::Project(id) | Scope::AgentProject(_, id) => id == project,
        Scope::Agent(_) => false,
    }
}

fn proposal_scope(memory: &MemoryStore, action: &ProposalAction) -> Result<Scope, String> {
    match action {
        ProposalAction::Create { scope, .. } => Ok(scope.clone()),
        ProposalAction::Update { item_id, .. } | ProposalAction::Forget { item_id, .. } => memory
            .read_item(item_id)
            .map(|item| item.scope)
            .map_err(failure),
    }
}

pub fn view(memory: &MemoryStore, store: &Store, id: &str) -> Result<Value, String> {
    let project = project(store, id)?;
    let choices: Vec<_> = domain_choices(memory, store)?
        .into_iter()
        .filter(|row| includes(&row.scope, &project))
        .collect();
    let scopes: Vec<_> = choices.iter().map(|row| row.scope.clone()).collect();
    let items = memory.list_items(&scopes).map_err(failure)?;
    let conversations = store.all_conversations().map_err(failure)?;
    let conversations: Vec<_> = conversations
        .iter()
        .filter(|c| c.project_id.as_ref() == Some(&project))
        .collect();
    let mut warnings = vec![];
    for conversation in &conversations {
        warnings.extend(response_records(memory, store, &conversation.id)?.1);
    }
    let proposals: Vec<_> = memory
        .pending_proposals()
        .map_err(failure)?
        .into_iter()
        .filter(|p| conversations.iter().any(|c| c.id == p.source_conversation))
        .filter(|p| proposal_scope(memory, &p.action).is_ok_and(|scope| includes(&scope, &project)))
        .map(|p| {
            let (scope, content, action) = match p.action {
                ProposalAction::Create { scope, content, .. } => (scope.label(), content, "Create"),
                ProposalAction::Update {
                    item_id, content, ..
                } => (
                    memory
                        .read_item(&item_id)
                        .map(|i| i.scope.label())
                        .unwrap_or_default(),
                    content,
                    "Update",
                ),
                ProposalAction::Forget { item_id, .. } => {
                    let item = memory.read_item(&item_id).ok();
                    (
                        item.as_ref().map(|i| i.scope.label()).unwrap_or_default(),
                        item.map(|i| i.content).unwrap_or_default(),
                        "Forget",
                    )
                }
            };
            json!({"id": p.id, "scope": scope, "content": content, "action": action,
                "sourceConversation": p.source_conversation.0,
                "sourceAgentName": store.load_agent(&p.agent).ok().map(|a| a.name)})
        })
        .collect();
    let scopes: Vec<_> = choices
        .iter()
        .map(|row| {
            let (title, agent) = match &row.scope {
                Scope::Global => ("Global · all projects".to_string(), None),
                Scope::Project(_) => ("Shared project memory".to_string(), None),
                Scope::AgentProject(agent, _) => (
                    store
                        .load_agent(agent)
                        .map(|a| a.name)
                        .unwrap_or_else(|_| format!("Unavailable agent ({})", agent.0)),
                    Some(&agent.0),
                ),
                Scope::Agent(_) => unreachable!(),
            };
            json!({"scope": row.scope.label(), "title": title, "agentId": agent})
        })
        .collect();
    Ok(
        json!({"projectId": project.0, "items": items.iter().map(|i| item_dto(i, store)).collect::<Vec<_>>(),
        "scopes": scopes, "proposals": proposals, "warnings": warnings, "responses": [], "paused": false}),
    )
}

/// This app action is the user's explicit instruction to share an item with the project.
pub fn create(
    memory: &MemoryStore,
    store: &Store,
    id: &str,
    item_id: &str,
    kind: ItemKind,
    content: &str,
) -> Result<(MemoryItem, Option<String>), String> {
    let project = project(store, id)?;
    let content = content.trim();
    if content.is_empty() {
        return Err("Enter memory text".into());
    }
    memory
        .create_user_item(item_id, &Scope::Project(project), kind, content)
        .map_err(failure)
}

#[allow(clippy::too_many_arguments)]
pub fn mutate(
    memory: &MemoryStore,
    store: &Store,
    project_id: &str,
    id: &str,
    expected: u64,
    action: &str,
    content: Option<&str>,
    target: Option<&str>,
) -> Result<MemoryItem, String> {
    let project = project(store, project_id)?;
    let item = memory.read_item(id).map_err(failure)?;
    if !includes(&item.scope, &project) {
        return Err("This item is outside this project.".into());
    }
    if action == "move" {
        let choices = domain_choices(memory, store)?;
        if !choices
            .iter()
            .any(|row| includes(&row.scope, &project) && Some(row.scope.label().as_str()) == target)
        {
            return Err("Choose a domain in this project or global memory.".into());
        }
    }
    // Undo can restore an earlier scope. Validate it before invoking the shared mutation.
    if action == "undo"
        && item
            .history
            .last()
            .is_some_and(|r| !includes(&r.scope, &project))
    {
        return Err(
            "This revision belongs to another domain. Open Settings → Memory to restore it.".into(),
        );
    }
    memory::mutate(memory, store, id, expected, action, content, target, None)
}

pub fn resolve(
    memory: &MemoryStore,
    store: &Store,
    project_id: &str,
    id: &str,
    approve: bool,
) -> Result<MemoryResolution, String> {
    let project = project(store, project_id)?;
    let proposal = memory
        .pending_proposals()
        .map_err(failure)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("This proposal is already resolved. Reload memory.")?;
    let source = store
        .load_channel(&proposal.source_conversation)
        .map_err(failure)?;
    if source.project_id.as_ref() != Some(&project) {
        return Err("This proposal belongs to another project.".into());
    }
    if !includes(&proposal_scope(memory, &proposal.action)?, &project) {
        return Err("This proposal targets memory outside this project. Open Settings → Memory to manage it.".into());
    }
    memory::resolve_proposal(memory, store, id, approve, None)
}
