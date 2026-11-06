use super::{
    core::AppCore,
    dto::{HistoryDto, SnapshotDto},
};
use std::sync::Arc;
use tauri::State;
#[tauri::command]
pub fn get_snapshot(core: State<'_, Arc<AppCore>>) -> SnapshotDto {
    core.snapshot()
}
#[tauri::command]
pub fn get_history(core: State<'_, Arc<AppCore>>) -> HistoryDto {
    core.history_snapshot()
}
#[tauri::command]
pub fn send_message(channels: State<'_, Arc<super::channels::Channels>>, channel_id: String, text: String) -> Result<(), String> {
    channels.send_message(&channel_id, text)
}
#[tauri::command]
pub fn decide_approval(core: State<'_, Arc<AppCore>>, channel_id: String, approval_id: String, decision: String) {
    core.decide_in(Some(&channel_id), approval_id, decision);
}
#[tauri::command]
pub fn cancel_turn(core: State<'_, Arc<AppCore>>, channel_id: String) {
    core.cancel_in(Some(&channel_id));
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
pub fn mark_read(core: State<'_, Arc<AppCore>>, channel_id: String, message_id: String) -> Result<(), String> {
    core.mark_read_in(Some(&channel_id), &message_id)
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
    if action_id == "open_provider_settings" {
        if let Err(error) = super::windows::open_settings(&app, discard_core::settings_page::SettingsPage::Providers) { core.notice(format!("Cannot open CLI connections: {error}")); }
        return None;
    }
    if action_id == "relaunch" {
        let ctx = core.menu_context();
        let ending = super::plan::Confirm::Relaunch { safe_mode: ctx.safe_mode };
        if core.any_running() && actions.confirms() { core.ask(ending); }
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

#[tauri::command]
pub async fn create_channel(channels: State<'_, Arc<super::channels::Channels>>, name: String, workspace_mode: String, source_channel: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.create(&name, &workspace_mode, &source_channel)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn switch_channel(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.select(&channel_id)).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    super::windows::show_conversation(&app);
    Ok(())
}
#[tauri::command]
pub async fn open_branch_channel(channels: State<'_, Arc<super::channels::Channels>>, key: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.open_branch(&key)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_projects(context: State<'_, super::settings::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    store.visible_projects().map_err(|e| e.to_string())?.into_iter().map(|p| {
        let avatar = store.project_avatar(&p.id).map_err(|e| e.to_string())?;
        let main = store.project_main_agent(&p.id).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"id":p.id.0,"name":p.name,"path":p.path,"avatar":avatar,"mainAgentId":main.map(|id| id.0)}))
    }).collect()
}

#[tauri::command]
pub fn update_project_profile(app: tauri::AppHandle, core: State<'_, Arc<AppCore>>, context: State<'_, super::settings::SettingsContext>, project_id: String, name: String, avatar: Option<String>) -> Result<(), String> {
    let id = discard_core::model::ProjectId(project_id);
    context.store.lock().unwrap_or_else(|e| e.into_inner())
        .update_project_profile(&id, &name, avatar.as_deref()).map_err(|e| e.to_string())?;
    core.update_project_name(&id.0, name.trim());
    use tauri::Emitter;
    let _ = app.emit("discard://projects", ());
    Ok(())
}
#[tauri::command]
pub async fn select_project(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    channels.core.begin_project_navigation()?;
    let core = channels.core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.activate_project(&discard_core::model::ProjectId(project_id))).await.map_err(|e| e.to_string()).and_then(|r| r);
    core.end_project_change();
    let channel_id = result?;
    use tauri::Emitter;
    let _ = app.emit("discard://conversation-opened", channel_id.clone());
    let _ = app.emit("discard://conversations", ());
    Ok(channel_id)
}

#[tauri::command]
pub async fn get_project_channels(channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<Vec<super::dto::ProjectChannelDto>, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.list_project_channels(&discard_core::model::ProjectId(project_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_project_members(channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<Vec<super::dto::ChannelMemberDto>, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.list_project_members(&discard_core::model::ProjectId(project_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn invite_channel_member(channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String, agent_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.invite_channel_member(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id), &discard_core::model::AgentId(agent_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn select_canonical_branch(channels: State<'_, Arc<super::channels::Channels>>, project_id: String, branch: String, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.select_canonical_branch(&discard_core::model::ProjectId(project_id), &branch, &discard_core::model::ChannelId(channel_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn select_canonical_project_channel(channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.select_canonical_general(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_channel_details(channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String) -> Result<super::dto::ChannelDetailsDto, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.channel_details(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn update_channel_topic(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String, expected_version: u64, text: String) -> Result<super::dto::ChannelTopicDto, String> {
    let channels = channels.inner().clone();
    let topic = tauri::async_runtime::spawn_blocking(move || channels.update_channel_topic(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id), expected_version, &text)).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://channel-details", topic.channel_id.clone());
    Ok(topic)
}
#[tauri::command]
pub async fn set_project_main_agent(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, agent_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.change_project_main_agent(&discard_core::model::ProjectId(project_id), &discard_core::model::AgentId(agent_id))).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    Ok(())
}
#[tauri::command]
pub async fn remove_project(channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<(), String> {
    let channels = channels.inner().clone(); channels.core.begin_project_change()?;
    let core = channels.core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let id = discard_core::model::ProjectId(project_id);
        let next = channels.store.lock().unwrap_or_else(|e| e.into_inner()).visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id != id);
        if channels.core.current_project().id == id {
            if let Some(next) = next { channels.activate_project(&next.id)?; } else { channels.core.select_sidebar_project(None); }
        }
        channels.store.lock().unwrap_or_else(|e| e.into_inner()).hide_project(&id).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string()).and_then(|r| r);
    core.end_project_change(); result
}
#[tauri::command]
pub async fn choose_project_folder(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, change_current: bool) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let channels = channels.inner().clone(); channels.core.begin_project_change()?;
    let snapshot = channels.core.snapshot(); let core = channels.core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        let title = if change_current { "Change project folder" } else { "Import project folder" };
        let selected = app.dialog().file().set_title(title).set_directory(&snapshot.project_path).blocking_pick_folder();
        let Some(selected) = selected else { return Ok(false) };
        let path = selected.into_path().map_err(|e| e.to_string())?;
        let project = {
            let store = channels.store.lock().unwrap_or_else(|e| e.into_inner());
            if change_current {
                store.change_project_folder(&discard_core::model::ProjectId(snapshot.project_id), &path).map_err(|e| e.to_string())?;
                store.selected_project().map_err(|e| e.to_string())?.ok_or("Project unavailable")?
            } else { store.import_project_folder(&path).map_err(|e| e.to_string())? }
        };
        channels.activate_project(&project.id)?;
        if change_current { channels.reload_project(&project.id)?; }
        Ok(true)
    }).await.map_err(|e| e.to_string()).and_then(|r| r);
    core.end_project_change(); result
}

#[tauri::command]
pub fn get_conversations(context: State<'_, super::settings::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    let direct_messages = store.direct_message_agents().map_err(|e| e.to_string())?;
    store.all_conversations().map_err(|e| e.to_string())?.into_iter().map(|channel| {
        let agent = store.load_agent(&channel.agent_id).map_err(|e| e.to_string())?;
        let archived = store.channel_archived(&channel.id).map_err(|e| e.to_string())?;
        let agent_archived = store.agent_archived(&channel.agent_id).ok();
        let is_default = store.default_conversation(&channel.agent_id).map(|c| c.id == channel.id).unwrap_or(false);
        let in_dm_list = channel.project_id.is_none() && direct_messages.contains(&channel.agent_id);
        Ok(serde_json::json!({"inDmList":in_dm_list,"isDefault":is_default,"id":channel.id.0,"name":channel.name,"agentId":channel.agent_id.0,"agent":agent.name,"provider":agent.provider,"model":agent.model,"projectId":channel.project_id.map(|p|p.0),"workspaceMode":channel.workspace_mode.as_str(),"archived":archived,"agentArchived":agent_archived == Some(true),"deleted":agent_archived.is_none()}))
    }).collect()
}

#[tauri::command]
pub async fn search_agent_history(context: State<'_, super::settings::SettingsContext>, agent_id: String, query: String) -> Result<discard_core::store::HistorySearch, String> {
    let store = context.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store.lock().unwrap_or_else(|e| e.into_inner())
            .search_direct_history(&discard_core::model::AgentId(agent_id), &query)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn create_conversation(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, agent_id: String, project_id: Option<String>, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.create_conversation(&discard_core::model::AgentId(agent_id), project_id.map(discard_core::model::ProjectId).as_ref(), &name)).await.map_err(|e| e.to_string())?;
    // Creation can persist before runtime selection fails. Refresh every catalog in either case.
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    result
}
#[tauri::command]
pub async fn open_agent_chat(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, agent_id: String) -> Result<String, String> {
    use tauri::Emitter;
    let channels = channels.inner().clone();
    let id = tauri::async_runtime::spawn_blocking(move || channels.open_agent(&discard_core::model::AgentId(agent_id))).await.map_err(|e| e.to_string())??;
    let _ = app.emit("discard://conversations", ());
    Ok(id)
}

#[tauri::command]
pub async fn restore_conversation(channels: State<'_, Arc<super::channels::Channels>>, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.restore_conversation(&channel_id)).await.map_err(|e| e.to_string())?
}
