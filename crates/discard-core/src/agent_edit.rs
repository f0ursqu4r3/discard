//! Review and save user-initiated agent changes.
use crate::agent::{AgentChange, AgentConfig, Initiator};
use crate::capability::{ApprovalPolicy, CapabilityMatrix};
use crate::model::AgentId;
use crate::store::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDraft { pub name: String, pub avatar: Option<String>, pub provider: String, pub model: Option<String>, pub system_prompt: String, pub policy: ApprovalPolicy, pub plugin_write: bool }
impl From<&AgentConfig> for AgentDraft {
    fn from(c: &AgentConfig) -> Self { Self { name:c.name.clone(), avatar:c.avatar.clone(), provider:c.provider.clone(), model: c.model.clone(), system_prompt: c.system_prompt.clone(), policy: c.policy, plugin_write: c.plugin_write } }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange { pub field: &'static str, pub before: String, pub after: String }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyChoice { pub policy: ApprovalPolicy, pub enabled: bool, pub reason: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReview { pub changes: Vec<FieldChange>, pub needs_confirm: bool, pub blocked: Option<String> }
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("No changes to save")]
    Unchanged,
    #[error("{0}")]
    Blocked(String),
    #[error("Confirmation is required")]
    NeedsConfirm,
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub fn policy_choices(matrix: &CapabilityMatrix) -> Vec<PolicyChoice> {
    [ApprovalPolicy::Ask, ApprovalPolicy::DenyAll].into_iter().map(|policy| {
        let reason = matrix.enforcement_for(policy).err().map(|e| e.to_string());
        PolicyChoice { policy, enabled: reason.is_none(), reason }
    }).collect()
}

pub fn review(current: &AgentConfig, draft: &AgentDraft, matrix: &CapabilityMatrix) -> AgentReview {
    let mut changes = Vec::new();
    let model = |m: &Option<String>| m.as_deref().unwrap_or("(default)").to_string();
    let policy = |p| match p { ApprovalPolicy::Ask => "Ask before tool calls", ApprovalPolicy::DenyAll => "Deny all" }.to_string();
    let flag = |b| if b { "on" } else { "off" }.to_string();
    if current.model != draft.model {
        changes.push(FieldChange { field: "Model", before: model(&current.model), after: model(&draft.model) });
    }
    let mut add = |field, before: String, after: String| { if before != after { changes.push(FieldChange { field, before, after }); } };
    add("Name",current.name.clone(),draft.name.clone());
    if current.avatar != draft.avatar { add("Avatar",format!("{:?}",current.avatar),format!("{:?}",draft.avatar)); }
    add("Provider",current.provider.clone(),draft.provider.clone());
    add("System prompt", current.system_prompt.clone(), draft.system_prompt.clone());
    add("Approval policy", policy(current.policy), policy(draft.policy));
    add("plugin.write", flag(current.plugin_write), flag(draft.plugin_write));
    let id = AgentId(String::new());
    let needs_confirm = (current.policy != draft.policy && AgentChange::SetPolicy { agent: id.clone(), from: current.policy, to: draft.policy }.needs_user_approval(&Initiator::User)) || (current.plugin_write != draft.plugin_write && AgentChange::SetPluginWrite { agent: id, enabled: draft.plugin_write }.needs_user_approval(&Initiator::User));
    let blocked = if draft.provider != matrix.provider {
        Some("Select the capability matrix for this provider.".into())
    } else {
        (current.policy != draft.policy || current.provider != draft.provider).then(|| matrix.enforcement_for(draft.policy).err().map(|e| e.to_string())).flatten()
    };
    AgentReview { changes, needs_confirm, blocked }
}

pub fn save(store: &Store, id: &AgentId, draft: &AgentDraft, matrix: &CapabilityMatrix, confirmed: bool) -> Result<AgentConfig, SaveError> {
    let mut cfg = store.load_agent(id)?;
    let result = review(&cfg, draft, matrix);
    if result.changes.is_empty() { return Err(SaveError::Unchanged); }
    if let Some(reason) = result.blocked { return Err(SaveError::Blocked(reason)); }
    if result.needs_confirm && !confirmed { return Err(SaveError::NeedsConfirm); }
    cfg.name=draft.name.clone(); cfg.avatar=draft.avatar.clone(); cfg.provider=draft.provider.clone();
    cfg.model = draft.model.clone(); cfg.system_prompt = draft.system_prompt.clone(); cfg.policy = draft.policy; cfg.plugin_write = draft.plugin_write;
    store.update_agent(id, &cfg)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capabilities, CapabilityMatrix};

    fn setup(policy: ApprovalPolicy) -> (Store, AgentId, AgentConfig, CapabilityMatrix) {
        let store = Store::open_in_memory().unwrap();
        let id = AgentId::new();
        let cfg = AgentConfig { name: "A".into(), avatar: None, provider: "p".into(), model: Some("old-model".into()), system_prompt: "old".into(), policy, plugin_write: false };
        store.insert_agent(&id, &cfg).unwrap();
        let matrix = CapabilityMatrix { provider: "p".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: true, session_resume: true, cancellation: true, pre_execution_blocking: true } };
        (store, id, cfg, matrix)
    }

    #[test]
    fn unchanged_draft_has_no_changes_and_save_says_unchanged() {
        let (store, id, cfg, matrix) = setup(ApprovalPolicy::Ask);
        let draft = AgentDraft::from(&cfg);
        assert!(review(&cfg, &draft, &matrix).changes.is_empty());
        assert!(matches!(save(&store, &id, &draft, &matrix, false), Err(SaveError::Unchanged)));
    }

    #[test]
    fn a_model_named_default_is_not_treated_as_absent() {
        let (store, id, cfg, matrix) = setup(ApprovalPolicy::Ask);
        let mut draft = AgentDraft::from(&cfg);
        draft.model = Some("(default)".into());
        assert_eq!(save(&store, &id, &draft, &matrix, false).unwrap().model, draft.model);
    }

    #[test]
    fn review_lists_changes_and_loosening_needs_confirmation() {
        let (store, id, cfg, matrix) = setup(ApprovalPolicy::DenyAll);
        let mut draft = AgentDraft::from(&cfg);
        draft.model = Some("model".into()); draft.system_prompt = "new".into(); draft.policy = ApprovalPolicy::Ask; draft.plugin_write = true;
        let result = review(&cfg, &draft, &matrix);
        assert_eq!(result.changes.iter().map(|c| c.field).collect::<Vec<_>>(), ["Model", "System prompt", "Approval policy", "plugin.write"]);
        assert!(result.needs_confirm);
        assert!(matches!(save(&store, &id, &draft, &matrix, false), Err(SaveError::NeedsConfirm)));
        assert_eq!(store.load_agent(&id).unwrap(), cfg);
        assert_eq!(save(&store, &id, &draft, &matrix, true).unwrap().policy, ApprovalPolicy::Ask);
    }

    #[test]
    fn tightening_does_not_need_confirmation_and_unenforceable_policy_is_blocked() {
        let (store, id, cfg, mut matrix) = setup(ApprovalPolicy::Ask);
        let mut draft = AgentDraft::from(&cfg); draft.policy = ApprovalPolicy::DenyAll;
        assert!(!review(&cfg, &draft, &matrix).needs_confirm);
        save(&store, &id, &draft, &matrix, false).unwrap();
        matrix.capabilities.interactive_approvals = false;
        draft.policy = ApprovalPolicy::Ask;
        assert!(matches!(save(&store, &id, &draft, &matrix, true), Err(SaveError::Blocked(_))));
        assert!(!policy_choices(&matrix)[0].enabled);
        assert!(matches!(save(&store, &AgentId::new(), &draft, &matrix, true), Err(SaveError::Store(_))));
    }

    #[test]
    fn plugin_write_on_requires_confirmation_and_off_does_not() {
        let (store, id, cfg, matrix) = setup(ApprovalPolicy::Ask);
        let mut draft = AgentDraft::from(&cfg);
        draft.plugin_write = true;
        assert!(review(&cfg, &draft, &matrix).needs_confirm);
        assert!(matches!(save(&store, &id, &draft, &matrix, false), Err(SaveError::NeedsConfirm)));
        let enabled = save(&store, &id, &draft, &matrix, true).unwrap();
        draft.plugin_write = false;
        assert!(!review(&enabled, &draft, &matrix).needs_confirm);
        assert!(!save(&store, &id, &draft, &matrix, false).unwrap().plugin_write);
    }

    #[test]
    fn model_and_prompt_edits_save_without_confirm() {
        let (store, id, cfg, matrix) = setup(ApprovalPolicy::Ask);
        let mut draft = AgentDraft::from(&cfg);
        draft.model = Some("model".into());
        draft.system_prompt = "prompt".into();
        let saved = save(&store, &id, &draft, &matrix, false).unwrap();
        assert_eq!(saved.model, draft.model);
        assert_eq!(saved.system_prompt, draft.system_prompt);
    }
    #[test]
    fn identity_and_personality_edits_preserve_agent_and_conversation_ids() {
        let (store,id,cfg,matrix)=setup(ApprovalPolicy::Ask);
        let direct=store.ensure_default_conversation(&id).unwrap();
        let mut draft=AgentDraft::from(&cfg); draft.name="Robin".into(); draft.avatar=Some("avatar.png".into()); draft.system_prompt="Be precise".into();
        let saved=save(&store,&id,&draft,&matrix,false).unwrap();
        assert_eq!(saved.name,"Robin"); assert_eq!(saved.avatar,draft.avatar); assert_eq!(saved.system_prompt,draft.system_prompt);
        assert_eq!(store.default_conversation(&id).unwrap().id,direct.id);
        draft.provider="other".into();
        assert!(matches!(save(&store,&id,&draft,&matrix,false),Err(SaveError::Blocked(_))));
    }

    #[test]
    fn user_cannot_clear_the_selected_model() {
        let (store,id,cfg,matrix)=setup(ApprovalPolicy::Ask);
        let mut draft=AgentDraft::from(&cfg); draft.model=None;
        assert!(matches!(save(&store,&id,&draft,&matrix,false),Err(SaveError::Store(StoreError::Invalid(_)))));
        assert_eq!(store.load_agent(&id).unwrap(),cfg);
    }

}
