use discard_core::{
    memory::{MemoryStore, Scope},
    memory_items::{MemoryChange, MemoryItem, MutationOrigin, ProposalAction},
    model::ChannelId,
    store::Store,
};
use serde_json::{Value, json};

fn failure(error: impl std::fmt::Display) -> String {
    format!(
        "{error}. Reload memory before trying again. Use Import file edits and reload if you edited the item file."
    )
}
fn conversation(store: &Store, id: &str) -> Result<discard_core::model::Channel, String> {
    if id.is_empty() {
        return Err("Choose a conversation first".into());
    }
    store
        .load_channel(&ChannelId(id.into()))
        .map_err(|_| "Source conversation is unavailable".into())
}
fn domain_choices(
    memory: &MemoryStore,
    store: &Store,
) -> Result<Vec<discard_core::memory_page::ScopeSummary>, String> {
    Ok(discard_core::memory_page::summaries(
        memory,
        &store
            .all_agents()
            .map_err(failure)?
            .into_iter()
            .map(|(id, config, _)| (id, config))
            .collect::<Vec<_>>(),
        &store.list_projects().map_err(failure)?,
    ))
}
fn item_dto(item: &MemoryItem, store: &Store) -> Value {
    json!({ "id":item.id, "scope":item.scope.label(), "kind":item.kind, "content":item.content,
        "version":item.version, "createdAt":item.created_at, "updatedAt":item.updated_at, "deleted":item.deleted,
        "sourceConversation":item.source_conversation.as_ref().map(|c| &c.0),
        "sourceAvailable":item.source_conversation.as_ref().is_some_and(|c| store.load_channel(c).is_ok()),
        "history":item.history.iter().map(|h| json!({"version":h.version,"scope":h.scope.label(),"content":h.content,"deleted":h.deleted,"updatedAt":h.updated_at,"origin":h.origin})).collect::<Vec<_>>() })
}
/// Reconcile durable receipts, and render truthful cards even while SQL writes fail.
fn response_records(
    memory: &MemoryStore,
    store: &Store,
    channel: &ChannelId,
) -> Result<
    (
        Vec<(discard_core::model::MessageId, MemoryChange)>,
        Vec<String>,
    ),
    String,
> {
    let approvals = memory.approved_changes(channel).map_err(failure)?;
    let warnings=store.reconcile_response_memory(channel,&approvals).err().map(|e|format!("Memory is approved and saved. Its response notice could not be saved. Reload memory to retry: {e}")).into_iter().collect();
    let mut records = store.response_memory(channel).map_err(failure)?;
    for approval in approvals {
        let MemoryChange::Saved { key, item } = &approval else {
            continue;
        };
        let Some(proposal_id) = &item.proposal_id else {
            continue;
        };
        if records
            .iter()
            .any(|(_, c)| matches!(c,MemoryChange::Saved{key:existing,..} if existing==key))
        {
            continue;
        }
        if let Some((response, _)) = records
            .iter()
            .find(|(_, c)| matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id))
        {
            records.push((response.clone(), approval));
        }
    }
    Ok((records, warnings))
}

/// Read context from the saved conversation. Sidebar selection is never used.
pub fn memory_view(
    memory: &MemoryStore,
    store: &Store,
    channel_id: Option<&str>,
) -> Result<Value, String> {
    let choices = domain_choices(memory, store)?;
    let channel = channel_id.map(|id| conversation(store, id)).transpose()?;
    let scopes = channel
        .as_ref()
        .map(|c| Scope::applicable(&c.agent_id, c.project_id.as_ref()))
        .unwrap_or_else(|| choices.iter().map(|r| r.scope.clone()).collect());
    let items = memory.list_items(&scopes).map_err(failure)?;
    let pending = memory.pending_proposals().map_err(failure)?;
    let proposals: Vec<_> = pending.iter().filter(|p| channel.as_ref().is_none_or(|c| c.id==p.source_conversation)).map(|p| {
        let (scope, content, action) = match &p.action {
            ProposalAction::Create {scope,content,..} => (scope.label(),content.clone(),"Create"),
            ProposalAction::Update {item_id,content,..} => (memory.read_item(item_id).map(|i| i.scope.label()).unwrap_or_default(),content.clone(),"Update"),
            ProposalAction::Forget {item_id,..} => memory.read_item(item_id).map(|i|(i.scope.label(),i.content,"Forget")).unwrap_or_default(),
        };
        json!({"id":p.id,"scope":scope,"content":content,"action":action,"sourceConversation":p.source_conversation.0})
    }).collect();
    let mut responses = Vec::new();
    let mut warnings = Vec::new();
    if let Some(channel) = &channel {
        let (records, problems) = response_records(memory, store, &channel.id)?;
        warnings.extend(problems);
        for (response, change) in records {
            match change {
                MemoryChange::Saved {key,item} if item.origin != MutationOrigin::User => responses.push(json!({"key":key,"responseId":response.0,"type":if item.origin==MutationOrigin::Automatic {"automatic"} else {"approved"},"item":item_dto(&item,store)})),
                MemoryChange::Proposed(p) => responses.push(json!({"key":p.id,"responseId":response.0,"type":"proposal","proposalId":p.id,"pending":pending.iter().any(|current|current.id==p.id)})),
                _=>{},
            }
        }
    }
    if channel.is_none() {
        for conversation in store.all_conversations().map_err(failure)? {
            warnings.extend(response_records(memory, store, &conversation.id)?.1);
        }
    }
    Ok(
        json!({"warnings":warnings,"agentId":channel.as_ref().map(|c| &c.agent_id.0),"projectId":channel.as_ref().and_then(|c|c.project_id.as_ref().map(|p|&p.0)),"items":items.iter().map(|i|item_dto(i,store)).collect::<Vec<_>>(),"scopes":choices.iter().map(|r|json!({"scope":r.scope.label(),"title":format!("{} · {}",r.scope.label(),r.title)})).collect::<Vec<_>>(),"proposals":proposals,"responses":responses,"paused":channel.as_ref().map(|c|memory.capture_paused(&c.id)).transpose().map_err(failure)?.unwrap_or(false)}),
    )
}
/// Explicit app actions authorize shared domain choices. Model JSON never calls this API.
#[allow(clippy::too_many_arguments)]
pub fn mutate(
    memory: &MemoryStore,
    store: &Store,
    id: &str,
    expected: u64,
    action: &str,
    content: Option<&str>,
    target: Option<&str>,
    requester: Option<&str>,
) -> Result<MemoryItem, String> {
    let channel = requester.map(|id| conversation(store, id)).transpose()?;
    let item = memory.read_item(id).map_err(failure)?;
    if let Some(c) = &channel
        && !Scope::applicable(&c.agent_id, c.project_id.as_ref()).contains(&item.scope)
    {
        return Err(
            "This item is outside this conversation. Open Settings → Memory to manage it.".into(),
        );
    }
    match action {
        "edit" => memory.update_item(
            id,
            expected,
            content.ok_or("Enter memory text")?,
            MutationOrigin::User,
        ),
        "forget" => memory.forget_item_in(id, expected, channel.as_ref().map(|c| &c.id)),
        "undo" => memory.undo_item(id, expected),
        "move" => {
            let scope = domain_choices(memory, store)?
                .into_iter()
                .find(|r| Some(r.scope.label().as_str()) == target)
                .ok_or("Choose an available domain")?
                .scope;
            memory.move_item(id, expected, &scope)
        }
        _ => return Err("Unknown memory action".into()),
    }
    .map_err(failure)
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryResolution {
    pub status: &'static str,
    pub committed: bool,
    pub association_pending: bool,
    pub warning: Option<String>,
}

/// Hold the agent Store lock at command entry through approval, so lifecycle changes cannot race it.
pub fn resolve_proposal(
    memory: &MemoryStore,
    store: &Store,
    id: &str,
    approve: bool,
    requester: Option<&str>,
) -> Result<MemoryResolution, String> {
    let proposal = memory
        .pending_proposals()
        .map_err(failure)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("This proposal is already resolved. Reload memory.")?;
    if let Some(requester) = requester
        && conversation(store, requester)?.id != proposal.source_conversation
    {
        return Err("Proposal belongs to another conversation".into());
    }
    if !approve {
        memory.reject_proposal(id).map_err(failure)?;
        return Ok(MemoryResolution {
            status: "rejected",
            committed: false,
            association_pending: false,
            warning: None,
        });
    }
    let source = conversation(store, &proposal.source_conversation.0)?;
    if source.agent_id != proposal.agent || store.agent_archived(&proposal.agent).unwrap_or(true) {
        return Err("The proposing agent is unavailable or archived. Reject this proposal or restore the agent.".into());
    }
    let scopes = Scope::applicable(&source.agent_id, source.project_id.as_ref());
    let scope = match &proposal.action {
        ProposalAction::Create { scope, .. } => scope.clone(),
        ProposalAction::Update { item_id, .. } | ProposalAction::Forget { item_id, .. } => {
            memory.read_item(item_id).map_err(failure)?.scope
        }
    };
    if !scopes.contains(&scope) {
        return Err("The proposal no longer matches its source conversation context".into());
    }
    let mut warning = None;
    // An item commit can precede an index/proposal-file error. Its atomic receipt
    // still proves that approval happened, so never report that as an uncommitted write.
    let item = match memory.approve_proposal(id) {
        Ok(item) => item,
        Err(error) => {
            let committed = memory
                .approved_changes(&source.id)
                .map_err(failure)?
                .into_iter()
                .find_map(|c| match c {
                    MemoryChange::Saved { item, .. } if item.proposal_id.as_deref() == Some(id) => {
                        Some(item)
                    }
                    _ => None,
                });
            let Some(item) = committed else {
                return Err(failure(error));
            };
            warning = Some(format!(
                "Memory is approved and saved. Reload memory to retry its remaining records: {error}"
            ));
            item
        }
    };
    let key = format!("{}:{}", item.id, item.version);
    let approvals = memory.approved_changes(&source.id);
    let linked = match approvals {
        Ok(approvals) => match store.reconcile_response_memory(&source.id, &approvals) {
            Ok(_) => match store.response_memory(&source.id) {
                Ok(records) => records.iter().any(
                    |(_, c)| matches!(c,MemoryChange::Saved{key:existing,..} if existing==&key),
                ),
                Err(error) => {
                    warning = Some(format!(
                        "Memory is approved and saved. Reload memory to retry its response notice: {error}"
                    ));
                    false
                }
            },
            Err(error) => {
                warning = Some(format!(
                    "Memory is approved and saved. Its response notice could not be saved. Reload memory to retry: {error}"
                ));
                false
            }
        },
        Err(error) => {
            warning = Some(format!(
                "Memory is approved and saved. Reload memory to retry its response notice: {error}"
            ));
            false
        }
    };
    Ok(MemoryResolution {
        status: "approved",
        committed: true,
        association_pending: !linked,
        warning,
    })
}

pub fn resume(memory: &MemoryStore, store: &Store, channel: &str) -> Result<(), String> {
    let channel = conversation(store, channel)?;
    memory.resume_capture(&channel.id).map_err(failure)
}
#[tauri::command]
pub fn memory_get(
    context: tauri::State<'_, super::SettingsContext>,
    channel_id: Option<String>,
) -> Result<Value, String> {
    memory_view(
        &context.memory,
        &context.store.lock().unwrap_or_else(|e| e.into_inner()),
        channel_id.as_deref(),
    )
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn memory_mutate(
    app: tauri::AppHandle,
    context: tauri::State<'_, super::SettingsContext>,
    id: String,
    expected_version: u64,
    action: String,
    content: Option<String>,
    scope: Option<String>,
    channel_id: Option<String>,
) -> Result<(), String> {
    use tauri::Emitter;
    mutate(
        &context.memory,
        &context.store.lock().unwrap_or_else(|e| e.into_inner()),
        &id,
        expected_version,
        &action,
        content.as_deref(),
        scope.as_deref(),
        channel_id.as_deref(),
    )?;
    let _ = app.emit("discard://memory", ());
    Ok(())
}
#[tauri::command]
pub fn memory_resolve(
    app: tauri::AppHandle,
    context: tauri::State<'_, super::SettingsContext>,
    id: String,
    approve: bool,
    channel_id: Option<String>,
) -> Result<MemoryResolution, String> {
    use tauri::Emitter;
    let outcome = resolve_proposal(
        &context.memory,
        &context.store.lock().unwrap_or_else(|e| e.into_inner()),
        &id,
        approve,
        channel_id.as_deref(),
    )?;
    let _ = app.emit("discard://memory", ());
    Ok(outcome)
}
#[tauri::command]
pub fn memory_resume(
    app: tauri::AppHandle,
    context: tauri::State<'_, super::SettingsContext>,
    channel_id: String,
) -> Result<(), String> {
    use tauri::Emitter;
    resume(
        &context.memory,
        &context.store.lock().unwrap_or_else(|e| e.into_inner()),
        &channel_id,
    )?;
    let _ = app.emit("discard://memory", ());
    Ok(())
}
#[tauri::command]
pub fn memory_reconcile(
    app: tauri::AppHandle,
    context: tauri::State<'_, super::SettingsContext>,
) -> Result<(), String> {
    use tauri::Emitter;
    context.memory.reconcile().map_err(failure)?;
    let _ = app.emit("discard://memory", ());
    Ok(())
}
#[tauri::command]
pub fn memory_reveal_item(
    context: tauri::State<'_, super::SettingsContext>,
    id: String,
) -> Result<(), String> {
    context.memory.read_item(&id).map_err(failure)?;
    super::reveal_path(
        &context
            .home
            .join("memory")
            .join(".items")
            .join(format!("{id}.md")),
    )
}

#[tauri::command]
pub async fn memory_fresh_conversation(
    app: tauri::AppHandle,
    context: tauri::State<'_, super::SettingsContext>,
    channels: tauri::State<'_, std::sync::Arc<super::super::channels::Channels>>,
    channel_id: String,
) -> Result<String, String> {
    use tauri::Emitter;
    let source = conversation(
        &context.store.lock().unwrap_or_else(|e| e.into_inner()),
        &channel_id,
    )?;
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        channels.create_conversation(
            &source.agent_id,
            source.project_id.as_ref(),
            "Fresh conversation",
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = app.emit("discard://conversations", ());
    if result.is_ok() {
        super::super::windows::show_conversation(&app);
    }
    result
}
