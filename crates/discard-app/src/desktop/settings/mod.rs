pub mod general;

#[cfg(test)]
mod tests {
    #[test]
    fn empty_search_lists_all_pages_with_availability() {
        let entries = super::sidebar_entries("");
        assert_eq!(entries.len(), discard_core::settings_page::SettingsPage::all().len());
        for (entry, page) in entries.iter().zip(discard_core::settings_page::SettingsPage::all()) {
            assert_eq!(entry.available, page.is_available());
        }
    }
    #[test]
    fn no_match_gives_no_entries() { assert!(super::sidebar_entries("zzz").is_empty()); }
    #[test]
    fn unavailable_entry_cannot_be_selected() {
        assert!(!super::entry_for(discard_core::settings_page::SettingsPage::Workspaces, false).available);
    }
    #[test]
    fn search_theme_keeps_only_appearance() {
        let entries = super::sidebar_entries("theme");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Appearance");
    }
}

/// A page shown in the settings sidebar.
#[derive(serde::Serialize)]
pub struct SidebarEntry {
    pub title: &'static str,
    pub available: bool,
}

fn entry_for(page: discard_core::settings_page::SettingsPage, available: bool) -> SidebarEntry {
    SidebarEntry { title: page.title(), available }
}

pub fn sidebar_entries(query: &str) -> Vec<SidebarEntry> {
    discard_core::settings_page::search(query).into_iter().map(|page| entry_for(page, page.is_available())).collect()
}

#[tauri::command]
pub fn settings_search(query: String) -> Vec<SidebarEntry> { sidebar_entries(&query) }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralPatch { pub default_provider: Option<String>, pub confirm_while_working: Option<bool> }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearancePatch {
    pub theme_source: Option<String>,
    pub theme: Option<String>,
    pub ignore_plugin_themes: Option<bool>,
    pub mini_project_rail: Option<bool>,
    pub message_density: Option<String>,
    pub show_message_avatars: Option<bool>,
}

#[tauri::command]
pub fn settings_get(app: tauri::AppHandle, context: tauri::State<'_, SettingsContext>, page: String, actions: tauri::State<'_, std::sync::Arc<super::actions::DesktopActions>>) -> Result<serde_json::Value, String> {
    let mut store = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    store.reload_if_changed();
    let settings = store.settings().clone();
    let issues = general::issue_lines(store.issues());
    drop(store);
    match page.as_str() {
        "general" => Ok(serde_json::json!({"defaultProvider": settings.general.default_provider, "confirmWhileWorking": settings.general.confirm_while_working, "providers": discard_core::bootstrap::PROVIDERS, "issues": issues, "themeProblems": appearance_data(&app, &context, &settings.appearance)["problems"] })),
        "appearance" => Ok(appearance_data(&app, &context, &settings.appearance)),
        _ => Err("Unknown settings page".into()),
    }
}

fn apply_one(store: &mut discard_core::settings::SettingsStore, changes: Vec<discard_core::settings::Change>) -> Result<(), String> {
    if changes.len() != 1 { return Err("Send one setting at a time".into()); }
    store.reload_if_changed();
    store.apply(changes.into_iter().next().unwrap()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_set_general(app: tauri::AppHandle, actions: tauri::State<'_, std::sync::Arc<super::actions::DesktopActions>>, patch: GeneralPatch) -> Result<(), String> {
    use discard_core::settings::Change;
    use tauri::Emitter;
    let mut changes = Vec::new();
    if let Some(value) = patch.default_provider { changes.push(Change::DefaultProvider(value)); }
    if let Some(value) = patch.confirm_while_working { changes.push(Change::ConfirmWhileWorking(value)); }
    apply_one(&mut actions.settings.lock().unwrap_or_else(|e| e.into_inner()), changes)?;
    let _ = app.emit("discard://settings", ());
    Ok(())
}

#[tauri::command]
pub fn settings_set_appearance(app: tauri::AppHandle, actions: tauri::State<'_, std::sync::Arc<super::actions::DesktopActions>>, patch: AppearancePatch) -> Result<(), String> {
    use discard_core::{settings::Change, ghostty::resolve::ThemeSource};
    use tauri::Emitter;
    let mut changes = Vec::new();
    if let Some(value) = patch.theme_source { changes.push(Change::ThemeSource(ThemeSource::from_setting(&value).ok_or_else(|| "invalid theme source".to_string())?)); }
    if let Some(value) = patch.theme { changes.push(Change::Theme(value)); }
    if let Some(value) = patch.ignore_plugin_themes { changes.push(Change::IgnorePluginThemes(value)); }
    if let Some(value) = patch.mini_project_rail { changes.push(Change::MiniProjectRail(value)); }
    if let Some(value) = patch.show_message_avatars { changes.push(Change::ShowMessageAvatars(value)); }
    if let Some(value) = patch.message_density {
        changes.push(Change::MessageDensity(discard_core::settings::MessageDensity::from_setting(&value)
            .ok_or_else(|| "invalid message density".to_string())?));
    }
    apply_one(&mut actions.settings.lock().unwrap_or_else(|e| e.into_inner()), changes)?;
    let _ = app.emit("discard://settings", ());
    Ok(())
}

/// Shared data needed by settings pages.
pub struct SettingsContext {
    pub home: std::path::PathBuf,
    pub plugins: std::sync::Arc<discard_lua::PluginService>,
    pub providers: Vec<super::actions::ProviderInfo>,
    pub recheck: super::actions::Recheck,
    pub store: std::sync::Arc<std::sync::Mutex<discard_core::store::Store>>,
    pub memory: std::sync::Arc<discard_core::memory::MemoryStore>,
    pub core: std::sync::Arc<super::core::AppCore>,
    pub load_report: discard_lua::LoadReport,
    pub trusted_reviews: std::sync::Mutex<std::collections::HashSet<String>>,
    pub logs: std::sync::Mutex<logs::LogState>,
}

fn appearance_data(app: &tauri::AppHandle, context: &SettingsContext, settings: &discard_core::settings::AppearanceSettings) -> serde_json::Value {
    use discard_core::ghostty::{live::LiveTheme, map::swatches, spec::{Appearance, ThemeSpec}, lookup::Origin};
    use tauri::Manager;
    let mode = if app.get_webview_window("main").and_then(|w| w.theme().ok()) == Some(tauri::Theme::Light) { Appearance::Light } else { Appearance::Dark };
    let (dirs, configs) = super::theme::theme_env(&context.home);
    let live = LiveTheme::new(dirs.clone(), configs.clone(), settings, mode);
    let resolved = live.resolved();
    let spec = ThemeSpec::parse(&settings.theme).unwrap_or_else(|| ThemeSpec::Single(resolved.theme_name.clone()));
    let found = dirs.find(&resolved.theme_name);
    let origin = found.as_ref().map(|f| match f.origin { Origin::Yours => "Yours", Origin::Ghostty => "Ghostty", Origin::Bundled => "Bundled" });
    let file = found.as_ref().map(|f| f.path.as_ref().map_or_else(|| "Built into Discard".into(), |p| p.display().to_string())).unwrap_or_else(|| "No theme file found".into());
    let config = configs.into_iter().find(|p| p.is_file()).map_or_else(|| "No Ghostty config found".into(), |p| p.display().to_string());
    let names: Vec<_> = dirs.list().into_iter().map(|(name, origin)| (name, match origin { Origin::Yours => "Yours", Origin::Ghostty => "Ghostty", Origin::Bundled => "Bundled" })).collect();
    let palette: Vec<_> = swatches(&resolved.colors).into_iter().map(|(name, color)| (name, color.to_hex())).collect();
    serde_json::json!({"miniProjectRail": settings.mini_project_rail, "messageDensity": settings.message_density.as_str(), "showMessageAvatars": settings.show_message_avatars, "themeSource": settings.theme_source.as_str(), "theme": settings.theme, "ignorePluginThemes": settings.ignore_plugin_themes, "single": matches!(spec, ThemeSpec::Single(_)), "light": spec.name_for(Appearance::Light), "dark": spec.name_for(Appearance::Dark), "activeName": resolved.theme_name, "origin": origin, "file": file, "config": config, "themes": names, "swatches": palette, "problems": super::problems::theme_problem_lines(&resolved.problems), "overrides": context.plugins.theme_sources() })
}

#[tauri::command]
pub fn settings_copy_theme(context: tauri::State<'_, SettingsContext>, name: String) -> Result<(), String> {
    let (dirs, _) = super::theme::theme_env(&context.home);
    let path = discard_core::ghostty::copy::copy_to_yours(&dirs, &name).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = { let mut c = std::process::Command::new("cmd"); c.args(["/c", "start", ""]); c };
    let status = command.arg(path).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("Editor command failed: {status}")) }
}

pub mod appearance;

#[tauri::command]
pub fn settings_choose_theme(app: tauri::AppHandle, actions: tauri::State<'_, std::sync::Arc<super::actions::DesktopActions>>, choice: String, name: Option<String>) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let light = app.get_webview_window("main").and_then(|w| w.theme().ok()) == Some(tauri::Theme::Light);
    let mut store = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    store.reload_if_changed();
    let theme = appearance::theme_choice(&store.settings().appearance.theme, &choice, name.as_deref(), light)?;
    store.apply(discard_core::settings::Change::Theme(theme)).map_err(|e| e.to_string())?;
    let _ = app.emit("discard://settings", ());
    Ok(())
}

pub mod providers;

pub mod agents;

pub mod memory;
pub mod project_memory;

pub mod workspaces;

/// Reveal a known path with the platform file manager.
fn reveal_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = { let mut c = std::process::Command::new("open"); c.arg("-R"); c };
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = { let mut c = std::process::Command::new("explorer"); c.arg("/select,"); c };
    #[cfg(target_os = "linux")]
    let path = path.parent().unwrap_or(path);
    let status = command.arg(path).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("Cannot reveal path: {status}")) }
}

pub mod advanced;

pub mod keybindings;

pub mod plugins;

pub mod logs;
