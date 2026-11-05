#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_memory_text_to_twenty_thousand_characters() {
        let text = limit_text(&"a".repeat(20_001));
        assert!(text.ends_with("\nText is limited to 20,000 characters."));
        assert!(text.starts_with(&"a".repeat(20_000)));
    }
}

/// Limit the text shown in the settings window.
pub fn limit_text(text: &str) -> String {
    let mut result: String = text.chars().take(20_000).collect();
    if text.chars().count() > 20_000 { result.push_str("\nText is limited to 20,000 characters."); }
    result
}
fn summaries(context: &super::SettingsContext) -> Vec<discard_core::memory_page::ScopeSummary> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    discard_core::memory_page::summaries(&context.memory, &store.list_agents().unwrap_or_default(), &store.list_projects().unwrap_or_default())
}
#[tauri::command]
pub fn settings_memory_scopes(context: tauri::State<'_, super::SettingsContext>) -> Vec<serde_json::Value> {
    summaries(&context).into_iter().map(|row| serde_json::json!({"scope": row.scope.label(), "title": row.title, "files": row.files.into_iter().map(|f| f.path).collect::<Vec<_>>() })).collect()
}
#[tauri::command]
pub fn settings_memory_read(context: tauri::State<'_, super::SettingsContext>, scope: String, path: String) -> Result<String, String> {
    let row = summaries(&context).into_iter().find(|row| row.scope.label() == scope && row.files.iter().any(|f| f.path == path)).ok_or("Memory file is unavailable")?;
    context.memory.read(&row.scope, &path).map(|content| limit_text(&content.text)).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn settings_reveal_memory(context: tauri::State<'_, super::SettingsContext>, scope: String) -> Result<(), String> {
    use discard_core::memory::Scope;
    let row = summaries(&context).into_iter().find(|row| row.scope.label() == scope).ok_or("Memory scope is unavailable")?;
    let root = context.home.join("memory");
    let path = match row.scope { Scope::Global => root.join("global"), Scope::Project(id) => root.join("projects").join(id.0), Scope::Agent(id) => root.join("agents").join(id.0) };
    super::reveal_path(&path)
}
