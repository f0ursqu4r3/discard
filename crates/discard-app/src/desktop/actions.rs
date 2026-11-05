//! Execute the pure action plans for the desktop window.
use super::{core::AppCore, plan::{self, Confirm, Plan}};
use discard_core::{action::Action, context_menu::ContextTarget, model::ChannelId, settings::SettingsStore, workspace::GitWorkspace};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{AppHandle, Manager};

pub type Relaunch = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;
pub struct DesktopActions {
    pub settings: Arc<Mutex<SettingsStore>>,
    pub workspace: Arc<GitWorkspace>,
    pub channel_id: ChannelId,
    pub relaunch: Relaunch,
    pub exiting: AtomicBool,
}
impl DesktopActions {
    pub fn confirms(&self) -> bool { self.settings.lock().unwrap_or_else(|e| e.into_inner()).settings().general.confirm_while_working }
}
/// Prepare a quit or relaunch before ending the running turn.
pub fn perform(app: &AppHandle, core: &AppCore, actions: &DesktopActions, ending: Confirm) {
    match plan::prepare_exit(ending, Some(actions.relaunch.as_ref())) {
        Ok(()) => {
            if core.menu_context().turn_running { core.cancel(); }
            actions.exiting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        Err(error) => core.notice(format!("Could not relaunch: {error}")),
    }
}
/// Start the archive operation away from the UI thread.
pub fn archive(core: Arc<AppCore>, actions: &DesktopActions, force: bool) {
    if !core.archive_started() { return; }
    let workspace = actions.workspace.clone();
    let channel_id = actions.channel_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        core.archive_finished(plan::archive_step(workspace.archive(&channel_id, force)));
    });
}
/// Run a core action. Copy results return to the webview for the Web Clipboard API.
pub fn run(app: &AppHandle, core: &Arc<AppCore>, actions: &DesktopActions, action: Action, target: Option<ContextTarget>) -> Option<String> {
    let ctx = core.menu_context();
    match plan::plan_targeted(action, target.is_some(), ctx.turn_running, ctx.archived) {
        plan::TargetedPlan::Copy => return target.as_ref().and_then(|target| core.resolve_copy(target, action)),
        plan::TargetedPlan::Archive => { archive(core.clone(), actions, false); return None; }
        plan::TargetedPlan::ArchiveBlocked => { core.notice("Stop the turn before archiving the channel".into()); return None; }
        plan::TargetedPlan::Nothing => {}
    }
    match plan::plan(action, ctx.turn_running, actions.confirms(), ctx.safe_mode) {
        Plan::OpenSettings(page) => if let Err(error) = super::windows::open_settings(app, page) { core.notice(format!("Cannot open Settings: {error}")); },
        Plan::CancelTurn => core.cancel(),
        Plan::ToggleMemberList => { let _ = core.set_member_list_visible(!core.snapshot().member_list_visible); }
        Plan::Ask(ending) => core.ask(ending),
        Plan::Perform(ending) => perform(app, core, actions, ending),
        Plan::Minimize => if let Some(window) = app.get_webview_window("main") { let _ = window.minimize(); },
        Plan::Zoom => if let Some(window) = app.get_webview_window("main") { if window.is_maximized().unwrap_or(false) { let _ = window.unmaximize(); } else { let _ = window.maximize(); } },
        Plan::CloseWindow => if let Some(window) = app.get_webview_window("main") { let _ = window.close(); },
        Plan::HideApp => { #[cfg(target_os = "macos")] let _ = app.hide(); #[cfg(not(target_os = "macos"))] if let Some(window) = app.get_webview_window("main") { let _ = window.hide(); } },
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
