//! Agent personas stored as TOML, and the approval rule for creating or loosening them.

use crate::capability::ApprovalPolicy;
use crate::model::AgentId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    pub policy: ApprovalPolicy,
}

impl AgentConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

/// Who asked for an agent change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initiator {
    User,
    Plugin(String),
    Agent(AgentId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentChange {
    Create { config: AgentConfig },
    SetPolicy { agent: AgentId, from: ApprovalPolicy, to: ApprovalPolicy },
}

/// Strictness rank. Higher is stricter.
fn strictness(p: ApprovalPolicy) -> u8 {
    match p {
        ApprovalPolicy::DenyAll => 2,
        ApprovalPolicy::Ask => 1,
    }
}

impl AgentChange {
    /// Rust-rendered user approval is needed unless the user made a change that does not loosen policy.
    /// A change started by a plugin or another agent always needs approval.
    pub fn needs_user_approval(&self, initiator: &Initiator) -> bool {
        match (initiator, self) {
            (Initiator::User, AgentChange::Create { .. }) => false,
            (Initiator::User, AgentChange::SetPolicy { from, to, .. }) => strictness(*to) < strictness(*from),
            (Initiator::Plugin(_) | Initiator::Agent(_), _) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(policy: ApprovalPolicy) -> AgentConfig {
        AgentConfig {
            name: "Scout".into(),
            avatar: None,
            provider: "claude".into(),
            model: Some("haiku".into()),
            system_prompt: "Be brief.".into(),
            policy,
        }
    }

    #[test]
    fn config_round_trips_through_toml() {
        let c = cfg(ApprovalPolicy::Ask);
        let text = c.to_toml().unwrap();
        assert_eq!(AgentConfig::from_toml(&text).unwrap(), c);
    }

    #[test]
    fn toml_defaults_optional_fields() {
        let c = AgentConfig::from_toml("name='A'\nprovider='claude'\npolicy='ask'\n").unwrap();
        assert_eq!(c.model, None);
        assert_eq!(c.system_prompt, "");
    }

    #[test]
    fn toml_rejects_missing_policy() {
        assert!(AgentConfig::from_toml("name='A'\nprovider='claude'\n").is_err());
    }

    #[test]
    fn plugin_created_agents_always_need_approval() {
        let change = AgentChange::Create { config: cfg(ApprovalPolicy::DenyAll) };
        assert!(change.needs_user_approval(&Initiator::Plugin("p".into())));
        assert!(change.needs_user_approval(&Initiator::Agent(AgentId::new())));
    }

    #[test]
    fn plugin_cannot_loosen_policy_without_approval() {
        let a = AgentId::new();
        let loosen = AgentChange::SetPolicy { agent: a.clone(), from: ApprovalPolicy::DenyAll, to: ApprovalPolicy::Ask };
        assert!(loosen.needs_user_approval(&Initiator::Plugin("p".into())));
    }

    #[test]
    fn plugin_tightening_policy_still_needs_approval() {
        let a = AgentId::new();
        let tighten = AgentChange::SetPolicy { agent: a, from: ApprovalPolicy::Ask, to: ApprovalPolicy::DenyAll };
        assert!(tighten.needs_user_approval(&Initiator::Plugin("p".into())));
    }

    #[test]
    fn user_creating_an_agent_is_the_approval_itself() {
        let change = AgentChange::Create { config: cfg(ApprovalPolicy::Ask) };
        assert!(!change.needs_user_approval(&Initiator::User));
    }

    #[test]
    fn user_loosening_policy_is_confirmed_but_tightening_is_not() {
        let a = AgentId::new();
        let loosen = AgentChange::SetPolicy { agent: a.clone(), from: ApprovalPolicy::DenyAll, to: ApprovalPolicy::Ask };
        let tighten = AgentChange::SetPolicy { agent: a, from: ApprovalPolicy::Ask, to: ApprovalPolicy::DenyAll };
        assert!(loosen.needs_user_approval(&Initiator::User));
        assert!(!tighten.needs_user_approval(&Initiator::User));
    }
}
