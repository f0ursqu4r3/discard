use super::{
    core::{AppCore, WINDOW_CLOSED},
    dto::{HistoryDto, SnapshotDto},
    view::ViewId,
};
use std::sync::Arc;
use tauri::State;

/// The view that a command acts on, from the label of the calling window. An app window with a view gets that view.
/// An app window with no view is closed: `Err`. Any other window (Settings) gets the focused view.
fn resolve_view(label: &str, has_view: impl Fn(&ViewId) -> bool, focused: ViewId) -> Result<ViewId, String> {
    match ViewId::from_label(label) {
        Some(view) if has_view(&view) => Ok(view),
        Some(_) => Err(WINDOW_CLOSED.into()),
        None => Ok(focused),
    }
}
fn view_of(window: &tauri::WebviewWindow, core: &AppCore) -> Result<ViewId, String> {
    resolve_view(window.label(), |view| core.has_view(view), core.focused_view())
}
/// The view of a command that returns no error. `None` when the window is closed: the call acts on no view, not
/// even the main view.
fn quiet_view(label: &str, has_view: impl Fn(&ViewId) -> bool, focused: ViewId) -> Option<ViewId> {
    resolve_view(label, has_view, focused).ok()
}
fn view_quiet(window: &tauri::WebviewWindow, core: &AppCore) -> Option<ViewId> {
    quiet_view(window.label(), |view| core.has_view(view), core.focused_view())
}
#[tauri::command]
pub fn get_snapshot(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>) -> SnapshotDto {
    view_quiet(&window, &core).map_or_else(|| core.closed_snapshot(), |view| core.snapshot(&view))
}
#[tauri::command]
pub fn get_history(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>) -> HistoryDto {
    view_quiet(&window, &core).map_or_else(|| core.closed_history(), |view| core.history_snapshot(&view))
}
#[tauri::command]
pub fn send_message(channels: State<'_, Arc<super::channels::Channels>>, context: State<'_, super::settings::SettingsContext>, channel_id: String, text: String, attachment_ids: Option<Vec<String>>, also_post: Option<bool>) -> Result<(), String> {
    let attachments = discard_core::attachments::load_all(&context.home, &attachment_ids.unwrap_or_default())?;
    channels.send_with_options(&channel_id, text, attachments, also_post.unwrap_or(false))
}

/// The open thread's transcript, or null when no thread is open. The `discard://thread-history` event sends the same value.
#[tauri::command]
pub fn get_thread_history(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>) -> Option<super::dto::ThreadHistoryDto> {
    core.thread_history_snapshot(&view_quiet(&window, &core)?)
}

/// Start a thread on a saved message of the selected channel, send its first message, and open it. Returns the thread channel ID.
#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn start_thread(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, context: State<'_, super::settings::SettingsContext>, parent_channel_id: String, anchor_message_id: String, text: String, attachment_ids: Option<Vec<String>>, also_post: Option<bool>) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let home = context.home.clone();
    let id = tauri::async_runtime::spawn_blocking(move || {
        let attachments = discard_core::attachments::load_all(&home, &attachment_ids.unwrap_or_default())?;
        channels.start_thread(&view, &parent_channel_id, &anchor_message_id, text, attachments, also_post.unwrap_or(false))
    }).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    Ok(id)
}

/// Show a thread of the selected channel beside it.
#[tauri::command]
pub async fn open_thread(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.open_thread(&view, &discard_core::model::ChannelId(thread_id))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn close_thread(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>) {
    if let Some(view) = view_quiet(&window, &core) { core.close_thread(&view); }
}

/// Select the parent channel of a followed thread and open the thread beside it. Returns the parent channel ID.
#[tauri::command]
pub async fn select_thread(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let (changed, parent) = tauri::async_runtime::spawn_blocking(move || channels.select_thread(&view, &discard_core::model::ChannelId(thread_id))).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    // The view that changed is the main view when the thread is outside the project of a project window.
    super::windows::show_conversation(&app, &changed);
    Ok(parent)
}

/// Show a thread message in the thread's parent channel. Any user or agent message. Posting again changes nothing.
/// Returns the ID of the reference row in the parent channel.
#[tauri::command]
pub async fn post_thread_message(channels: State<'_, Arc<super::channels::Channels>>, message_id: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.post_thread_message(&message_id)).await.map_err(|e| e.to_string())?
}

async fn follow(app: tauri::AppHandle, channels: Arc<super::channels::Channels>, thread_id: String, followed: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || channels.set_thread_followed(&discard_core::model::ChannelId(thread_id), followed)).await.map_err(|e| e.to_string())??;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    Ok(())
}
/// Follow a thread: it shows in the channel bar under its parent.
#[tauri::command]
pub async fn follow_thread(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<(), String> {
    follow(app, channels.inner().clone(), thread_id, true).await
}
/// Leave a thread: it leaves the channel bar. Its chip stays.
#[tauri::command]
pub async fn leave_thread(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<(), String> {
    follow(app, channels.inner().clone(), thread_id, false).await
}

/// Give a thread a new name. Returns the stored name. The name is trimmed, line breaks become spaces, and the length
/// is 1 to 80 characters.
#[tauri::command]
pub async fn rename_thread(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.rename_thread(&discard_core::model::ChannelId(thread_id), &name)).await.map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    result
}

/// How many messages of a thread were posted to its channel. A delete keeps these in the channel.
#[tauri::command]
pub async fn thread_posted_count(channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<usize, String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.thread_posted_count(&discard_core::model::ChannelId(thread_id))).await.map_err(|e| e.to_string())?
}

/// Delete a thread with its messages. A message that was posted to the channel stays there. Returns how many stayed.
#[tauri::command]
pub async fn delete_thread(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, thread_id: String) -> Result<usize, String> {
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.delete_thread(&discard_core::model::ChannelId(thread_id))).await.map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    result
}

/// Remove a posted reference row from its channel. The message stays in its thread.
#[tauri::command]
pub async fn remove_posted_message(channels: State<'_, Arc<super::channels::Channels>>, message_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.remove_posted_message(&message_id)).await.map_err(|e| e.to_string())?
}

/// Rename a text channel. Returns the stored name.
#[tauri::command]
pub async fn rename_text_channel(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.rename_text_channel(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id), &name)).await.map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    result
}

/// Tell the webview that the channel lists changed.
fn conversations_changed(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
}

/// Mute or unmute a channel. A muted channel shows no unread marker.
#[tauri::command]
pub async fn set_channel_muted(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String, muted: bool) -> Result<(), String> {
    let channels = channels.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.set_channel_muted(&discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id), muted)).await.map_err(|e| e.to_string())?;
    conversations_changed(&app);
    result
}

/// Archive the channel of a row. `archived` is false when the archive needs the second confirm: call again with `force`.
#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn archive_project_channel(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, workspace: State<'_, Arc<discard_core::workspace::GitWorkspace>>, terminals: State<'_, Arc<super::terminal::Terminals>>, project_id: String, channel_id: String, force: bool) -> Result<super::dto::ArchiveChannelDto, String> {
    let (channels, workspace) = (channels.inner().clone(), workspace.inner().clone());
    let view = view_of(&window, &channels.core)?;
    let closing = channel_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.archive_channel(&view, &workspace, &discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id), force)).await.map_err(|e| e.to_string())?;
    if result.as_ref().is_ok_and(|archived| archived.archived) { terminals.close_channel(&closing, Some("This channel was archived. The session ended.")); }
    conversations_changed(&app);
    result
}

/// Where Create Worktree… will put the worktree of a branch.
#[tauri::command]
pub async fn worktree_plan(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, workspace: State<'_, Arc<discard_core::workspace::GitWorkspace>>, project_id: String, repository: String, branch: String) -> Result<super::dto::WorktreePlanDto, String> {
    let (channels, workspace) = (channels.inner().clone(), workspace.inner().clone());
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.worktree_plan(&view, &workspace, &discard_core::model::ProjectId(project_id), &repository, &branch)).await.map_err(|e| e.to_string())?
}

/// Create a linked worktree for a branch. Returns its path.
#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn create_branch_worktree(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, workspace: State<'_, Arc<discard_core::workspace::GitWorkspace>>, project_id: String, repository: String, branch: String, channel_id: Option<String>) -> Result<String, String> {
    let (channels, workspace) = (channels.inner().clone(), workspace.inner().clone());
    let view = view_of(&window, &channels.core)?;
    let result = tauri::async_runtime::spawn_blocking(move || channels.create_worktree(&view, &workspace, &discard_core::model::ProjectId(project_id), &repository, &branch, channel_id.as_deref())).await.map_err(|e| e.to_string())?;
    conversations_changed(&app);
    result
}

/// Rename a local branch and its channel binding. Returns the new name.
#[tauri::command]
pub async fn rename_git_branch(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, repository: String, branch: String, new_name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let result = tauri::async_runtime::spawn_blocking(move || channels.rename_branch(&view, &discard_core::model::ProjectId(project_id), &repository, &branch, &new_name)).await.map_err(|e| e.to_string())?;
    conversations_changed(&app);
    result
}

/// The unmerged commits that deleting a branch would drop, or the reason it cannot be deleted.
#[tauri::command]
pub async fn branch_deletion_info(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, repository: String, branch: String) -> Result<usize, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.branch_deletion_info(&view, &discard_core::model::ProjectId(project_id), &repository, &branch)).await.map_err(|e| e.to_string())?
}

/// Delete a local branch. Unmerged commits need `force`.
#[tauri::command]
pub async fn delete_git_branch(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, repository: String, branch: String, force: bool) -> Result<super::dto::BranchDeletionDto, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let result = tauri::async_runtime::spawn_blocking(move || channels.delete_branch(&view, &discard_core::model::ProjectId(project_id), &repository, &branch, force)).await.map_err(|e| e.to_string())?;
    conversations_changed(&app);
    result
}

/// Start a new branch in a working tree and switch it to the branch. Returns the new name.
#[tauri::command]
pub async fn create_git_branch(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, repository: String, worktree: String, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let result = tauri::async_runtime::spawn_blocking(move || channels.create_branch(&view, &discard_core::model::ProjectId(project_id), &repository, &worktree, &name)).await.map_err(|e| e.to_string())?;
    conversations_changed(&app);
    result
}

/// The files that removing a working tree would discard, or the reason it cannot be removed.
#[tauri::command]
pub async fn worktree_removal_info(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, repository: String, worktree: String) -> Result<super::dto::WorktreeFilesDto, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.worktree_removal_info(&view, &discard_core::model::ProjectId(project_id), &repository, &worktree)).await.map_err(|e| e.to_string())?
}

/// Remove a linked working tree. The branch stays. Files that would be lost need `force`.
#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn remove_git_worktree(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, terminals: State<'_, Arc<super::terminal::Terminals>>, project_id: String, repository: String, worktree: String, force: bool) -> Result<super::dto::WorktreeRemovalDto, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let folder = std::path::PathBuf::from(&worktree);
    let result = tauri::async_runtime::spawn_blocking(move || channels.remove_worktree(&view, &discard_core::model::ProjectId(project_id), &repository, &worktree, force)).await.map_err(|e| e.to_string())?;
    if result.as_ref().is_ok_and(|removal| removal.removed) { terminals.close_inside(&folder, Some("The worktree of this terminal was removed. The session ended.")); }
    conversations_changed(&app);
    result
}

#[tauri::command]
pub fn get_worktree_requests(access: State<'_, Arc<discard_core::worktree_access::WorktreeAccess>>, channel_id: String) -> Vec<discard_core::worktree_access::RemovalRequest> {
    access.requests(&discard_core::model::ChannelId(channel_id))
}

#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn decide_worktree_removal(
    window: tauri::WebviewWindow,
    access: State<'_, Arc<discard_core::worktree_access::WorktreeAccess>>,
    terminals: State<'_, Arc<super::terminal::Terminals>>,
    channels: State<'_, Arc<super::channels::Channels>>,
    core: State<'_, Arc<AppCore>>,
    channel_id: String, request_id: String, approve: bool,
) -> Result<discard_core::worktree_access::RemovalRequest, String> {
    let access = access.inner().clone();
    let store = channels.store.clone();
    let core = core.inner().clone();
    let view = view_of(&window, &core)?;
    let terminals = terminals.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let channel = discard_core::model::ChannelId(channel_id);
        if approve { core.begin_project_change().map_err(|_| "Wait for active responses and archive operations to finish before removing a worktree.".to_string())?; }
        let decision = access.decide(&channel, &request_id, approve);
        if approve { core.end_project_change(); }
        let request = decision?;
        if approve && request.state == "completed" { terminals.close_channel(&channel.0, Some("The worktree of this terminal was removed. The session ended.")); }
        let message = discard_core::model::Message {
            id: discard_core::model::MessageId::new(), channel_id: channel.clone(), role: discard_core::model::Role::System,
            blocks: vec![discard_core::model::Block::Markdown { text: request.result.clone().unwrap_or_else(|| request.state.clone()) }],
        };
        store.lock().unwrap_or_else(|e| e.into_inner()).insert_message(&message).map_err(|e| format!("Action status: {}. Could not save its transcript note: {e}", request.state))?;
        core.apply_channel(&channel.0, discard_core::ui_state::Update::ConversationMarker(message));
        core.refresh_branches_in_background(&view);
        Ok(request)
    }).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn decide_approval(core: State<'_, Arc<AppCore>>, channel_id: String, approval_id: String, decision: String, rule_cut: Option<usize>) {
    core.decide_in(Some(&channel_id), approval_id, decision, rule_cut);
}
#[tauri::command]
pub fn cancel_turn(core: State<'_, Arc<AppCore>>, channel_id: String) {
    core.cancel_in(Some(&channel_id));
}

#[tauri::command]
pub fn set_panels(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, channel_list: f32, member_list: f32) -> Result<(), String> {
    core.set_panels(&view_of(&window, &core)?, channel_list, member_list)
}
#[tauri::command]
pub fn set_member_list_visible(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, visible: bool) -> Result<(), String> {
    core.set_member_list_visible(&view_of(&window, &core)?, visible)
}
#[tauri::command]
pub fn toggle_branch(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, key: String) {
    if let Some(view) = view_quiet(&window, &core) { core.toggle_branch(&view, key); }
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
pub fn archive_confirm(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, accept: bool) {
    let Some(view) = view_quiet(&window, &core) else { return };
    if !core.has_pending_archive(&view) { return; }
    if accept { super::actions::archive(core.inner().clone(), &actions, &view, true); } else { core.cancel_archive(&view); }
}
#[tauri::command]
pub fn context_menu(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, target: super::dto::ContextTargetDto) -> Vec<super::dto::MenuEntryDto> {
    let Some(view) = view_quiet(&window, &core) else { return vec![] };
    discard_core::context_menu::context_menu(&target.into(), &core.menu_context(&view)).into_iter().filter_map(super::dto::MenuEntryDto::from_entry).collect()
}
#[tauri::command]
pub fn run_action(app: tauri::AppHandle, window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, action_id: String, target: Option<super::dto::ContextTargetDto>) -> Option<String> {
    let view = view_quiet(&window, &core)?;
    if action_id == "open_provider_settings" {
        if let Err(error) = super::windows::open_settings(&app, discard_core::settings_page::SettingsPage::Providers) { core.notice(&view, format!("Cannot open CLI connections: {error}")); }
        return None;
    }
    if action_id == "relaunch" {
        let ctx = core.menu_context(&view);
        let ending = super::plan::Confirm::Relaunch { safe_mode: ctx.safe_mode };
        if core.any_running() && actions.confirms() { core.ask(ending); }
        else { super::actions::perform(&app, &core, &actions, ending); }
        return None;
    }
    let action = discard_core::action::Action::from_id(&action_id)?;
    super::actions::run(&app, core.inner(), &actions, &view, action, target.map(Into::into))
}
#[tauri::command]
pub fn plugin_menu_action(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, plugin: String, index: usize, target: super::dto::ContextTargetDto) -> Result<(), String> {
    let view = view_of(&window, &core)?;
    let target = target.into();
    let allowed = discard_core::context_menu::context_menu(&target, &core.menu_context(&view)).iter().any(|item| matches!(item, discard_core::action::MenuEntry::Plugin { plugin: p, index: i, .. } if p == &plugin && *i == index));
    if !allowed { return Err("plugin menu item is unavailable".into()); }
    match core.run_plugin_menu(&view, &plugin, index, Some(&target)) {
        Ok(()) => Ok(()),
        Err(error) => { core.plugin_failure(&plugin, &error); Err(error) }
    }
}

/// The slash commands: built-ins first, then plugin commands.
#[tauri::command]
pub async fn list_commands(core: State<'_, Arc<AppCore>>) -> Result<Vec<super::dto::CommandDto>, String> {
    Ok(discard_core::slash::registry(&core.plugin_commands()).0.into_iter().map(Into::into).collect())
}

/// Run a slash command in a channel. Returns the text to show, if any.
#[tauri::command]
pub async fn run_command(app: tauri::AppHandle, window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<super::actions::DesktopActions>>, channel_id: String, name: String, args: String) -> Result<Option<String>, String> {
    use discard_core::{action::Action, context_menu::ContextTarget, slash::{self, Builtin}};
    use super::slash::{Step, builtin_step};
    if !slash::valid_name(&name) { return Err(format!("Unknown command: /{name}")); }
    let view = view_of(&window, &core)?;
    let channel = core.command_channel(&channel_id).ok_or("The channel is not open.")?;
    if channel.archived { return Err("The channel is archived.".into()); }
    let (entries, shadowed) = slash::registry(&core.plugin_commands());
    for lost in &shadowed { tracing::warn!(plugin = %lost.plugin, command = %lost.name, by = %lost.by, "plugin command shadowed"); }
    if let Some(builtin) = Builtin::from_name(&name) {
        return match builtin_step(builtin, &channel, &entries) {
            Step::Output(text) => Ok(Some(slash::truncate_output(text))),
            Step::Error(text) => Err(text),
            Step::Dispatch(action) => {
                // Stop and archive act on the channel. The other actions are app-wide, and a channel target would drop them in a thread.
                let target = matches!(action, Action::CancelTurn | Action::ArchiveChannel).then(|| ContextTarget::Channel { id: channel_id });
                super::actions::run(&app, core.inner(), &actions, &view, action, target);
                Ok(None)
            }
        };
    }
    let owner = entries.iter().find(|entry| entry.name == name).map(|entry| entry.source.clone());
    let run_core = core.inner().clone();
    let (run_name, run_args) = (name.clone(), args);
    let result = tauri::async_runtime::spawn_blocking(move || run_core.run_plugin_command(&channel_id, &run_name, &run_args)).await.map_err(|e| e.to_string())?;
    match result {
        Some(Ok(text)) => Ok(text.map(slash::truncate_output)),
        Some(Err(error)) => {
            if let Some(plugin) = owner { core.plugin_failure(&plugin, &error); }
            Err(slash::truncate_output(error))
        }
        None => Err(format!("Unknown command: /{name}")),
    }
}

#[tauri::command]
pub fn plugin_action(window: tauri::WebviewWindow, core: State<'_, Arc<AppCore>>, plugin: String, action: String) -> Result<(), String> {
    core.plugin_action(&view_of(&window, &core)?, &plugin, &action)
}

#[tauri::command]
pub fn web_shortcuts(actions: State<'_, Arc<super::actions::DesktopActions>>) -> Vec<(String, String)> {
    let settings = actions.settings.lock().unwrap_or_else(|e| e.into_inner());
    super::menu::web_shortcuts(&settings.settings().keymap)
}

#[tauri::command]
pub async fn create_channel(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, name: String, workspace_mode: String, source_channel: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.create(&view, &name, &workspace_mode, &source_channel)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn switch_channel(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let (changed, in_project) = tauri::async_runtime::spawn_blocking(move || channels.select(&view, &channel_id).map(|changed| (changed, channels.in_project(&channel_id)))).await.map_err(|e| e.to_string())??;
    // A project channel is in the lists already. Only a direct message can change the catalogs.
    if !in_project {
        use tauri::Emitter;
        let _ = app.emit("discard://conversations", ());
    }
    // The view that changed gets the event. It is the main view when a project window asked for a target outside its project.
    super::windows::show_conversation(&app, &changed);
    Ok(())
}
#[tauri::command]
pub async fn create_text_channel(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let selecting = view.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.create_text_channel(&selecting, &discard_core::model::ProjectId(project_id), &name)).await.map_err(|e| e.to_string())?;
    // Creation can persist before runtime selection fails. Refresh every catalog in either case.
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    // The new channel is open. Show the conversation page.
    if let Ok(id) = &result { let _ = app.emit_to(view.label(), "discard://conversation-opened", id.clone()); }
    result
}
#[tauri::command]
pub async fn open_branch_channel(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, key: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.open_branch(&view, &key)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_projects(context: State<'_, super::settings::SettingsContext>) -> Result<Vec<serde_json::Value>, String> {
    let store = context.store.lock().unwrap_or_else(|e| e.into_inner());
    store.visible_projects().map_err(|e| e.to_string())?.into_iter().map(|p| {
        let avatar = store.project_avatar(&p.id).map_err(|e| e.to_string())?;
        let main = store.project_main_agent(&p.id).map_err(|e| e.to_string())?;
        let show_archived = store.project_show_archived(&p.id).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"id":p.id.0,"name":p.name,"path":p.path,"avatar":avatar,"mainAgentId":main.map(|id| id.0),"showArchived":show_archived}))
    }).collect()
}

#[tauri::command]
pub fn update_project_profile(app: tauri::AppHandle, core: State<'_, Arc<AppCore>>, context: State<'_, super::settings::SettingsContext>, project_id: String, name: String, avatar: Option<String>) -> Result<(), String> {
    let id = discard_core::model::ProjectId(project_id);
    context.store.lock().unwrap_or_else(|e| e.into_inner())
        .update_project_profile(&id, &name, avatar.as_deref()).map_err(|e| e.to_string())?;
    core.update_project_name(&id.0, name.trim());
    super::windows::set_project_window_title(&app, &id, name.trim());
    use tauri::Emitter;
    let _ = app.emit("discard://projects", ());
    Ok(())
}
#[tauri::command]
pub fn set_project_show_archived(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, project_id: String, show: bool) -> Result<(), String> {
    context.store.lock().unwrap_or_else(|e| e.into_inner())
        .set_project_show_archived(&discard_core::model::ProjectId(project_id), show).map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("discard://projects", ());
    Ok(())
}
#[tauri::command]
pub fn get_rail_layout(context: State<'_, super::settings::SettingsContext>) -> Result<serde_json::Value, String> {
    use discard_core::store::RailItem;
    let layout = context.store.lock().unwrap_or_else(|e| e.into_inner()).rail_layout().map_err(|e| e.to_string())?;
    let items: Vec<_> = layout.items.into_iter().map(|item| match item {
        RailItem::Project(id) => serde_json::json!({"kind":"project","projectId":id.0}),
        RailItem::Group(g) => serde_json::json!({"kind":"group","id":g.id,"name":g.name,"color":g.color,"collapsed":g.collapsed,"projectIds":g.project_ids.into_iter().map(|id| id.0).collect::<Vec<_>>()}),
    }).collect();
    Ok(serde_json::json!({"items":items}))
}
/// Run one rail mutation on the store and tell the web to reload the projects.
fn change_rail<T>(app: &tauri::AppHandle, context: &super::settings::SettingsContext, change: impl FnOnce(&discard_core::store::Store) -> discard_core::store::Result<T>) -> Result<T, String> {
    let out = change(&context.store.lock().unwrap_or_else(|e| e.into_inner())).map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("discard://projects", ());
    Ok(out)
}
/// Move a project into a group (`group_id` set) or to the top level (`group_id` empty). `index` counts the list without the project.
#[tauri::command]
pub fn move_rail_project(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, project_id: String, group_id: Option<String>, index: usize) -> Result<(), String> {
    let target = group_id.map_or(discard_core::store::RailTarget::TopLevel, discard_core::store::RailTarget::Group);
    change_rail(&app, &context, |store| store.rail_move_project(&discard_core::model::ProjectId(project_id), &target, index))
}
/// Drop `dragged_id` onto the ungrouped project `target_id`. Returns the new group ID.
#[tauri::command]
pub fn group_rail_projects(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, dragged_id: String, target_id: String) -> Result<String, String> {
    change_rail(&app, &context, |store| store.rail_group_projects(&discard_core::model::ProjectId(dragged_id), &discard_core::model::ProjectId(target_id)))
}
/// Put one project in a new group. Returns the new group ID.
#[tauri::command]
pub fn create_rail_group(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, project_id: String) -> Result<String, String> {
    change_rail(&app, &context, |store| store.rail_group_project(&discard_core::model::ProjectId(project_id)))
}
#[tauri::command]
pub fn move_rail_group(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, group_id: String, index: usize) -> Result<(), String> {
    change_rail(&app, &context, |store| store.rail_move_group(&group_id, index))
}
#[tauri::command]
pub fn rename_rail_group(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, group_id: String, name: String) -> Result<(), String> {
    change_rail(&app, &context, |store| store.rail_rename_group(&group_id, &name))
}
#[tauri::command]
pub fn set_rail_group_color(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, group_id: String, color: Option<String>) -> Result<(), String> {
    change_rail(&app, &context, |store| store.rail_set_group_color(&group_id, color.as_deref()))
}
#[tauri::command]
pub fn set_rail_group_collapsed(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, group_id: String, collapsed: bool) -> Result<(), String> {
    change_rail(&app, &context, |store| store.rail_set_group_collapsed(&group_id, collapsed))
}
#[tauri::command]
pub fn ungroup_rail_group(app: tauri::AppHandle, context: State<'_, super::settings::SettingsContext>, group_id: String) -> Result<(), String> {
    change_rail(&app, &context, |store| store.rail_ungroup(&group_id))
}
#[tauri::command]
pub async fn select_project(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let caller = view_of(&window, &channels.core)?;
    let project = discard_core::model::ProjectId(project_id);
    // A project view that asks for another project changes the main view, so the guard belongs to that view.
    let view = channels.core.route(&caller, Some(&project));
    channels.core.begin_project_navigation(&view)?;
    let core = channels.core.clone();
    let navigating = view.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.activate_project(&navigating, &project)).await.map_err(|e| e.to_string()).and_then(|r| r);
    core.end_project_navigation(&view);
    // The first value is the view that changed.
    let (changed, channel_id) = result?;
    use tauri::Emitter;
    let _ = app.emit_to(changed.label(), "discard://conversation-opened", channel_id.clone());
    let _ = app.emit("discard://conversations", ());
    super::windows::reveal_changed_view(&app, &caller, &changed);
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
pub async fn select_canonical_branch(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, branch: String, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.select_canonical_branch(&view, &discard_core::model::ProjectId(project_id), &branch, &discard_core::model::ChannelId(channel_id))).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn select_canonical_project_channel(window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, project_id: String, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.select_canonical_general(&view, &discard_core::model::ProjectId(project_id), &discard_core::model::ChannelId(channel_id))).await.map_err(|e| e.to_string())?
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
pub async fn open_project_window(app: tauri::AppHandle, channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    // An async command, so the window is not built on the thread of the event loop. The view opens on a blocking thread.
    tauri::async_runtime::spawn_blocking(move || super::windows::open_project_window(&app, &channels, &discard_core::model::ProjectId(project_id))).await.map_err(|e| e.to_string())?
}
/// Show and focus the main window. With `page`, the main window also gets `discard://navigate` with the page.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle, page: Option<String>) {
    use tauri::Emitter;
    super::windows::show_main_window(&app);
    if let Some(page) = page { let _ = app.emit_to(super::view::ViewId::main().label(), "discard://navigate", page); }
}
#[tauri::command]
pub async fn remove_project(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, terminals: State<'_, Arc<super::terminal::Terminals>>, project_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    // The window that asks must be open. The project window of the removed project closes first, so the next project
    // never applies to it.
    view_of(&window, &channels.core)?;
    channels.core.begin_project_change()?;
    super::windows::close_project_window(&app, &channels.core, &discard_core::model::ProjectId(project_id.clone()));
    let core = channels.core.clone();
    let (terminals, closing) = (terminals.inner().clone(), project_id.clone());
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let id = discard_core::model::ProjectId(project_id);
        // A project view never changes project: only the main view moves to the next project.
        channels.leave_removed_project(&id)?;
        channels.store.lock().unwrap_or_else(|e| e.into_inner()).hide_project(&id).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string()).and_then(|r| r);
    core.end_project_change();
    if result.is_ok() { terminals.close_project(&closing, Some("This project was removed. The session ended.")); }
    result
}
#[tauri::command]
pub async fn choose_project_folder(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, change_current: bool) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    channels.core.begin_project_change()?;
    let snapshot = channels.core.snapshot(&view); let core = channels.core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        let title = if change_current { "Change project folder" } else { "Import project folder" };
        let selected = app.dialog().file().set_title(title).set_directory(&snapshot.project_path).blocking_pick_folder();
        let Some(selected) = selected else { return Ok(false) };
        let path = selected.into_path().map_err(|e| e.to_string())?;
        let project = {
            let store = channels.store.lock().unwrap_or_else(|e| e.into_inner());
            if change_current {
                // The project of the calling view, not the project that the store has selected (the main view's).
                let id = discard_core::model::ProjectId(snapshot.project_id);
                store.change_project_folder(&id, &path).map_err(|e| e.to_string())?;
                store.visible_projects().map_err(|e| e.to_string())?.into_iter().find(|project| project.id == id).ok_or("Project unavailable")?
            } else { store.import_project_folder(&path).map_err(|e| e.to_string())? }
        };
        channels.activate_project(&view, &project.id)?;
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
        let threads = super::threads::stored_links(&store, &channel.id, |id| context.core.last_read(id))?;
        Ok(serde_json::json!({"threads":threads,"inDmList":in_dm_list,"isDefault":is_default,"id":channel.id.0,"name":channel.name,"agentId":channel.agent_id.0,"agent":agent.name,"avatar":agent.avatar,"provider":agent.provider,"model":agent.model,"projectId":channel.project_id.map(|p|p.0),"workspaceMode":channel.workspace_mode.as_str(),"archived":archived,"agentArchived":agent_archived == Some(true),"deleted":agent_archived.is_none()}))
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
pub async fn create_conversation(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, agent_id: String, project_id: Option<String>, name: String) -> Result<String, String> {
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    let project = project_id.map(discard_core::model::ProjectId);
    // The view that selects the conversation: a project view fixed to another project leads to the main view.
    let changed = channels.core.route(&view, project.as_ref());
    let selecting = view.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.create_conversation(&selecting, &discard_core::model::AgentId(agent_id), project.as_ref(), &name)).await.map_err(|e| e.to_string())?;
    // Creation can persist before runtime selection fails. Refresh every catalog in either case.
    use tauri::Emitter;
    let _ = app.emit("discard://conversations", ());
    if result.is_ok() && super::windows::view_to_reveal(&view, &changed).is_some() { super::windows::show_conversation(&app, &changed); }
    result
}
#[tauri::command]
pub async fn open_agent_chat(app: tauri::AppHandle, window: tauri::WebviewWindow, channels: State<'_, Arc<super::channels::Channels>>, agent_id: String) -> Result<String, String> {
    use tauri::Emitter;
    let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    // A direct message has no project: a project view leads to the main view.
    let changed = channels.core.route(&view, None);
    let selecting = view.clone();
    let id = tauri::async_runtime::spawn_blocking(move || channels.open_agent(&selecting, &discard_core::model::AgentId(agent_id))).await.map_err(|e| e.to_string())??;
    let _ = app.emit("discard://conversations", ());
    if super::windows::view_to_reveal(&view, &changed).is_some() { super::windows::show_conversation(&app, &changed); }
    Ok(id)
}

#[tauri::command]
pub async fn restore_conversation(channels: State<'_, Arc<super::channels::Channels>>, channel_id: String) -> Result<(), String> {
    let channels = channels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || channels.restore_conversation(&channel_id)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_always_allow_rules(channels: State<'_, Arc<super::channels::Channels>>, project_id: String) -> Result<Vec<super::dto::RuleDto>, String> {
    let store = channels.store.lock().unwrap_or_else(|e| e.into_inner());
    store.always_allow_rules(&discard_core::model::ProjectId(project_id)).map_err(|e| e.to_string())?.into_iter().map(|rule| {
        let agent = store.load_agent(&rule.agent).map_err(|e| e.to_string())?;
        let (kind, text) = match rule.target {
            discard_core::always_allow::RuleTarget::Command { words } => ("command", words.join(" ")),
            discard_core::always_allow::RuleTarget::Host { host } => ("host", host),
        };
        Ok(super::dto::RuleDto {
            id: rule.id,
            agent_id: rule.agent.0,
            agent_name: agent.name,
            kind: kind.into(),
            text,
            created_at: rule.created_at,
        })
    }).collect()
}

#[tauri::command]
pub fn revoke_always_allow_rule(channels: State<'_, Arc<super::channels::Channels>>, project_id: String, rule_id: String) -> Result<bool, String> {
    channels.store.lock().unwrap_or_else(|e| e.into_inner()).delete_always_allow_rule(&discard_core::model::ProjectId(project_id), &rule_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_write_requests(requests: State<'_, Arc<discard_core::write_requests::WriteRequests>>, channel_id: String) -> Vec<discard_core::write_requests::WriteRequest> {
    requests.list(&discard_core::model::ChannelId(channel_id))
}

#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "Tauri passes one argument per command field.")]
pub async fn decide_write_request(
    window: tauri::WebviewWindow,
    grants: State<'_, Arc<discard_core::workspace_grants::MainCheckoutGrants>>,
    requests: State<'_, Arc<discard_core::write_requests::WriteRequests>>,
    workspace: State<'_, Arc<discard_core::workspace::GitWorkspace>>,
    channels: State<'_, Arc<super::channels::Channels>>,
    channel_id: String, request_id: String, approve: bool,
) -> Result<discard_core::write_requests::WriteRequest, String> {
    let grants = grants.inner().clone();
    let requests = requests.inner().clone(); let workspace = workspace.inner().clone(); let channels = channels.inner().clone();
    let view = view_of(&window, &channels.core)?;
    tauri::async_runtime::spawn_blocking(move || channels.decide_write_request(&view, &requests, &workspace, &grants, &channel_id, &request_id, approve)).await.map_err(|e| e.to_string())?
}

/// The requests of agents in `channel_id` to add an agent to the project, oldest first, decided or not.
/// The card shows a pending one. Requests are not stored: none survives an app restart.
#[tauri::command]
pub fn get_agent_requests(requests: State<'_, Arc<discard_core::agent_requests::AgentRequests>>, channel_id: String) -> Vec<super::dto::AgentRequestDto> {
    requests.list(&discard_core::model::ChannelId(channel_id)).into_iter().map(Into::into).collect()
}

/// Decide a pending request: `add` makes the agent a project member and continues the requesting agent, `deny`
/// continues it with the note that the user declined. The channel must be the selected channel and idle.
/// Refreshes `discard://channel-details` (members) and `discard://conversations`.
#[tauri::command]
pub async fn decide_agent_request(
    app: tauri::AppHandle,
    requests: State<'_, Arc<discard_core::agent_requests::AgentRequests>>,
    channels: State<'_, Arc<super::channels::Channels>>,
    channel_id: String, request_id: String, decision: String,
) -> Result<super::dto::AgentRequestDto, String> {
    let approve = match decision.as_str() { "add" => true, "deny" => false, _ => return Err("Decide with add or deny.".into()) };
    let requests = requests.inner().clone(); let channels = channels.inner().clone();
    let channel = channel_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || channels.decide_agent_request(&requests, &channel, &request_id, approve)).await.map_err(|e| e.to_string())?;
    // The decision can save and its follow-up still fail, so refresh in either case.
    use tauri::Emitter;
    let _ = app.emit("discard://channel-details", channel_id);
    let _ = app.emit("discard://conversations", ());
    result.map(Into::into)
}

/// The changes of the folder of a channel, for the Changes tab. `scope` is `uncommitted` or `branch`.
/// It reads Git only, and it runs off the main thread.
#[tauri::command]
pub async fn channel_changes(channels: State<'_, Arc<super::channels::Channels>>, channel_id: String, scope: String) -> Result<super::dto::ChannelChangesDto, String> {
    let channels = channels.inner().clone();
    let scope = discard_core::workspace_diff::Scope::parse(&scope)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (folder, _) = super::terminal::cwd::for_channel(&channels.store, &discard_core::model::ChannelId(channel_id)).map_err(|error| {
            if error == super::terminal::cwd::NO_PROJECT { "Direct messages have no project, so they have no changes.".to_owned() } else { error }
        })?;
        discard_core::workspace_diff::channel_changes(&folder, scope)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::model::ProjectId;

    #[test]
    fn a_command_acts_on_the_view_of_the_calling_window() { // Review Focus 2
        let project = ViewId::project(&ProjectId("p".into()));
        let has = |view: &ViewId| *view == project || view.is_main();
        assert_eq!(resolve_view("project-p", has, ViewId::main()), Ok(project.clone()));
        // A reload of the webview of the project window keeps its view, whatever window has the focus.
        let other = ViewId::project(&ProjectId("q".into()));
        assert_eq!(resolve_view("project-p", has, other.clone()), Ok(project.clone()));
        assert_eq!(resolve_view("main", has, other), Ok(ViewId::main()));
    }
    #[test]
    fn a_closed_project_window_gets_an_error_and_no_other_view() {
        let main_only = |view: &ViewId| view.is_main();
        assert_eq!(resolve_view("project-p", main_only, ViewId::main()), Err("This window is closed.".to_string()));
        assert_eq!(resolve_view("main", |_| false, ViewId::main()), Err("This window is closed.".to_string()));
    }
    #[test]
    fn a_quiet_command_of_a_closed_window_acts_on_no_view() {
        let main_only = |view: &ViewId| view.is_main();
        assert_eq!(quiet_view("project-p", main_only, ViewId::main()), None, "a closed project window does not fall back to the main view");
        assert_eq!(quiet_view("main", |_| false, ViewId::main()), None);
        assert_eq!(quiet_view("main", main_only, ViewId::main()), Some(ViewId::main()));
        let focused = ViewId::project(&ProjectId("p".into()));
        assert_eq!(quiet_view("settings", |_| false, focused.clone()), Some(focused));
    }
    #[test]
    fn a_window_that_is_not_an_app_window_gets_the_focused_view() {
        let focused = ViewId::project(&ProjectId("p".into()));
        assert_eq!(resolve_view("settings", |_| false, focused.clone()), Ok(focused.clone()));
        assert_eq!(resolve_view("project-", |_| false, focused.clone()), Ok(focused), "a label that is not canonical is not an app window");
    }
}
