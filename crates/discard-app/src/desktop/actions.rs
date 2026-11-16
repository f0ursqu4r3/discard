//! Execute the pure action plans for the desktop window.
use super::{core::AppCore, plan::{self, Confirm, Plan}, view::ViewId};
use discard_core::{action::Action, context_menu::ContextTarget, model::ChannelId, settings::SettingsStore, workspace::GitWorkspace};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{AppHandle, Emitter, Manager};

pub type Relaunch = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

/// One provider and the last result of its setup check.
#[derive(Clone)]
pub struct ProviderInfo {
    pub provider: Arc<dyn discard_core::provider::Provider>,
    pub id: String,
    pub matrix: discard_core::capability::CapabilityMatrix,
    pub setup: Arc<Mutex<Option<discard_core::provider::SetupStatus>>>,
}

/// Runs the setup check for one provider again.
pub type Recheck = Arc<dyn Fn(&str) + Send + Sync>;

pub struct DesktopActions {
    pub settings: Arc<Mutex<SettingsStore>>,
    pub workspace: Arc<GitWorkspace>,
    pub relaunch: Relaunch,
    pub exiting: AtomicBool,
    pub terminals: Arc<super::terminal::Terminals>,
}
impl DesktopActions {
    pub fn confirms(&self) -> bool { self.settings.lock().unwrap_or_else(|e| e.into_inner()).settings().general.confirm_while_working }
}
/// Prepare a quit or relaunch before ending the running turn.
pub fn perform(app: &AppHandle, core: &AppCore, actions: &DesktopActions, ending: Confirm) {
    match plan::prepare_exit(ending, Some(actions.relaunch.as_ref())) {
        Ok(()) => {
            if core.any_running() { core.cancel_all(); }
            actions.exiting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        Err(error) => core.notice(&core.focused_view(), format!("Could not relaunch: {error}")),
    }
}
/// Start the archive operation away from the UI thread.
pub fn archive(core: Arc<AppCore>, actions: &DesktopActions, view: &ViewId, force: bool) {
    archive_for(core, actions, view, force, None);
}
fn archive_for(core: Arc<AppCore>, actions: &DesktopActions, view: &ViewId, force: bool, channel: Option<&str>) {
    if !core.archive_started_for(view, channel) { return; }
    let workspace = actions.workspace.clone();
    let terminals = actions.terminals.clone();
    let channel_id = ChannelId(core.active_channel_id(view));
    let view = view.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = workspace.archive(&channel_id, force);
        if matches!(outcome, Ok(discard_core::workspace::ArchiveOutcome::Archived { .. })) { terminals.close_channel(&channel_id.0, Some("This channel was archived. The session ended.")); }
        core.archive_finished(&view, plan::archive_step(outcome));
    });
}
/// Run a core action. Copy results return to the webview for the Web Clipboard API.
pub fn run(app: &AppHandle, core: &Arc<AppCore>, actions: &DesktopActions, view: &ViewId, action: Action, target: Option<ContextTarget>) -> Option<String> {
    let channel = match &target { Some(ContextTarget::Channel { id }) => Some(id.as_str()), _ => None };
    if !plan::channel_target_allowed(action, channel, &core.active_channel_id(view)) { return None; }
    let ctx = core.menu_context(view);
    // The turn state of the target channel decides a stop. Other actions use the active channel.
    let turn_running = match channel { Some(id) if action == Action::CancelTurn => core.channel_running(id), _ => ctx.turn_running };
    match plan::plan_targeted(action, target.is_some(), ctx.turn_running, ctx.archived) {
        plan::TargetedPlan::Copy => return target.as_ref().and_then(|target| core.resolve_copy(view, target, action)),
        plan::TargetedPlan::Archive => {
            if let Some(ContextTarget::Channel { id }) = &target { archive_for(core.clone(), actions, view, false, Some(id)); }
            return None;
        }
        plan::TargetedPlan::ArchiveBlocked => { core.notice(view, "Stop the turn before archiving the channel".into()); return None; }
        plan::TargetedPlan::Nothing => {}
    }
    match plan::plan(action, if matches!(action, Action::Quit | Action::ToggleSafeMode) { core.any_running() } else { turn_running }, actions.confirms(), ctx.safe_mode) {
        Plan::OpenSettings(page) => if let Err(error) = super::windows::open_settings(app, page) { core.notice(view, format!("Cannot open Settings: {error}")); },
        Plan::OpenAgentProfile => {
            let id = match &target {
                Some(ContextTarget::Agent { id, .. }) => id.clone(),
                _ => core.snapshot(view).agent_id,
            };
            // The window of the view shows the profile.
            let _ = app.emit_to(view.label(), "discard://agent-profile", id);
            super::windows::show_view_window(app, view);
        }
        Plan::CancelTurn => match channel { Some(id) => core.cancel_in(Some(id)), None => core.cancel(view) },
        Plan::ToggleTerminal => { let _ = app.emit_to(view.label(), "discard://toggle-terminal", ()); }
        Plan::ToggleMemberList => { let _ = core.set_member_list_visible(view, !core.snapshot(view).member_list_visible); }
        Plan::Ask(ending) => super::windows::ask_visibly(app, core, ending),
        Plan::Perform(ending) => perform(app, core, actions, ending),
        Plan::Minimize => if let Some(window) = app.get_webview_window(view.label()) { let _ = window.minimize(); },
        Plan::Zoom => if let Some(window) = app.get_webview_window(view.label()) { if window.is_maximized().unwrap_or(false) { let _ = window.unmaximize(); } else { let _ = window.maximize(); } },
        // The close request decides: a window that is not the last closes alone, and the last one follows the Quit rule.
        Plan::CloseWindow => if let Some(window) = app.get_webview_window(view.label()) { let _ = window.close(); },
        Plan::ShowMainWindow => super::windows::show_main_window(app),
        Plan::HideApp => { #[cfg(target_os = "macos")] let _ = app.hide(); #[cfg(not(target_os = "macos"))] if let Some(window) = app.get_webview_window(view.label()) { let _ = window.hide(); } },
        Plan::HideOthers => other_apps(app, false),
        Plan::ShowAll => other_apps(app, true),
        Plan::Nothing => {},
    }
    None
}

/// Ask macOS to hide or show the other applications on the main thread.
#[cfg(target_os = "macos")]
fn other_apps(app: &AppHandle, show: bool) {
    let _ = app.run_on_main_thread(move || {
        if let Some(main_thread) = objc2::MainThreadMarker::new() {
            let application = objc2_app_kit::NSApplication::sharedApplication(main_thread);
            if show { application.unhideAllApplications(None); } else { application.hideOtherApplications(None); }
        }
    });
}
#[cfg(not(target_os = "macos"))]
fn other_apps(_app: &AppHandle, _show: bool) {}
