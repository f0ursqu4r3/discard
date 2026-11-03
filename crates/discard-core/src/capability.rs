//! Provider capability matrix and the fail-closed policy rule.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// The provider pauses a tool call and waits for the host to answer.
    pub interactive_approvals: bool,
    pub tool_call_visibility: bool,
    pub per_session_mcp: bool,
    pub session_resume: bool,
    pub cancellation: bool,
    /// The host can block a tool call before it runs (not just observe it).
    pub pre_execution_blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMatrix {
    pub provider: String,
    /// Schema version of this fixture.
    pub matrix_version: u32,
    /// CLI version the matrix was recorded against.
    pub cli_version: String,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// Pause on every tool call that needs permission and ask the user.
    Ask,
    /// The provider denies everything that would need permission.
    DenyAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Approvals go to the host through the provider's interactive channel.
    HostPrompts,
    /// The provider auto-denies anything that needs permission.
    ProviderDenies,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("provider {provider} cannot enforce policy {policy:?}: {reason}")]
    CannotEnforce { provider: String, policy: ApprovalPolicy, reason: &'static str },
}

impl CapabilityMatrix {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Choose how to enforce `policy`. Never falls back to a looser mode.
    pub fn enforcement_for(&self, policy: ApprovalPolicy) -> Result<EnforcementMode, PolicyError> {
        match policy {
            ApprovalPolicy::Ask if self.capabilities.interactive_approvals => Ok(EnforcementMode::HostPrompts),
            ApprovalPolicy::Ask => Err(PolicyError::CannotEnforce {
                provider: self.provider.clone(),
                policy,
                reason: "provider cannot pause tool calls for host approval",
            }),
            ApprovalPolicy::DenyAll => Ok(EnforcementMode::ProviderDenies),
        }
    }

    /// Check that the provider can run in `mode`. Used when a session starts.
    pub fn supports(&self, mode: EnforcementMode) -> Result<(), PolicyError> {
        match mode {
            EnforcementMode::HostPrompts => self.enforcement_for(ApprovalPolicy::Ask).map(|_| ()),
            EnforcementMode::ProviderDenies => Ok(()),
        }
    }

    pub fn hook_mode(&self, hook: HookKind) -> HookMode {
        let c = &self.capabilities;
        match hook {
            HookKind::BlockToolCall if c.pre_execution_blocking => HookMode::Active,
            HookKind::BlockToolCall if c.tool_call_visibility => HookMode::ObserveOnly,
            HookKind::BlockToolCall => HookMode::Unsupported,
            HookKind::ObserveEvents if c.tool_call_visibility => HookMode::Active,
            HookKind::ObserveEvents => HookMode::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// A hook that can stop a tool call before it runs.
    BlockToolCall,
    /// A hook that only reads events.
    ObserveEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    Active,
    /// Shown as "observe only" in the plugin UI. Never presented as a block.
    ObserveOnly,
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(interactive: bool, pre_block: bool) -> CapabilityMatrix {
        CapabilityMatrix {
            provider: "test".into(),
            matrix_version: 1,
            cli_version: "0".into(),
            capabilities: Capabilities {
                interactive_approvals: interactive,
                tool_call_visibility: true,
                per_session_mcp: true,
                session_resume: true,
                cancellation: true,
                pre_execution_blocking: pre_block,
            },
        }
    }

    #[test]
    fn ask_policy_uses_host_prompts_when_supported() {
        assert_eq!(caps(true, true).enforcement_for(ApprovalPolicy::Ask), Ok(EnforcementMode::HostPrompts));
    }

    #[test]
    fn ask_policy_fails_closed_when_provider_cannot_pause() {
        let err = caps(false, false).enforcement_for(ApprovalPolicy::Ask).unwrap_err();
        assert!(matches!(err, PolicyError::CannotEnforce { policy: ApprovalPolicy::Ask, .. }));
    }

    #[test]
    fn deny_all_is_always_enforceable() {
        assert_eq!(caps(false, false).enforcement_for(ApprovalPolicy::DenyAll), Ok(EnforcementMode::ProviderDenies));
    }

    #[test]
    fn supports_host_prompts_only_with_interactive_approvals() {
        assert_eq!(caps(true, false).supports(EnforcementMode::HostPrompts), Ok(()));
        assert!(caps(false, false).supports(EnforcementMode::HostPrompts).is_err());
        assert_eq!(caps(false, false).supports(EnforcementMode::ProviderDenies), Ok(()));
    }

    #[test]
    fn blocking_hooks_are_active_only_with_pre_execution_blocking() {
        assert_eq!(caps(true, true).hook_mode(HookKind::BlockToolCall), HookMode::Active);
        assert_eq!(caps(true, false).hook_mode(HookKind::BlockToolCall), HookMode::ObserveOnly);
    }

    #[test]
    fn observe_hooks_need_tool_call_visibility() {
        let mut m = caps(true, false);
        assert_eq!(m.hook_mode(HookKind::ObserveEvents), HookMode::Active);
        m.capabilities.tool_call_visibility = false;
        assert_eq!(m.hook_mode(HookKind::ObserveEvents), HookMode::Unsupported);
    }

    #[test]
    fn matrix_loads_from_json_and_rejects_unknown_shapes() {
        let ok = serde_json::to_string(&caps(true, false)).unwrap();
        assert_eq!(CapabilityMatrix::from_json(&ok).unwrap(), caps(true, false));
        assert!(CapabilityMatrix::from_json("{\"provider\":\"x\"}").is_err());
    }
}
