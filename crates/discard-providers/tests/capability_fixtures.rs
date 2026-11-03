//! The checked-in capability matrices must load, and unverified providers must fail closed.

use discard_core::capability::{ApprovalPolicy, CapabilityMatrix, EnforcementMode, HookKind, HookMode};
use discard_providers::claude::ClaudeProvider;
use discard_providers::codex::CodexProvider;
use discard_core::provider::Provider;

const CLAUDE: &str = include_str!("../fixtures/claude.capabilities.json");
const CODEX: &str = include_str!("../fixtures/codex.capabilities.json");

#[test]
fn claude_matrix_loads_and_matches_the_provider() {
    let m = CapabilityMatrix::from_json(CLAUDE).unwrap();
    assert_eq!(m.provider, "claude");
    assert_eq!(&m, ClaudeProvider::new("claude").capabilities());
    assert_eq!(m.enforcement_for(ApprovalPolicy::Ask), Ok(EnforcementMode::HostPrompts));
    assert_eq!(m.hook_mode(HookKind::BlockToolCall), HookMode::Active);
}

#[test]
fn codex_matrix_loads_matches_the_provider_and_records_the_verified_capabilities() {
    let m = CapabilityMatrix::from_json(CODEX).unwrap();
    assert_eq!(m.provider, "codex");
    assert_eq!(&m, CodexProvider::new("codex").capabilities());
    assert!(m.matrix_version >= 2, "version 2 is the first matrix verified against a live app-server");
    assert_eq!(m.enforcement_for(ApprovalPolicy::Ask), Ok(EnforcementMode::HostPrompts));
    assert_eq!(m.hook_mode(HookKind::BlockToolCall), HookMode::Active);
}
