use discard_core::model::WorkspaceMode;
#[tauri::command]
pub fn settings_workspaces(context: tauri::State<'_, super::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let rows = discard_core::workspace_page::rows(&store, &discard_core::model::ChannelId(context.core.active_channel_id())).map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|project| serde_json::json!({"name": project.name, "path": project.path, "exists": project.exists, "channels": project.channels.into_iter().map(|channel| serde_json::json!({"name": channel.name, "mode": match channel.mode { WorkspaceMode::MainCheckout => "Main checkout", WorkspaceMode::Worktree => "Worktree" }, "archived": channel.archived, "current": channel.current, "worktree": channel.worktree.map(|w| serde_json::json!({"path": w.path, "branch": w.branch, "exists": w.exists})) })).collect::<Vec<_>>() })).collect())
}
#[tauri::command]
pub fn settings_reveal_workspace(context: tauri::State<'_, super::SettingsContext>, path: String) -> Result<(), String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let rows = discard_core::workspace_page::rows(&store, &discard_core::model::ChannelId(context.core.active_channel_id())).map_err(|e| e.to_string())?;
    if !rows.iter().any(|p| p.path == path || p.channels.iter().any(|c| c.worktree.as_ref().is_some_and(|w| w.path == path))) { return Err("Workspace is unavailable".into()); }
    super::reveal_path(std::path::Path::new(&path))
}
