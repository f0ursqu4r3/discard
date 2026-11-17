#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{agent::AgentConfig, agent_edit::AgentDraft, capability::{ApprovalPolicy, Capabilities, CapabilityMatrix}, model::AgentId, store::Store};
    #[test]
    fn loosening_policy_requires_confirmation() {
        let store = Store::open_in_memory().unwrap();
        let id = AgentId::new();
        let cfg = AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "p".into(), model: Some("m".into()), system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
        store.insert_agent(&id, &cfg).unwrap();
        let matrix = CapabilityMatrix { provider: "p".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        let mut draft = AgentDraft::from(&cfg); draft.policy = ApprovalPolicy::Ask;
        assert_eq!(save_agent(&store, &id, &draft, &matrix, false).unwrap_err(), "Confirmation is required");
        assert_eq!(store.load_agent(&id).unwrap(), cfg);
    }
    #[test]
    fn omitted_avatar_is_preserved_and_explicit_null_clears_it() {
        let current=AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name:"A".into(),avatar:Some("avatar.png".into()),provider:"p".into(),model:Some("m".into()),system_prompt:String::new(),policy:ApprovalPolicy::Ask,plugin_write:false };
        let mut value=serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":false,"pluginWrite":false});
        let draft:Draft=serde_json::from_value(value.clone()).unwrap();
        assert_eq!(draft.into_core(&current).unwrap().avatar,current.avatar);
        value["avatar"]=serde_json::Value::Null;
        let draft:Draft=serde_json::from_value(value).unwrap();
        assert_eq!(draft.into_core(&current).unwrap().avatar,None);
    }

    #[test]
    fn reasoning_drafts_preserve_omitted_values_and_clear_explicit_null() {
        let current = discard_core::agent::AgentConfig::from_toml("name='A'\nprovider='codex'\nmodel='m'\npolicy='ask'\nreasoning_effort='high'\n").unwrap();
        let mut value = serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":false,"pluginWrite":false});
        let parse = |value| serde_json::from_value::<Draft>(value).unwrap().into_core(&current).unwrap();
        assert_eq!(parse(value.clone()).reasoning_effort.as_deref(), Some("high"));
        value["reasoningEffort"] = serde_json::json!("low");
        assert_eq!(parse(value.clone()).reasoning_effort.as_deref(), Some("low"));
        value["reasoningEffort"] = serde_json::Value::Null;
        assert_eq!(parse(value).reasoning_effort, None);
        assert_eq!(settings_agent_defaults()["reasoningEffort"], serde_json::Value::Null);
    }

    #[test]
    fn defaults_generate_editable_names_and_selected_provider_uses_the_draft_matrix() {
        let defaults = settings_agent_defaults();
        assert!(!defaults["name"].as_str().unwrap().trim().is_empty());
        assert_eq!(defaults["model"], "");
        assert_eq!(defaults["systemPrompt"], "");
        let cfg = AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name: "A".into(), avatar: Some("keep.png".into()), provider: "old".into(), model: Some("old-model".into()), system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
        let value = serde_json::json!({"name":"Human name", "provider":"new", "model":"provider/exact-ID:1", "systemPrompt":"", "policy":"deny", "workspaceWrite":false,"pluginWrite":false});
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

    fn provider_info(id: &str, provider: discard_providers::fake::FakeProvider) -> crate::desktop::actions::ProviderInfo {
        let matrix = CapabilityMatrix { provider: id.into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        crate::desktop::actions::ProviderInfo { provider: std::sync::Arc::new(provider), id: id.into(), matrix, setup: std::sync::Arc::new(std::sync::Mutex::new(Some(discard_core::provider::SetupStatus::Ready))) }
    }

    #[test]
    fn agent_rows_report_autorun_support() {
        use discard_core::provider::SandboxAutorun;
        use discard_providers::fake::FakeProvider;
        let cfg = |provider: &str, autorun: bool| AgentConfig { workspace_write: true, sandbox_autorun: autorun, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: provider.into(), model: Some("m".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false };
        let providers = [
            provider_info("blocked", FakeProvider::new(vec![]).with_sandbox_autorun(SandboxAutorun::Unsupported("x".into()))),
            provider_info("ready", FakeProvider::new(vec![]).with_sandbox_autorun(SandboxAutorun::Supported)),
        ];
        let rows = agent_rows(vec![(AgentId::new(), cfg("blocked", false), false), (AgentId::new(), cfg("ready", true), false), (AgentId::new(), cfg("gone", false), false)], &providers);
        assert_eq!(rows[0]["sandboxAutorun"], false);
        assert_eq!(rows[0]["sandboxAutorunUnavailable"], "x");
        assert_eq!(rows[1]["sandboxAutorun"], true);
        assert_eq!(rows[1]["sandboxAutorunUnavailable"], serde_json::Value::Null);
        assert_eq!(rows[2]["sandboxAutorunUnavailable"], "This provider is not available.");
    }

    #[test]
    fn create_rejects_autorun() {
        let draft = |extra: serde_json::Value| {
            let mut value = serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":false,"pluginWrite":false});
            value.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let blank = AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "p".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
            serde_json::from_value::<Draft>(value).unwrap().into_core(&blank).unwrap()
        };
        assert_eq!(check_create_draft(&draft(serde_json::json!({"sandboxAutorun": true}))).unwrap_err(), "Create the agent first. Then review Sandbox autorun permission.");
        check_create_draft(&draft(serde_json::json!({}))).unwrap();
    }

    #[test]
    fn the_delegation_setting_reads_drafts_saves_and_asks_for_confirmation_like_other_permissions() {
        let store = Store::open_in_memory().unwrap();
        let id = AgentId::new();
        let cfg = AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "p".into(), model: Some("m".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false };
        store.insert_agent(&id, &cfg).unwrap();
        let matrix = CapabilityMatrix { provider: "p".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        // Read: the settings row carries the value.
        assert_eq!(agent_rows(vec![(id.clone(), cfg.clone(), false)], &[])[0]["delegation"], false);
        // Draft: an omitted value keeps the saved one, an explicit value replaces it.
        let mut value = serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":false,"pluginWrite":false});
        let keep = serde_json::from_value::<Draft>(value.clone()).unwrap().into_core(&cfg).unwrap();
        assert!(!keep.delegation);
        value["delegation"] = serde_json::json!(true);
        let draft = serde_json::from_value::<Draft>(value.clone()).unwrap().into_core(&cfg).unwrap();
        assert!(draft.delegation);
        assert!(serde_json::from_value::<Draft>(serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":false,"pluginWrite":false})).unwrap().into_core(&AgentConfig { delegation: true, ..cfg.clone() }).unwrap().delegation, "omitted keeps an enabled value");
        // Review lists the change and asks to confirm turning it on.
        let review = discard_core::agent_edit::review(&cfg, &draft, &matrix);
        assert!(review.needs_confirm && review.changes.iter().any(|c| c.field == "Delegation" && c.before == "off" && c.after == "on"), "{:?}", review.changes);
        // Save: refused without confirmation, stored with it, and the row reads back on.
        assert_eq!(save_agent(&store, &id, &draft, &matrix, false).unwrap_err(), "Confirmation is required");
        assert!(!store.load_agent(&id).unwrap().delegation);
        save_agent(&store, &id, &draft, &matrix, true).unwrap();
        let saved = store.load_agent(&id).unwrap();
        assert!(saved.delegation);
        assert_eq!(agent_rows(vec![(id.clone(), saved.clone(), false)], &[])[0]["delegation"], true);
        // Turning it off needs no confirmation.
        value["delegation"] = serde_json::json!(false);
        let off = serde_json::from_value::<Draft>(value).unwrap().into_core(&saved).unwrap();
        save_agent(&store, &id, &off, &matrix, false).unwrap();
        assert!(!store.load_agent(&id).unwrap().delegation);
        // A new agent starts without it, like the other permissions.
        assert_eq!(check_create_draft(&draft).unwrap_err(), "Create the agent first. Then review Delegation permission.");
    }

    #[test]
    fn omitted_autorun_keeps_the_saved_value_and_explicit_values_replace_it() {
        let current = AgentConfig { workspace_write: true, sandbox_autorun: true, delegation: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "p".into(), model: Some("m".into()), system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false };
        let mut value = serde_json::json!({"model":"m","systemPrompt":"","policy":"ask","workspaceWrite":true,"pluginWrite":false});
        assert!(serde_json::from_value::<Draft>(value.clone()).unwrap().into_core(&current).unwrap().sandbox_autorun);
        value["sandboxAutorun"] = serde_json::json!(false);
        assert!(!serde_json::from_value::<Draft>(value).unwrap().into_core(&current).unwrap().sandbox_autorun);
    }

}

use discard_core::{agent_edit::{self, AgentDraft}, capability::{ApprovalPolicy, CapabilityMatrix}, model::AgentId, store::Store};

/// Save an agent through the core validation and approval checks.
pub fn save_agent(store: &Store, id: &AgentId, draft: &AgentDraft, matrix: &CapabilityMatrix, confirmed: bool) -> Result<(), String> {
    agent_edit::save(store, id, draft, matrix, confirmed).map(|_| ()).map_err(|e| e.to_string())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft { name: Option<String>, #[serde(default, deserialize_with = "deserialize_avatar")] avatar: Option<Option<String>>, provider: Option<String>, model: String, #[serde(default, deserialize_with = "deserialize_avatar")] reasoning_effort: Option<Option<String>>, system_prompt: String, policy: String, plugin_write: bool, workspace_write: Option<bool>, #[serde(default)] sandbox_autorun: Option<bool>, #[serde(default)] delegation: Option<bool> }
// Preserve an omitted avatar in existing forms. Explicit null clears it.
fn deserialize_avatar<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    <Option<String> as serde::Deserialize>::deserialize(deserializer).map(Some)
}
impl Draft {
    pub(crate) fn into_core(self, current: &discard_core::agent::AgentConfig) -> Result<AgentDraft, String> {
        let policy = match self.policy.as_str() { "ask" => ApprovalPolicy::Ask, "deny" => ApprovalPolicy::DenyAll, _ => return Err("Unknown policy".into()) };
        Ok(AgentDraft { reasoning_effort: self.reasoning_effort.unwrap_or_else(|| current.reasoning_effort.clone()), name:self.name.unwrap_or_else(||current.name.clone()), avatar:self.avatar.unwrap_or_else(||current.avatar.clone()), provider:self.provider.unwrap_or_else(||current.provider.clone()), model: if self.model.is_empty() { None } else { Some(self.model) }, system_prompt: self.system_prompt, policy, plugin_write: self.plugin_write, workspace_write: self.workspace_write.unwrap_or(current.workspace_write), sandbox_autorun: self.sandbox_autorun.unwrap_or(current.sandbox_autorun), delegation: self.delegation.unwrap_or(current.delegation) })
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
    Ok(agent_rows(rows, &context.providers))
}
/// Settings rows. `sandboxAutorunUnavailable` carries the reason autorun cannot run. It is a state block
/// (a failed provider check, an unsupported platform, or a missing provider), not planned work.
fn agent_rows(rows: Vec<(AgentId, discard_core::agent::AgentConfig, bool)>, providers: &[crate::desktop::actions::ProviderInfo]) -> Vec<serde_json::Value> {
    rows.into_iter().map(|(id, cfg, archived)| {
        let unavailable = match providers.iter().find(|p| p.id == cfg.provider) {
            Some(info) => match info.provider.sandbox_autorun() { discard_core::provider::SandboxAutorun::Supported => None, discard_core::provider::SandboxAutorun::Unsupported(reason) => Some(reason) },
            None => Some("This provider is not available.".to_string()),
        };
        serde_json::json!({"id": id.0, "name": cfg.name, "avatar": cfg.avatar, "archived": archived, "provider": cfg.provider, "model": cfg.model.unwrap_or_default(), "reasoningEffort": cfg.reasoning_effort, "systemPrompt": cfg.system_prompt, "policy": policy_name(cfg.policy), "workspaceWrite": cfg.workspace_write, "sandboxAutorun": cfg.sandbox_autorun, "delegation": cfg.delegation, "sandboxAutorunUnavailable": unavailable, "pluginWrite": cfg.plugin_write})
    }).collect()
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
pub async fn settings_agent_generate_personality(
    context: tauri::State<'_, super::SettingsContext>,
    provider: String, model: String, reasoning_effort: Option<String>,
    name: String, description: String, current: String,
) -> Result<String, String> {
    let info = context.providers.iter().find(|info| info.id == provider).ok_or("Unknown provider")?;
    let provider = info.provider.clone();
    generate_personality(provider.as_ref(), model, reasoning_effort, &name, &description, &current).await
}

async fn generate_personality(
    provider: &dyn discard_core::provider::Provider, model: String, reasoning_effort: Option<String>,
    name: &str, description: &str, current: &str,
) -> Result<String, String> {
    use discard_core::{personality, provider::TextRequest};
    if model.trim().is_empty() { return Err("Choose a model first.".into()); }
    let request = TextRequest {
        model, reasoning_effort,
        instructions: personality::instructions(),
        prompt: personality::request_prompt(name, description, current)?,
    };
    let output = provider.generate_text(request).await.map_err(|error| error.to_string())?;
    personality::clean(&output)
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
    if !matches!(*provider.setup.lock().unwrap_or_else(|e| e.into_inner()), Some(discard_core::provider::SetupStatus::Ready)) {
        return Err("Connect this provider in CLI connections first.".into());
    }
    Ok(())
}
#[tauri::command]
pub fn settings_agent_defaults() -> serde_json::Value {
    serde_json::json!({"name": discard_core::agent::generated_name(), "avatar": null, "provider": "", "model": "", "reasoningEffort": null, "systemPrompt": "", "policy": "deny", "pluginWrite": false})
}
/// New agents start without elevated permissions. The user reviews each one after creation.
fn check_create_draft(draft: &AgentDraft) -> Result<(), String> {
    if draft.workspace_write { return Err("Create the agent first. Then review Workspace writes permission.".into()); }
    if draft.sandbox_autorun { return Err("Create the agent first. Then review Sandbox autorun permission.".into()); }
    if draft.plugin_write { return Err("Create the agent first. Then review plugin.write permission.".into()); }
    if draft.delegation { return Err("Create the agent first. Then review Delegation permission.".into()); }
    Ok(())
}
#[tauri::command]
pub fn settings_agent_create(app: tauri::AppHandle, context: tauri::State<'_, super::SettingsContext>, draft: Draft) -> Result<String, String> {
    let blank = discard_core::agent::AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: None, name: discard_core::agent::generated_name(), avatar: None, provider: String::new(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
    let draft = draft.into_core(&blank)?;
    check_create_draft(&draft)?;
    let provider = selected_provider(&context.providers, &draft)?;
    require_connected(provider)?;
    provider.matrix.enforcement_for(draft.policy).map_err(|e| e.to_string())?;
    let cfg = discard_core::agent::AgentConfig { workspace_write: false, sandbox_autorun: false, delegation: false, reasoning_effort: draft.reasoning_effort, name: draft.name, avatar: draft.avatar, provider: draft.provider, model: draft.model, system_prompt: draft.system_prompt, policy: draft.policy, plugin_write: false };
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
pub async fn settings_agent_delete(app: tauri::AppHandle, channels: tauri::State<'_, std::sync::Arc<crate::desktop::channels::Channels>>, context: tauri::State<'_, super::SettingsContext>, id: String, delete_conversations: bool, delete_private_memory: bool) -> Result<(), String> {
    let core = context.core.clone(); let store = context.store.clone(); let memory = context.memory.clone(); let channels = channels.inner().clone();
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
        let reconcile = if !delete_conversations { channels.reconcile_deleted_agent(&id) } else { Ok(()) };
        agents_changed(&app);
        use tauri::Emitter;
        let _ = app.emit("discard://conversations", ());
        let _ = app.emit("discard://channel-details", core.active_channel_id(&core.focused_view()));
        reconcile
    }).await.map_err(|e| e.to_string()).and_then(|result| result);
    guard_core.end_agent_change(&guard_id); result
}

#[cfg(test)]
mod personality_tests {
    use super::generate_personality;
    use discard_core::personality;
    use discard_providers::fake::FakeProvider;

    #[tokio::test]
    async fn generation_uses_draft_values_and_cleans_the_result_without_a_session() {
        let provider = FakeProvider::new(vec![]).with_generated_text(Ok("```text\n\"You are concise.\"\n```".into()));
        let result = generate_personality(&provider, "draft-model".into(), Some("high".into()), "River", "brief", "You are patient.").await.unwrap();
        assert_eq!(result, "You are concise.");
        let requests = provider.probe.text_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, "draft-model");
        assert_eq!(requests[0].reasoning_effort.as_deref(), Some("high"));
        assert_eq!(requests[0].instructions, personality::instructions());
        assert_eq!(requests[0].prompt, personality::request_prompt("River", "brief", "You are patient.").unwrap());
        assert!(provider.probe.specs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_input_never_reaches_the_provider_and_generation_errors_are_returned() {
        let provider = FakeProvider::new(vec![]).with_generated_text(Err("generation failed".into()));
        assert_eq!(generate_personality(&provider, "  ".into(), None, "", "brief", "").await.unwrap_err(), "Choose a model first.");
        assert_eq!(generate_personality(&provider, "m".into(), None, "", "  ", "").await.unwrap_err(), "Describe the personality first.");
        assert!(provider.probe.text_requests.lock().unwrap().is_empty());
        assert!(generate_personality(&provider, "m".into(), None, "", "brief", "").await.unwrap_err().contains("generation failed"));
        let empty = FakeProvider::new(vec![]).with_generated_text(Ok("  ".into()));
        assert_eq!(generate_personality(&empty, "m".into(), None, "", "brief", "").await.unwrap_err(), "The model returned no text. Try again.");
    }
}
