#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unconfirmed_action_does_not_run() {
        let mut ran = false;
        assert!(require_confirmation(false, || { ran = true; Ok("done".into()) }).is_err());
        assert!(!ran);
    }
}

/// Run an advanced operation only after confirmation.
pub fn require_confirmation<F: FnOnce() -> Result<String, String>>(confirmed: bool, run: F) -> Result<String, String> {
    if !confirmed { return Err("Confirmation is required".into()); }
    run()
}
fn action(id: &str) -> Result<discard_core::advanced::AdvancedAction, String> {
    use discard_core::advanced::AdvancedAction::*;
    match id { "reset_settings" => Ok(ResetSettings), "rebuild_memory_index" => Ok(RebuildMemoryIndex), "clear_logs" => Ok(ClearLogs), _ => Err("Unknown advanced action".into()) }
}
#[tauri::command]
pub fn settings_advanced_actions(context: tauri::State<'_, super::SettingsContext>) -> Vec<serde_json::Value> {
    ["reset_settings", "rebuild_memory_index", "clear_logs"].into_iter().map(|id| { let a = action(id).unwrap(); serde_json::json!({"id": id, "title": a.title(), "lines": discard_core::advanced::confirm_lines(a, &context.home)}) }).collect()
}
#[tauri::command]
pub fn settings_advanced_run(app: tauri::AppHandle, context: tauri::State<'_, super::SettingsContext>, actions: tauri::State<'_, std::sync::Arc<super::super::actions::DesktopActions>>, action_id: String, confirmed: bool) -> Result<String, String> {
    use discard_core::advanced::AdvancedAction::*;
    use tauri::Emitter;
    let action = action(&action_id)?;
    let result = require_confirmation(confirmed, || match action {
        ResetSettings => actions.settings.lock().unwrap_or_else(|e| e.into_inner()).reset().map(|p| p.map_or("Settings reset".into(), |p| format!("Backup: {}", p.display()))).map_err(|e| e.to_string()),
        RebuildMemoryIndex => context.memory.rebuild_index().map(|s| format!("Indexed {} files", s.indexed)).map_err(|e| e.to_string()),
        ClearLogs => discard_core::advanced::clear_logs(&context.home).map(|n| format!("Cleared {n} files")),
    })?;
    if action == ClearLogs { *context.logs.lock().unwrap_or_else(|e| e.into_inner()) = super::logs::LogState::open(&context.home); }
    let _ = app.emit("discard://settings", ());
    Ok(result)
}
