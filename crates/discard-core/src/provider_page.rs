//! Provider status and capability information for Settings.
use crate::capability::{ApprovalPolicy, CapabilityMatrix, HookKind, HookMode};
use crate::provider::SetupStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupLine { Checking, Ready, NotInstalled { install_hint: String }, NotLoggedIn { login_command: String } }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRow { pub label: &'static str, pub supported: bool }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRow { pub label: &'static str, pub mode: HookMode }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCard { pub id: String, pub setup: SetupLine, pub capabilities: Vec<CapabilityRow>, pub hooks: Vec<HookRow>, pub ask_policy_blocked: Option<String> }

pub fn provider_card(id: &str, matrix: &CapabilityMatrix, setup: Option<&SetupStatus>) -> ProviderCard {
    let c = &matrix.capabilities;
    let capabilities = [("Interactive approvals", c.interactive_approvals), ("Tool call visibility", c.tool_call_visibility), ("Per-session MCP", c.per_session_mcp), ("Session resume", c.session_resume), ("Cancellation", c.cancellation), ("Block tool calls before they run", c.pre_execution_blocking)].into_iter().map(|(label, supported)| CapabilityRow { label, supported }).collect();
    let hooks = [("Block a tool call", HookKind::BlockToolCall), ("Observe events", HookKind::ObserveEvents)].into_iter().map(|(label, kind)| HookRow { label, mode: matrix.hook_mode(kind) }).collect();
    let setup = match setup { None => SetupLine::Checking, Some(SetupStatus::Ready) => SetupLine::Ready, Some(SetupStatus::NotInstalled { install_hint }) => SetupLine::NotInstalled { install_hint: install_hint.clone() }, Some(SetupStatus::NotLoggedIn { login_command }) => SetupLine::NotLoggedIn { login_command: login_command.clone() } };
    ProviderCard { id: id.into(), setup, capabilities, hooks, ask_policy_blocked: matrix.enforcement_for(ApprovalPolicy::Ask).err().map(|e| e.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capabilities, CapabilityMatrix, HookMode};
    use crate::provider::SetupStatus;

    fn matrix(interactive: bool, visible: bool, block: bool) -> CapabilityMatrix {
        CapabilityMatrix { provider: "test".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: interactive, tool_call_visibility: visible, per_session_mcp: false, session_resume: true, cancellation: false, pre_execution_blocking: block } }
    }

    #[test]
    fn card_lists_the_six_capabilities_in_order_with_their_values() {
        let card = provider_card("test", &matrix(true, true, false), None);
        assert_eq!(card.capabilities.iter().map(|r| (r.label, r.supported)).collect::<Vec<_>>(), [("Interactive approvals", true), ("Tool call visibility", true), ("Per-session MCP", false), ("Session resume", true), ("Cancellation", false), ("Block tool calls before they run", false)]);
        assert_eq!(card.setup, SetupLine::Checking);
        assert!(card.ask_policy_blocked.is_none());
    }

    #[test]
    fn hook_and_setup_status_and_policy_reason_follow_matrix() {
        for (visible, block, expected) in [(true, true, HookMode::Active), (true, false, HookMode::ObserveOnly), (false, false, HookMode::Unsupported)] {
            let card = provider_card("test", &matrix(false, visible, block), Some(&SetupStatus::NotInstalled { install_hint: "install here".into() }));
            assert_eq!(card.hooks[0].mode, expected);
            assert_eq!(card.setup, SetupLine::NotInstalled { install_hint: "install here".into() });
            assert!(card.ask_policy_blocked.unwrap().contains("cannot pause"));
        }
        assert_eq!(provider_card("test", &matrix(true, true, true), Some(&SetupStatus::NotLoggedIn { login_command: "login here".into() })).setup, SetupLine::NotLoggedIn { login_command: "login here".into() });
    }
}
