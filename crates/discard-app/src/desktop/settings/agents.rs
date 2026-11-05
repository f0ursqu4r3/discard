#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{agent::AgentConfig, agent_edit::AgentDraft, capability::{ApprovalPolicy, Capabilities, CapabilityMatrix}, model::AgentId, store::Store};
    #[test]
    fn loosening_policy_requires_confirmation() {
        let store = Store::open_in_memory().unwrap();
        let id = AgentId::new();
        let cfg = AgentConfig { name: "A".into(), avatar: None, provider: "p".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::DenyAll, plugin_write: false };
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
    fn into_core(self, current: &discard_core::agent::AgentConfig) -> Result<AgentDraft, String> {
        let policy = match self.policy.as_str() { "ask" => ApprovalPolicy::Ask, "deny" => ApprovalPolicy::DenyAll, _ => return Err("Unknown policy".into()) };
        Ok(AgentDraft { name:self.name.unwrap_or_else(||current.name.clone()), avatar:self.avatar.unwrap_or_else(||current.avatar.clone()), provider:self.provider.unwrap_or_else(||current.provider.clone()), model: if self.model.is_empty() { None } else { Some(self.model) }, system_prompt: self.system_prompt, policy, plugin_write: self.plugin_write })
    }
}
fn policy_name(policy: ApprovalPolicy) -> &'static str { match policy { ApprovalPolicy::Ask => "ask", ApprovalPolicy::DenyAll => "deny" } }
#[tauri::command]
pub fn settings_agent_get(context: tauri::State<'_, super::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let rows = store.list_agents().map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(id, cfg)| serde_json::json!({"id": id.0, "name": cfg.name, "provider": cfg.provider, "model": cfg.model.unwrap_or_default(), "systemPrompt": cfg.system_prompt, "policy": policy_name(cfg.policy), "pluginWrite": cfg.plugin_write})).collect())
}
#[tauri::command]
pub fn settings_agent_review(context: tauri::State<'_, super::SettingsContext>, id: String, draft: Draft) -> Result<serde_json::Value, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = store.load_agent(&AgentId(id)).map_err(|e| e.to_string())?;
    let provider = context.providers.iter().find(|p| p.id == cfg.provider).ok_or("Provider is unavailable")?;
    let draft = draft.into_core(&cfg)?;
    let review = agent_edit::review(&cfg, &draft, &provider.matrix);
    let choices = agent_edit::policy_choices(&provider.matrix);
    Ok(serde_json::json!({"changes": review.changes.into_iter().map(|c| serde_json::json!({"field": c.field, "before": c.before, "after": c.after})).collect::<Vec<_>>(), "needsConfirm": review.needs_confirm, "blocked": review.blocked, "choices": choices.into_iter().map(|c| serde_json::json!({"policy": policy_name(c.policy), "enabled": c.enabled, "reason": c.reason})).collect::<Vec<_>>() }))
}
#[tauri::command]
pub fn settings_agent_save(context: tauri::State<'_, super::SettingsContext>, id: String, draft: Draft, confirmed: bool) -> Result<(), String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let id = AgentId(id);
    let cfg = store.load_agent(&id).map_err(|e| e.to_string())?;
    let provider = context.providers.iter().find(|p| p.id == cfg.provider).ok_or("Provider is unavailable")?;
    save_agent(&store, &id, &draft.into_core(&cfg)?, &provider.matrix, confirmed)
}
