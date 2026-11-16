#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trust_requires_ui_only_review() {
        assert!(check_trust(false, true).is_err());
        assert!(check_trust(true, true).is_ok());
        assert!(check_trust(false, false).is_ok());
    }
}

/// Trust is available only for reviews that request UI access alone.
pub fn check_trust(offered: bool, trusted: bool) -> Result<(), String> {
    if trusted && !offered { Err("Trust is available only for UI-only plugins".into()) } else { Ok(()) }
}
#[tauri::command]
pub fn settings_plugins(context: tauri::State<'_, super::SettingsContext>, core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>) -> serde_json::Value {
    use discard_lua::{PluginState, ReviewKind};
    let reviews = context.plugins.pending().into_iter().map(|r| serde_json::json!({"id": r.id, "name": r.name, "kind": match r.kind { ReviewKind::New => "New plugin", ReviewKind::Update => "Update" }, "description": r.manifest.description, "capabilities": r.capability_lines, "changes": r.capability_changes, "diff": r.diff, "fullAccess": r.full_local_access, "trustOffered": r.trust_offered})).collect::<Vec<_>>();
    let statuses = context.plugins.statuses();
    let errors = context.plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).errors();
    let problems = super::super::problems::problem_lines(&errors, &statuses, &context.load_report);
    let rows: Vec<_> = statuses.into_iter().map(|s| { let modified = s.state == PluginState::ModifiedOutsideReview; let state = match s.state { PluginState::Running => "running".into(), PluginState::Failed(e) => format!("failed: {}", e.lines().next().unwrap_or("")), PluginState::ModifiedOutsideReview => "disabled: files changed outside review".into(), PluginState::DisabledSafeMode => "disabled: safe mode".into() }; serde_json::json!({"name": s.name, "state": state, "modified": modified, "trusted": s.trusted_edits}) }).collect();
    serde_json::json!({"reviews": reviews, "statuses": rows, "problems": problems, "safeMode": context.load_report.safe_mode, "notice": core.plugin_notice()})
}
#[tauri::command]
pub fn settings_plugin_trust(context: tauri::State<'_, super::SettingsContext>, review_id: String, trusted: bool) -> Result<(), String> {
    let review = context.plugins.pending().into_iter().find(|r| r.id == review_id).ok_or("Review is unavailable")?;
    check_trust(review.trust_offered, trusted)?;
    let mut trust = context.trusted_reviews.lock().unwrap_or_else(|e| e.into_inner());
    if trusted { trust.insert(review_id); } else { trust.remove(&review_id); }
    Ok(())
}
#[tauri::command]
pub fn settings_plugin_approve(context: tauri::State<'_, super::SettingsContext>, core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>, review_id: String) -> Result<(), String> {
    let trusted = context.trusted_reviews.lock().unwrap_or_else(|e| e.into_inner()).remove(&review_id);
    let result = context.plugins.approve(&review_id, trusted).map(|_| ()).map_err(|e| e.to_string());
    core.set_plugin_notice(result.as_ref().err().cloned());
    core.clear_slots(); result
}
#[tauri::command]
pub fn settings_plugin_reject(context: tauri::State<'_, super::SettingsContext>, core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>, review_id: String) -> Result<(), String> {
    context.trusted_reviews.lock().unwrap_or_else(|e| e.into_inner()).remove(&review_id);
    if !context.plugins.reject(&review_id) { return Err("Review is unavailable".into()); }
    core.set_plugin_notice(None);
    core.clear_slots(); Ok(())
}
#[tauri::command]
pub fn settings_plugin_rollback(context: tauri::State<'_, super::SettingsContext>, core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>, name: String) -> Result<(), String> {
    let result = context.plugins.rollback(&name).map_err(|e| e.to_string());
    core.set_plugin_notice(result.as_ref().err().cloned());
    core.clear_slots(); result
}

/// Compare a modified plugin with the repository's current committed files.
#[tauri::command]
pub fn settings_plugin_modified_diff(context: tauri::State<'_, super::SettingsContext>, name: String) -> Result<String, String> {
    use discard_lua::{PluginState, snapshot::Snapshot};
    use std::{collections::BTreeMap, process::Command};
    if !context.plugins.statuses().iter().any(|s| s.name == name && s.state == PluginState::ModifiedOutsideReview) { return Err("Plugin is not modified outside review".into()); }
    let root = context.home.join("plugins");
    let current = Snapshot::capture(&root.join(&name)).map_err(|e| e.to_string())?;
    let git = |args: &[&str]| -> Result<Vec<u8>, String> {
        let output = discard_core::process::spawn_std(Command::new("git").args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"]).args(args).current_dir(&root).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_TERMINAL_PROMPT", "0").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).map_err(|e| e.to_string())?;
        if output.status.success() { Ok(output.stdout) } else { Err(String::from_utf8_lossy(&output.stderr).into_owned()) }
    };
    let prefix = format!("{name}/");
    let pathspec = format!(":(literal){prefix}");
    let listing = git(&["ls-tree", "-r", "--name-only", "-z", "HEAD", "--", &pathspec])?;
    let mut files = BTreeMap::new();
    for path in listing.split(|byte| *byte == 0).filter(|path| !path.is_empty()) {
        let path = std::str::from_utf8(path).map_err(|e| e.to_string())?;
        let relative = path.strip_prefix(&prefix).ok_or("Invalid plugin path")?;
        files.insert(relative.to_string(), git(&["show", &format!("HEAD:{path}")])?);
    }
    if files.is_empty() { return Err("No committed plugin files were found".into()); }
    Ok(discard_lua::repo::diff(Some(&Snapshot::from_files(files)), &current))
}

#[cfg(test)]
mod notice_tests {
    use super::super::super::{core::{AppCore, CoreInit}, view::ViewId};
    use discard_core::{model::ChannelId, ui_state::UiState};
    #[test]
    fn plugin_notice_can_be_shown_and_dismissed() {
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId::new(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.set_plugin_notice(Some("failed".into()));
        assert!(core.snapshot(&ViewId::main()).banners.problem_lines.contains(&"failed".into()));
        core.set_plugin_notice(None);
        assert!(core.snapshot(&ViewId::main()).banners.problem_lines.is_empty());
    }
}

#[tauri::command]
pub fn settings_plugin_dismiss_notice(core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>) {
    core.set_plugin_notice(None);
}
