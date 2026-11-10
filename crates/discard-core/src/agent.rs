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
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    pub policy: ApprovalPolicy,
    /// May write plugins (as drafts the user must review). Off by default.
    #[serde(default)]
    pub plugin_write: bool,
    /// May edit the bound branch or worktree. Off by default.
    #[serde(default)]
    pub workspace_write: bool,
    /// May run sandboxed shell commands without approval cards. Needs `workspace_write`. Off by default.
    #[serde(default)]
    pub sandbox_autorun: bool,
}

/// Generate an editable human name without using a model as identity.
pub fn generated_name() -> String {
    const NAMES: &[&str] = &["Alex", "Casey", "Jordan", "Morgan", "Quinn", "Robin", "Sam", "Taylor"];
    let id = uuid::Uuid::new_v4();
    NAMES[(id.as_bytes()[0] as usize) % NAMES.len()].into()
}

impl AgentConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() { return Err("Enter an agent name.".into()); }
        if self.provider.trim().is_empty() { return Err("Select a provider.".into()); }
        if self.model.as_ref().is_some_and(|model| model.trim().is_empty()) { return Err("Select a model.".into()); }
        if self.reasoning_effort.as_ref().is_some_and(|effort| !matches!(effort.as_str(), "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra")) { return Err("Choose a valid reasoning level.".into()); }
        Ok(())
    }

    /// Runtime identity and optional personality; the stored personality stays editable on its own.
    pub fn runtime_system_prompt(&self) -> String {
        let mut prompt = format!(
            "Your name is {:?}. You are an AI agent in Discard. Use this name when introducing yourself or answering questions about your identity. The provider and model describe the technology you use, not your name.",
            self.name
        );
        prompt.push_str("\n\nResponse style: Answer the current request directly in short, plain sentences. Lead with the result or next action. Avoid repeating facts and warnings already given unless they changed or are needed for this decision. Use short paragraphs, and lists only when they help. State the specific missing fact once instead of repeating generic limitations. Keep code blocks for exact commands or code; explain their purpose in one sentence. Quote paths correctly in commands, and use a known working directory. Never claim an action ran until a tool confirms its result. When an app approval is pending, say what the approval will do and let the approval card carry the details. Adapt to the user's requested level of detail.");
        if !self.system_prompt.trim().is_empty() {
            prompt.push_str("\n\nPersonality and instructions:\n");
            prompt.push_str(&self.system_prompt);
        }
        prompt
    }

    /// The prompt for a chat turn. Chats run in Conversation mode, which never loads plugin
    /// tools, so the agent learns how the user can get them.
    pub fn conversation_system_prompt(&self) -> String {
        let mut prompt = self.runtime_system_prompt();
        prompt.push_str(if self.plugin_write {
            "\n\nPlugin tools: Your profile allows plugin writes, but this chat has no plugin tools. Plugin tools load only when the user starts Discard from a terminal with `discard --builder`. If the user asks you to create or change a plugin, tell them this once. Never claim you wrote a plugin."
        } else {
            "\n\nPlugin tools: This chat has no plugin tools. To create or change a plugin, the user starts Discard from a terminal with `discard --builder` and asks the Builder agent. If the user asks you for a plugin, tell them this once. Never claim you wrote a plugin."
        });
        prompt
    }

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
    SetPluginWrite { agent: AgentId, enabled: bool },
    SetWorkspaceWrite { agent: AgentId, enabled: bool },
    SetSandboxAutorun { agent: AgentId, enabled: bool },
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
            (Initiator::User, AgentChange::SetPluginWrite { enabled, .. } | AgentChange::SetWorkspaceWrite { enabled, .. } | AgentChange::SetSandboxAutorun { enabled, .. }) => *enabled,
            (Initiator::Plugin(_) | Initiator::Agent(_), _) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_workspace_writes_needs_user_approval() {
        let change = AgentChange::SetWorkspaceWrite { agent: AgentId("a".into()), enabled: true };
        assert!(change.needs_user_approval(&Initiator::User));
        let off = AgentChange::SetWorkspaceWrite { agent: AgentId("a".into()), enabled: false };
        assert!(!off.needs_user_approval(&Initiator::User));
        assert!(off.needs_user_approval(&Initiator::Plugin("p".into())));
    }

    #[test]
    fn enabling_sandbox_autorun_needs_user_approval() {
        let on = AgentChange::SetSandboxAutorun { agent: AgentId("a".into()), enabled: true };
        assert!(on.needs_user_approval(&Initiator::User));
        let off = AgentChange::SetSandboxAutorun { agent: AgentId("a".into()), enabled: false };
        assert!(!off.needs_user_approval(&Initiator::User));
        assert!(off.needs_user_approval(&Initiator::Plugin("p".into())));
    }

    #[test]
    fn old_agent_toml_reads_sandbox_autorun_as_false() {
        assert!(!AgentConfig::from_toml("name = 'A'\nprovider = 'claude'\npolicy = 'ask'\n").unwrap().sandbox_autorun);
    }

    #[test]
    fn old_agent_toml_reads_workspace_write_as_false() {
        assert!(!AgentConfig::from_toml("name = 'A'\nprovider = 'claude'\npolicy = 'ask'\n").unwrap().workspace_write);
    }

    fn cfg(policy: ApprovalPolicy) -> AgentConfig {
        AgentConfig {
            workspace_write: false, sandbox_autorun: false,
            reasoning_effort: None,
            name: "Scout".into(),
            avatar: None,
            provider: "claude".into(),
            model: Some("haiku".into()),
            system_prompt: "Be brief.".into(),
            policy,
            plugin_write: false,
        }
    }

    #[test]
    fn runtime_prompt_includes_identity_even_without_personality() {
        let mut config = cfg(ApprovalPolicy::Ask);
        config.name = "Quinn".into();
        config.system_prompt.clear();
        let prompt = config.runtime_system_prompt();
        assert!(prompt.contains("Your name is \"Quinn\"."));
        assert!(prompt.contains("AI agent in Discard"));
        assert!(!prompt.contains("Personality and instructions:"));
        assert!(config.system_prompt.is_empty());
    }

    #[test]
    fn runtime_prompt_preserves_personality_and_quotes_the_name() {
        let mut config = cfg(ApprovalPolicy::Ask);
        config.name = "Quinn \"Q\"\nHelper".into();
        let prompt = config.runtime_system_prompt();
        assert!(prompt.contains(r#"Your name is "Quinn \"Q\"\nHelper"."#));
        assert!(prompt.ends_with("Personality and instructions:\nBe brief."));
        assert_eq!(config.system_prompt, "Be brief.");
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut c = cfg(ApprovalPolicy::Ask);
        c.reasoning_effort = Some("high".into());
        let text = c.to_toml().unwrap();
        assert_eq!(AgentConfig::from_toml(&text).unwrap(), c);
    }

    #[test]
    fn toml_defaults_optional_fields() {
        let c = AgentConfig::from_toml("name='A'\nprovider='claude'\npolicy='ask'\n").unwrap();
        assert_eq!(c.model, None);
        assert_eq!(c.reasoning_effort, None);
        assert_eq!(c.system_prompt, "");
    }

    #[test]
    fn plugin_write_defaults_to_off_and_round_trips() {
        let c = AgentConfig::from_toml("name='A'\nprovider='claude'\npolicy='ask'\n").unwrap();
        assert!(!c.plugin_write, "plugin authoring must be opt-in");
        let mut on = cfg(ApprovalPolicy::Ask);
        on.plugin_write = true;
        assert!(AgentConfig::from_toml(&on.to_toml().unwrap()).unwrap().plugin_write);
    }

    #[test]
    fn chat_prompt_tells_the_agent_how_the_user_gets_plugin_tools() {
        let mut c = cfg(ApprovalPolicy::Ask);
        assert!(!c.runtime_system_prompt().contains("discard --builder"), "builder sessions have the tools");
        let off = c.conversation_system_prompt();
        assert!(off.starts_with(&c.runtime_system_prompt()));
        assert!(off.contains("This chat has no plugin tools."));
        assert!(off.contains("`discard --builder` and asks the Builder agent"));
        c.plugin_write = true;
        let on = c.conversation_system_prompt();
        assert!(on.contains("Your profile allows plugin writes, but this chat has no plugin tools."));
        assert!(on.contains("`discard --builder`"));
        assert!(!on.contains("Builder agent"));
    }

    #[test]
    fn turning_on_plugin_writing_needs_user_approval_when_a_plugin_or_agent_asks() {
        let on = AgentChange::SetPluginWrite { agent: AgentId::new(), enabled: true };
        assert!(on.needs_user_approval(&Initiator::Plugin("p".into())));
        assert!(on.needs_user_approval(&Initiator::Agent(AgentId::new())));
        assert!(on.needs_user_approval(&Initiator::User), "granting a power is always confirmed");
        let off = AgentChange::SetPluginWrite { agent: AgentId::new(), enabled: false };
        assert!(!off.needs_user_approval(&Initiator::User), "taking a power away by the user needs no confirmation");
        assert!(off.needs_user_approval(&Initiator::Plugin("p".into())));
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
