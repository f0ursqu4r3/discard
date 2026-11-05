use discard_core::{capability::{CapabilityMatrix, HookMode}, provider::SetupStatus, provider_page::{provider_card, SetupLine}};

/// Build a provider card from its current setup and capabilities.
pub fn provider_row(id: &str, matrix: &CapabilityMatrix, setup: Option<&SetupStatus>) -> serde_json::Value {
    let card = provider_card(id, matrix, setup);
    let (status, command) = match card.setup {
        SetupLine::Checking => ("Checking", None), SetupLine::Ready => ("Ready", None),
        SetupLine::NotInstalled { install_hint } => ("Not installed", Some(install_hint)),
        SetupLine::NotLoggedIn { login_command } => ("Not logged in", Some(login_command)),
    };
    serde_json::json!({"id": card.id, "setup": status, "command": command,
        "capabilities": card.capabilities.into_iter().map(|row| serde_json::json!({"label": row.label, "supported": row.supported})).collect::<Vec<_>>(),
        "hooks": card.hooks.into_iter().map(|row| serde_json::json!({"label": row.label, "mode": match row.mode { HookMode::Active => "Active", HookMode::ObserveOnly => "Observe only", HookMode::Unsupported => "Unsupported" }})).collect::<Vec<_>>(),
        "blocked": card.ask_policy_blocked })
}

#[tauri::command]
pub fn settings_providers(context: tauri::State<'_, super::SettingsContext>) -> Vec<serde_json::Value> {
    context.providers.iter().map(|info| { let setup = info.setup.lock().unwrap_or_else(|e| e.into_inner()); provider_row(&info.id, &info.matrix, setup.as_ref()) }).collect()
}

#[tauri::command]
pub fn settings_recheck_provider(context: tauri::State<'_, super::SettingsContext>, id: String) -> Result<(), String> {
    if !context.providers.iter().any(|provider| provider.id == id) { return Err("Provider is unavailable".into()); }
    (context.recheck)(&id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::capability::{Capabilities, CapabilityMatrix};
    #[test]
    fn provider_rows_show_all_capabilities() {
        let matrix = CapabilityMatrix { provider: "test".into(), matrix_version: 1, cli_version: "1".into(), capabilities: Capabilities { interactive_approvals: true, tool_call_visibility: true, per_session_mcp: false, session_resume: true, cancellation: false, pre_execution_blocking: false } };
        let row = provider_row("test", &matrix, None);
        assert_eq!(row["setup"], "Checking");
        assert_eq!(row["capabilities"].as_array().unwrap().len(), 6);
    }
}
