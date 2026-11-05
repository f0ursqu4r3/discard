use super::{
    core::AppCore,
    dto::{MessageDto, SnapshotDto},
};
use std::sync::Arc;
use tauri::State;
#[tauri::command]
pub fn get_snapshot(core: State<'_, Arc<AppCore>>) -> SnapshotDto {
    core.snapshot()
}
#[tauri::command]
pub fn get_history(core: State<'_, Arc<AppCore>>) -> Vec<MessageDto> {
    core.history()
}
#[tauri::command]
pub fn send_message(core: State<'_, Arc<AppCore>>, text: String) {
    core.send(text);
}
#[tauri::command]
pub fn decide_approval(core: State<'_, Arc<AppCore>>, approval_id: String, decision: String) {
    core.decide(approval_id, decision);
}
#[tauri::command]
pub fn cancel_turn(core: State<'_, Arc<AppCore>>) {
    core.cancel();
}

#[tauri::command]
pub fn set_panels(core: State<'_, Arc<AppCore>>, channel_list: f32, member_list: f32) -> Result<(), String> {
    core.set_panels(channel_list, member_list)
}
#[tauri::command]
pub fn set_member_list_visible(core: State<'_, Arc<AppCore>>, visible: bool) -> Result<(), String> {
    core.set_member_list_visible(visible)
}
#[tauri::command]
pub fn toggle_branch(core: State<'_, Arc<AppCore>>, key: String) {
    core.toggle_branch(key);
}

#[tauri::command]
pub fn mark_read(core: State<'_, Arc<AppCore>>, message_id: String) -> Result<(), String> {
    core.mark_read(&message_id)
}

#[tauri::command]
pub fn confirm_resolve(app: tauri::AppHandle, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, accept: bool) {
    if let Some(ending) = core.take_confirm() && accept { super::actions::perform(&app, &core, &actions, ending); }
}
#[tauri::command]
pub fn archive_confirm(core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, accept: bool) {
    if !core.has_pending_archive() { return; }
    if accept { super::actions::archive(core.inner().clone(), &actions, true); } else { core.cancel_archive(); }
}
#[tauri::command]
pub fn context_menu(core: State<'_, Arc<AppCore>>, target: super::dto::ContextTargetDto) -> Vec<super::dto::MenuEntryDto> {
    discard_core::context_menu::context_menu(&target.into(), &core.menu_context()).into_iter().filter_map(super::dto::MenuEntryDto::from_entry).collect()
}
#[tauri::command]
pub fn run_action(app: tauri::AppHandle, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, action_id: String, target: Option<super::dto::ContextTargetDto>) -> Option<String> {
    if action_id == "relaunch" {
        let ctx = core.menu_context();
        let ending = super::plan::Confirm::Relaunch { safe_mode: ctx.safe_mode };
        if ctx.turn_running && actions.confirms() { core.ask(ending); }
        else { super::actions::perform(&app, &core, &actions, ending); }
        return None;
    }
    let action = discard_core::action::Action::from_id(&action_id)?;
    super::actions::run(&app, core.inner(), &actions, action, target.map(Into::into))
}
#[tauri::command]
pub fn plugin_menu_action(core: State<'_, Arc<AppCore>>, plugin: String, index: usize, target: super::dto::ContextTargetDto) -> Result<(), String> {
    let target = target.into();
    let allowed = discard_core::context_menu::context_menu(&target, &core.menu_context()).iter().any(|item| matches!(item, discard_core::action::MenuEntry::Plugin { plugin: p, index: i, .. } if p == &plugin && *i == index));
    if !allowed { return Err("plugin menu item is unavailable".into()); }
    match core.run_plugin_menu(&plugin, index, Some(&target)) {
        Ok(()) => Ok(()),
        Err(error) => { core.plugin_failure(&plugin, &error); Err(error) }
    }
}

#[tauri::command]
pub fn plugin_action(core: State<'_, Arc<AppCore>>, plugin: String, action: String) -> Result<(), String> {
    core.plugin_action(&plugin, &action)
}

#[tauri::command]
pub fn web_shortcuts(actions: State<'_, Arc<super::actions::DesktopActions>>) -> Vec<(String, String)> {
    let settings = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    super::menu::web_shortcuts(&settings.settings().keymap)
}
