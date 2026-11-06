#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{agent::AgentConfig, agent_edit::AgentDraft, capability::{ApprovalPolicy, Capabilities, CapabilityMatrix}, model::AgentId, store::Store};
    #[test]
    fn loosening_policy_requires_confirmation() {
        let store = Store::open_in_memory().unwrap();
        let id = AgentId::new();
        let cfg = AgentConfig { name: "A".into(), avatar: None, provider: "p".into(), model: Some("m".into()), system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
        store.insert_agent(&id, &cfg).unwrap();
        let matrix = CapabilityMatrix { provider: "p".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        let mut draft = AgentDraft::from(&cfg); draft.policy = ApprovalPolicy::Ask;
        assert_eq!(save_agent(&store, &id, &draft, &matrix, false).unwrap_err(), "Confirmation is required");
        assert_eq!(store.load_agent(&id).unwrap(), cfg);
    }
    #[test]
    fn omitted_avatar_is_preserved_and_explicit_null_clears_it() {
        let current=AgentConfig { name:"A".into(),avatar:Some("avatar.png".into()),provider:"p".into(),model:Some("m".into()),system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false };
        let mut value=serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","pluginWrite":false});
        let draft:Draft=serde_json::from_value(value.clone()).unwrap();
        assert_eq!(draft.into_core(&current).unwrap().avatar,current.avatar);
        value["avatar"]=serde_json::Value::Null;
        let draft:Draft=serde_json::from_value(value).unwrap();
        assert_eq!(draft.into_core(&current).unwrap().avatar,None);
    }

    #[test]
    fn defaults_generate_editable_names_and_selected_provider_uses_the_draft_matrix() {
        let defaults = settings_agent_defaults();
        assert!(!defaults["name"].as_str().unwrap().trim().is_empty());
        assert_eq!(defaults["model"], "");
        assert_eq!(defaults["systemPrompt"], "");
        let cfg = AgentConfig { name: "A".into(), avatar: Some("keep.png".into()), provider: "old".into(), model: Some("old-model".into()), system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
        let value = serde_json::json!({"name":"Human name", "provider":"new", "model":"provider/exact-ID:1", "systemPrompt":"", "policy":"deny", "pluginWrite":false});
        let draft = serde_json::from_value::<Draft>(value).unwrap().into_core(&cfg).unwrap();
        let matrix = CapabilityMatrix { provider: "new".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        let provider = crate::desktop::actions::ProviderInfo { provider: std::sync::Arc::new(discard_providers::claude::ClaudeProvider::new("unused")), id: "new".into(), matrix, setup: std::sync::Arc::new(std::sync::Mutex::new(Some(discard_core::provider::SetupStatus::Ready))) };
        let providers = [provider];
        let chosen = selected_provider(&providers, &draft).unwrap();
        require_connected(chosen).unwrap();
        let store = Store::open_in_memory().unwrap(); let id = AgentId::new(); store.insert_agent(&id, &cfg).unwrap();
        save_agent(&store, &id, &draft, &chosen.matrix, false).unwrap();
        let saved = store.load_agent(&id).unwrap();
        assert_eq!(saved.provider, "new"); assert_eq!(saved.model.as_deref(), Some("provider/exact-ID:1")); assert_eq!(saved.avatar, cfg.avatar);
        *providers[0].setup.lock().unwrap() = None;
        assert!(require_connected(&providers[0]).is_err());
    }

}

use discard_core::{agent_edit::{self, AgentDraft}, capability::{ApprovalPolicy, CapabilityMatrix}, model::AgentId, store::Store};

/// Save an agent through the core validation and approval checks.
pub fn save_agent(store: &Store, id: &AgentId, draft: &AgentDraft, matrix: &CapabilityMatrix, confirmed: bool) -> Result<(), String> {
    agent_edit::save(store, id, draft, matrix, confirmed).map(|_| ()).map_err(|e| e.to_string())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft { name: Option<String>, #[serde(default, deserialize_with = "deserialize_avatar")] avatar: Option<Option<String>>, provider: Option<String>, model: String, system_prompt: String, policy: String, plugin_write: bool }
// Preserve an omitted avatar in existing forms. Explicit null clears it.
fn deserialize_avatar<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    <Option<String> as serde::Deserialize>::deserialize(deserializer).map(Some)
}
impl Draft {
    pub(crate) fn into_core(self, current: &discard_core::agent::AgentConfig) -> Result<AgentDraft, String> {
        let policy = match self.policy.as_str() { "ask" => ApprovalPolicy::Ask, "deny" => ApprovalPolicy::DenyAll, _ => return Err("Unknown policy".into()) };
        Ok(AgentDraft { name:self.name.unwrap_or_else(||current.name.clone()), avatar:self.avatar.unwrap_or_else(||current.avatar.clone()), provider:self.provider.unwrap_or_else(||current.provider.clone()), model: if self.model.is_empty() { None } else { Some(self.model) }, system_prompt: self.system_prompt, policy, plugin_write: self.plugin_write })
    }
}
fn selected_provider<'a>(providers: &'a [crate::desktop::actions::ProviderInfo], draft: &AgentDraft) -> Result<&'a crate::desktop::actions::ProviderInfo, String> {
    providers.iter().find(|p| p.id == draft.provider).ok_or_else(|| "Choose an available provider.".into())
}
fn policy_name(policy: ApprovalPolicy) -> &'static str { match policy { ApprovalPolicy::Ask => "ask", ApprovalPolicy::DenyAll => "deny" } }
#[tauri::command]
pub fn settings_agent_get(context: tauri::State<'_, super::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let rows = store.all_agents().map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(id, cfg, archived)| serde_json::json!({"id": id.0, "name": cfg.name, "avatar": cfg.avatar, "archived": archived, "provider": cfg.provider, "model": cfg.model.unwrap_or_default(), "systemPrompt": cfg.system_prompt, "policy": policy_name(cfg.policy), "pluginWrite": cfg.plugin_write})).collect())
}
#[tauri::command]
pub fn settings_agent_review(context: tauri::State<'_, super::SettingsContext>, id: String, draft: Draft) -> Result<serde_json::Value, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = store.load_agent(&AgentId(id)).map_err(|e| e.to_string())?;
    let draft = draft.into_core(&cfg)?;
    let provider = selected_provider(&context.providers, &draft)?;
    let review = agent_edit::review(&cfg, &draft, &provider.matrix);
    let choices = agent_edit::policy_choices(&provider.matrix);
    Ok(serde_json::json!({"changes": review.changes.into_iter().map(|c| serde_json::json!({"field": c.field, "before": c.before, "after": c.after})).collect::<Vec<_>>(), "needsConfirm": review.needs_confirm, "blocked": review.blocked, "choices": choices.into_iter().map(|c| serde_json::json!({"policy": policy_name(c.policy), "enabled": c.enabled, "reason": c.reason})).collect::<Vec<_>>() }))
}
#[tauri::command]
pub fn settings_agent_save(app: tauri::AppHandle, context: tauri::State<'_, super::SettingsContext>, id: String, draft: Draft, confirmed: bool) -> Result<(), String> {
    let (cfg, archived) = {
        let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
        let key = AgentId(id.clone());
        let cfg = store.load_agent(&key).map_err(|e| e.to_string())?;
        let draft = draft.into_core(&cfg)?;
        let provider = selected_provider(&context.providers, &draft)?;
        if draft.provider != cfg.provider { require_connected(provider)?; }
        save_agent(&store, &key, &draft, &provider.matrix, confirmed)?;
        (store.load_agent(&key).map_err(|e| e.to_string())?, store.agent_archived(&key).map_err(|e| e.to_string())?)
    };
    context.core.refresh_agent(&id, &cfg, archived);
    agents_changed(&app);
    Ok(())
}
fn agents_changed(app: &tauri::AppHandle) { use tauri::Emitter; let _ = app.emit("discard://agents", ()); }
fn require_connected(provider: &crate::desktop::actions::ProviderInfo) -> Result<(), String> {
    if !matches!(*provider.setup.lock().unwrap_or_else(|e| e.into_inner()), Some(discard_core::provider::SetupStatus::Ready { .. })) {
        return Err("Connect this provider in CLI connections first.".into());
    }
    Ok(())
}
#[tauri::command]
pub fn settings_agent_defaults() -> serde_json::Value {
    serde_json::json!({"name": discard_core::agent::generated_name(), "avatar": null, "provider": "", "model": "", "systemPrompt": "", "policy": "deny", "pluginWrite": false})
}
#[tauri::command]
pub fn settings_agent_create(app: tauri::AppHandle, context: tauri::State<'_, super::SettingsContext>, draft: Draft) -> Result<String, String> {
    let blank = discard_core::agent::AgentConfig { name: discard_core::agent::generated_name(), avatar: None, provider: String::new(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
    let draft = draft.into_core(&blank)?;
    if draft.plugin_write { return Err("Create the agent first. Then review plugin.write permission.".into()); }
    let provider = selected_provider(&context.providers, &draft)?;
    require_connected(provider)?;
    provider.matrix.enforcement_for(draft.policy).map_err(|e| e.to_string())?;
    let cfg = discard_core::agent::AgentConfig { name: draft.name, avatar: draft.avatar, provider: draft.provider, model: draft.model, system_prompt: draft.system_prompt, policy: draft.policy, plugin_write: false };
    let id = context.store.lock().unwrap_or_else(|e| e.into_inner()).create_agent(&cfg).map_err(|e| e.to_string())?;
    agents_changed(&app); context.core.changed.notify_one(); Ok(id.0)
}
#[tauri::command]
pub fn settings_agent_archive(app: tauri::AppHandle, channels: tauri::State<'_, std::sync::Arc<crate::desktop::channels::Channels>>, context: tauri::State<'_, super::SettingsContext>, id: String, archived: bool) -> Result<(), String> {
    context.core.begin_agent_change(&id)?;
    let result = (|| {
        let cfg = {
            let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
            store.set_agent_archived(&AgentId(id.clone()), archived).map_err(|e| e.to_string())?;
            store.load_agent(&AgentId(id.clone())).map_err(|e| e.to_string())?
        };
        context.core.refresh_agent(&id, &cfg, archived);
        agents_changed(&app);
        // Keep unsaved reply blocks and the live runtime when it is still usable.
        if !archived && context.core.agent_sessions_closed(&id) { channels.reload_agent(&id)?; }
        Ok(())
    })();
    context.core.end_agent_change(&id); result
}
/// Check every checkout before deleting any records. Never remove a borrowed or dirty checkout.
pub fn clean_agent_worktrees(store: &Store, agent: &AgentId) -> Result<(), String> {
    use discard_core::worktree::{Worktree, WorktreeManager};
    let channels = store.conversations_for_agent(agent).map_err(|e| e.to_string())?;
    let all = store.all_conversations().map_err(|e| e.to_string())?;
    let mut prepared = Vec::new();
    for channel in channels {
        let Some(record) = store.worktree_for(&channel.id).map_err(|e| e.to_string())? else { continue };
        if !record.owned { return Err(format!("Keep conversations to preserve the user-owned checkout at {}.", record.path)); }
        for other in all.iter().filter(|c| c.id != channel.id) {
            if store.worktree_for(&other.id).map_err(|e| e.to_string())?.is_some_and(|r| r.path == record.path) {
                return Err(format!("Keep conversations to preserve the shared checkout at {}.", record.path));
            }
        }
        let manager = WorktreeManager::new(std::path::Path::new(&record.repo));
        if std::path::Path::new(&record.path).exists() {
            manager.recorded(std::path::Path::new(&record.repo), std::path::Path::new(&record.path), &record.branch).map_err(|e| e.to_string())?;
        }
        let worktree = Worktree { root: record.path.into(), repo: record.repo.into(), branch: record.branch, base_commit: record.base_commit };
        let mut status = manager.status(&worktree).map_err(|e| e.to_string())?;
        status.unmerged_commits = 0; // Removal keeps the branch and its commits.
        if !status.is_clean() { return Err(format!("Save the work in {} before deleting conversations: {}", worktree.root.display(), status.describe().join(", "))); }
        prepared.push((manager, worktree));
    }
    for (manager, worktree) in prepared { manager.remove(&worktree, false).map_err(|e| e.to_string())?; }
    Ok(())
}
/// Application deletion combines transcript options with exhaustive private-memory cleanup.
pub fn delete_agent_data(store: &Store, memory: &discard_core::memory::MemoryStore, key: &AgentId, options: discard_core::store::DeleteAgentOptions) -> Result<(), String> {
    store.agent_archived(key).map_err(|e| e.to_string())?;
    if options.delete_conversations { clean_agent_worktrees(store, key)?; }
    if options.delete_private_memory { memory.delete_private_memory(key).map_err(|e| e.to_string())?; }
    store.delete_agent(key, options).map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn settings_agent_delete(app: tauri::AppHandle, context: tauri::State<'_, super::SettingsContext>, id: String, delete_conversations: bool, delete_private_memory: bool) -> Result<(), String> {
    let core = context.core.clone(); let store = context.store.clone(); let memory = context.memory.clone();
    core.begin_agent_change(&id)?;
    let guard_core = core.clone(); let guard_id = id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let key = AgentId(id.clone());
        let cfg = {
            let store = store.lock().unwrap_or_else(|e| e.into_inner());
            // Memory cleanup enumerates by agent ID, including orphan agent-project scopes.
            delete_agent_data(&store, &memory, &key, discard_core::store::DeleteAgentOptions { delete_conversations, delete_private_memory })?;
            store.load_agent(&key).map_err(|e| e.to_string())?
        };
        if delete_conversations { core.remove_agent_channels(&id); }
        else { core.refresh_agent(&id, &cfg, true); }
        agents_changed(&app); Ok(())
    }).await.map_err(|e| e.to_string()).and_then(|result| result);
    guard_core.end_agent_change(&guard_id); result
}
