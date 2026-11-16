#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{action::Action, settings::SettingsStore};
    #[test]
    fn conflicting_chord_keeps_old_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::open(&dir.path().join("settings.toml"));
        let before = store.settings().keymap.chord(Action::OpenSettings).cloned();
        assert!(set_key(&mut store, Action::OpenSettings, Some("cmd-q".into())).is_err());
        assert_eq!(store.settings().keymap.chord(Action::OpenSettings), before.as_ref());
    }
}

use discard_core::{action::Action, keymap::{self, KeyRule}, settings::{Change, SettingsStore}};
/// Validate and save a keybinding.
pub fn set_key(store: &mut SettingsStore, action: Action, chord: Option<String>) -> Result<(), String> {
    super::general::apply_change(store, Change::Keybinding { action, chord })
}
#[tauri::command]
pub fn settings_keymap_rows(actions: tauri::State<'_, std::sync::Arc<super::super::actions::DesktopActions>>) -> Vec<serde_json::Value> {
    let store = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    keymap::rows(&store.settings().keymap).into_iter().filter(|row| super::super::menu::platform_action(row.action, cfg!(target_os = "macos"))).map(|row| serde_json::json!({"id": row.action.id(), "label": row.action.label(), "chord": row.chord.as_ref().map(keymap::display_chord).unwrap_or_else(|| "Not set".into()), "bound": row.chord.is_some(), "custom": row.custom, "rule": match row.rule { KeyRule::Locked => "locked", KeyRule::Required => "required", KeyRule::Optional => "optional" }})).collect()
}
#[tauri::command]
pub fn settings_keymap_set(app: tauri::AppHandle, core: tauri::State<'_, std::sync::Arc<super::super::core::AppCore>>, actions: tauri::State<'_, std::sync::Arc<super::super::actions::DesktopActions>>, action_id: String, chord: Option<String>, reset: bool) -> Result<(), String> {
    use tauri::Emitter;
    let action = Action::from_id(&action_id).ok_or("Unknown action")?;
    let mut store = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    if reset { super::general::apply_change(&mut store, Change::ResetKeybinding(action))?; } else { set_key(&mut store, action, chord)?; }
    let map = store.settings().keymap.clone();
    drop(store);
    let menu = super::super::menu::build_menu(&app, &core.menu_context(&super::super::view::ViewId::main()), &map).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    let _ = app.emit("discard://shortcuts", super::super::menu::web_shortcuts(&map));
    let _ = app.emit("discard://settings", ());
    Ok(())
}
