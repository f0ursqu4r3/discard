//! The one place that runs a core `Action`. Menus, keys, and Rust-drawn buttons all call `run`.

use crate::Relaunch;
use crate::settings::open_settings;
use crate::shared::Shared;
use crate::view::DiscardView;
use discard_core::action::{Action, needs_confirm};
use discard_core::context_menu::ContextTarget;
use discard_core::workspace::ArchiveOutcome;
use discard_core::settings_page::SettingsPage;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity, Window};

/// An action that ends this process, and so may need the user to confirm it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Quit,
    Relaunch { safe_mode: bool },
}

impl Confirm {
    /// The question the confirm notice asks.
    pub fn message(self) -> &'static str {
        match self {
            Confirm::Quit => "The agent is working. Quit Discard and stop the turn?",
            Confirm::Relaunch { safe_mode: true } => {
                "The agent is working. Relaunch in safe mode and stop the turn?"
            }
            Confirm::Relaunch { safe_mode: false } => {
                "The agent is working. Relaunch normally and stop the turn?"
            }
        }
    }
}

/// What `run` does for one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    OpenSettings(SettingsPage),
    CancelTurn,
    ToggleMemberList,
    /// End the process now.
    Perform(Confirm),
    /// Show the confirm notice in the main window.
    Ask(Confirm),
    HideApp,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    /// Close the focused window. `close_plan` decides whether the main window may close.
    CloseWindow,
    Nothing,
}

/// What a request to close a window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Close {
    /// Close the window.
    Allow,
    /// Keep the window open and show the confirm notice.
    Ask(Confirm),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetedPlan { Copy, Archive, ArchiveBlocked, Nothing }

pub(crate) fn plan_targeted(action: Action, has_target: bool, turn_running: bool, archived: bool) -> TargetedPlan {
    if action.needs_target() { return if has_target { TargetedPlan::Copy } else { TargetedPlan::Nothing }; }
    if action == Action::ArchiveChannel {
        return if archived { TargetedPlan::Nothing } else if turn_running { TargetedPlan::ArchiveBlocked } else { TargetedPlan::Archive };
    }
    TargetedPlan::Nothing
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArchiveStep { Done(String), NeedsConfirm(Vec<String>), Failed(String) }

pub(crate) fn archive_step(result: Result<ArchiveOutcome, String>) -> ArchiveStep {
    match result {
        Ok(ArchiveOutcome::Archived { kept_branch: Some(branch) }) => ArchiveStep::Done(format!("Channel archived. The branch `{branch}` is kept. Relaunch to start a new channel.")),
        Ok(ArchiveOutcome::Archived { kept_branch: None }) => ArchiveStep::Done("Channel archived. Relaunch to start a new channel.".into()),
        Ok(ArchiveOutcome::NeedsConfirmation(items)) => ArchiveStep::NeedsConfirm(items),
        Err(error) => ArchiveStep::Failed(error),
    }
}

/// Decide whether a window may close. Closing the main window ends the app, so it follows the Quit rule.
/// Pure, so the rule is testable without a window.
pub(crate) fn close_plan(is_main: bool, turn_running: bool, confirm_while_working: bool) -> Close {
    if is_main && needs_confirm(Action::Quit, turn_running, confirm_while_working) {
        Close::Ask(Confirm::Quit)
    } else {
        Close::Allow
    }
}

/// Decide what to do. Pure, so the rules are testable without a window.
pub(crate) fn plan(
    action: Action,
    turn_running: bool,
    confirm_while_working: bool,
    safe_mode: bool,
) -> Plan {
    let ending = match action {
        Action::ArchiveChannel | Action::CopyMessageText | Action::CopyCode | Action::CopyChannelName | Action::CopyDiff | Action::CopyToolInput | Action::CopyToolOutput | Action::CopyAgentName | Action::CopyProjectName | Action::CopyProjectPath => return Plan::Nothing,
        Action::OpenSettings => return Plan::OpenSettings(SettingsPage::General),
        Action::ToggleMemberList => return Plan::ToggleMemberList,
        Action::OpenAgentSettings => return Plan::OpenSettings(SettingsPage::Agents),
        Action::OpenWorkspaceSettings => return Plan::OpenSettings(SettingsPage::Workspaces),
        Action::ReviewPlugins | Action::ShowPluginErrors => {
            return Plan::OpenSettings(SettingsPage::Plugins);
        }
        Action::CancelTurn if turn_running => return Plan::CancelTurn,
        Action::CancelTurn => return Plan::Nothing,
        Action::HideApp => return Plan::HideApp,
        Action::HideOthers => return Plan::HideOthers,
        Action::ShowAll => return Plan::ShowAll,
        Action::Minimize => return Plan::Minimize,
        Action::Zoom => return Plan::Zoom,
        Action::CloseWindow => return Plan::CloseWindow,
        Action::Quit => Confirm::Quit,
        Action::ToggleSafeMode => Confirm::Relaunch {
            safe_mode: !safe_mode,
        },
    };
    if needs_confirm(action, turn_running, confirm_while_working) {
        Plan::Ask(ending)
    } else {
        Plan::Perform(ending)
    }
}

/// The relaunch callback from the app. It starts a process that waits for this one to exit.
pub(crate) struct Relauncher(pub(crate) Relaunch);

impl Global for Relauncher {}

/// Prepare to end the process for `ending`. `Ok` means quit now. `Err` holds the message to show, and the app stays open.
/// Pure apart from `relaunch`, so the rule "a failed relaunch never quits" is testable.
pub(crate) fn prepare_exit(
    ending: Confirm,
    relaunch: Option<&(dyn Fn(bool) -> Result<(), String> + Send + Sync)>,
) -> Result<(), String> {
    match ending {
        Confirm::Quit => Ok(()),
        Confirm::Relaunch { safe_mode } => match relaunch {
            Some(relaunch) => relaunch(safe_mode),
            None => Err("relaunch is not available".to_string()),
        },
    }
}

/// The main window and its view, so the dispatcher can reach them from any window.
pub(crate) struct MainWindow {
    pub(crate) handle: AnyWindowHandle,
    pub(crate) view: WeakEntity<DiscardView>,
}

impl Global for MainWindow {}

/// What the dispatcher reads from the app before it decides.
struct State {
    main: Option<(AnyWindowHandle, WeakEntity<DiscardView>)>,
    view: Option<Entity<DiscardView>>,
    turn_running: bool,
    confirm_while_working: bool,
    safe_mode: bool,
}

fn state(cx: &mut App) -> State {
    let main = cx
        .try_global::<MainWindow>()
        .map(|m| (m.handle, m.view.clone()));
    let view = main.as_ref().and_then(|(_, view)| view.upgrade());
    let turn_running = view.as_ref().is_some_and(|v| v.read(cx).turn_running());
    let (confirm_while_working, safe_mode) = match cx.try_global::<Shared>() {
        Some(shared) => {
            let mut store = shared.settings.lock().unwrap_or_else(|p| p.into_inner());
            // Only the Settings window polls the file. Read it again here, so a hand edit applies while Settings is closed.
            store.reload_if_changed();
            (
                store.settings().general.confirm_while_working,
                shared.load_report.safe_mode,
            )
        }
        None => (true, false),
    };
    State {
        main,
        view,
        turn_running,
        confirm_while_working,
        safe_mode,
    }
}

/// Run `action`.
///
/// Do not call this while the main view is being updated (for example from a `cx.listener` on it):
/// `run` reads the view. Plain `on_click` closures, menu items, and key bindings are safe.
pub fn run(action: Action, cx: &mut App) {
    run_on(action, None, cx);
}

pub fn run_on(action: Action, target: Option<ContextTarget>, cx: &mut App) {
    let state = state(cx);
    if action.needs_target() || action == Action::ArchiveChannel {
        let Some(view) = state.view else { return; };
        match plan_targeted(action, target.is_some(), state.turn_running, view.read(cx).archived()) {
            TargetedPlan::Copy => {
                if let Some(text) = target.as_ref().and_then(|target| view.read(cx).resolve_copy(target, action)) {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                }
            }
            TargetedPlan::Archive => start_archive(false, cx),
            TargetedPlan::ArchiveBlocked => view.update(cx, |view, cx| view.show_notice("Cannot archive a channel while a turn runs.".into(), cx)),
            TargetedPlan::Nothing => {}
        }
        return;
    }
    match plan(
        action,
        state.turn_running,
        state.confirm_while_working,
        state.safe_mode,
    ) {
        Plan::OpenSettings(page) => open_settings(page, cx),
        Plan::CancelTurn => {
            if let Some(view) = state.view {
                view.update(cx, |view, cx| {
                    view.cancel();
                    cx.notify();
                });
            }
        }
        Plan::ToggleMemberList => {
            if let Some(view) = state.view { view.update(cx, |view, cx| view.toggle_member_list(cx)); }
        }
        Plan::Perform(ending) => perform(ending, cx),
        Plan::Ask(ending) => ask(ending, state, cx),
        Plan::HideApp => cx.hide(),
        Plan::HideOthers => cx.hide_other_apps(),
        Plan::ShowAll => cx.unhide_other_apps(),
        Plan::Minimize => with_active_window(cx, |window| window.minimize_window()),
        Plan::Zoom => with_active_window(cx, |window| window.zoom_window()),
        Plan::CloseWindow => {
            let Some(active) = cx.active_window() else {
                return;
            };
            let is_main = state
                .main
                .as_ref()
                .is_some_and(|(handle, _)| *handle == active);
            match close_plan(is_main, state.turn_running, state.confirm_while_working) {
                // `remove_window` skips the platform's should-close hook, so this check is the confirm for Cmd+W.
                // For the main window, the `on_window_closed` observer then quits. The Settings window closes alone.
                Close::Allow => with_active_window(cx, |window| window.remove_window()),
                Close::Ask(ending) => ask(ending, state, cx),
            }
        }
        Plan::Nothing => {}
    }
}

fn start_archive(force: bool, cx: &mut App) {
    let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) else { return; };
    if view.read(cx).turn_running() || view.read(cx).archived() || view.read(cx).archive_busy() { return; }
    let Some(shared) = cx.try_global::<Shared>() else { return; };
    let workspace = shared.workspace.clone();
    let channel = shared.channel_id.clone();
    view.update(cx, |view, cx| view.set_archive_busy(true, cx));
    let task = cx.background_spawn(async move { workspace.archive(&channel, force) });
    cx.spawn(async move |cx| {
        let result = archive_step(task.await);
        view.update(cx, |view, cx| {
            view.set_archive_busy(false, cx);
            match result {
                ArchiveStep::Done(message) => { view.set_archived(cx); view.show_notice(message, cx); }
                ArchiveStep::NeedsConfirm(items) => view.set_pending_archive(Some(items), cx),
                ArchiveStep::Failed(error) => view.show_notice(format!("Could not archive channel: {error}"), cx),
            }
        });
    }).detach();
}

pub(crate) fn confirm_archive(cx: &mut App) {
    if let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) {
        view.update(cx, |view, cx| view.set_pending_archive(None, cx));
        start_archive(true, cx);
    }
}

pub(crate) fn cancel_archive(cx: &mut App) {
    if let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) {
        view.update(cx, |view, cx| view.set_pending_archive(None, cx));
    }
}

pub(crate) fn relaunch_same_mode(cx: &mut App) {
    let safe_mode = state(cx).safe_mode;
    perform(Confirm::Relaunch { safe_mode }, cx);
}

fn plugin_item_notice(plugin: &str, error: &str) -> String { format!("{plugin}: {error}") }

pub(crate) fn run_plugin_item(plugin: &str, index: usize, target: Option<&ContextTarget>, cx: &mut App) {
    let Some(shared) = cx.try_global::<Shared>() else { return; };
    let kind = target.map_or("app", |target| target.kind().id());
    let id = target.map_or_else(String::new, ContextTarget::target_id);
    let result = shared.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).run_menu_item(plugin, index, kind, &id);
    if let Err(error) = result {
        *shared.plugin_notice.lock().unwrap_or_else(|p| p.into_inner()) = Some(plugin_item_notice(plugin, &error.to_string()));
    }
    shared.plugin_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Run `f` on the focused window after the current update ends.
/// A key or menu action can arrive from inside that window's update, and GPUI cannot update a window from inside
/// its own update.
fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window) + 'static) {
    if let Some(active) = cx.active_window() {
        cx.defer(move |cx| {
            let _ = active.update(cx, |_, window, _| f(window));
        });
    }
}

/// Show the confirm notice for `ending` in the main window and bring that window to the front.
fn ask(ending: Confirm, state: State, cx: &mut App) {
    match (state.view, state.main) {
        (Some(view), Some((handle, _))) => {
            view.update(cx, |view, cx| view.set_pending_confirm(Some(ending), cx));
            // The main window may be the one dispatching this key or click right now, and GPUI cannot update
            // a window from inside its own update. Activate it after the current update ends.
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
        }
        // No main view means no turn to protect.
        _ => perform(ending, cx),
    }
}

/// The main window's should-close hook (the red close button). `true` lets the window close, and then the app
/// quits. `false` keeps it open and shows the confirm notice.
pub(crate) fn main_window_should_close(cx: &mut App) -> bool {
    let state = state(cx);
    match close_plan(true, state.turn_running, state.confirm_while_working) {
        Close::Allow => true,
        Close::Ask(ending) => {
            ask(ending, state, cx);
            false
        }
    }
}

/// Send `Cancel` to the engine when a turn runs. Best effort before a quit: the process ends soon after,
/// so the provider may not stop its child processes in time.
pub(crate) fn cancel_running_turn(cx: &mut App) {
    let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) else {
        return;
    };
    if view.read(cx).turn_running() {
        view.update(cx, |view, _| view.cancel());
    }
}

/// End the process. A relaunch first starts the waiting child. If that fails, the app stays open
/// and the main window shows the error.
pub(crate) fn perform(ending: Confirm, cx: &mut App) {
    let relaunch = cx.try_global::<Relauncher>().map(|r| r.0.clone());
    match prepare_exit(ending, relaunch.as_deref()) {
        Ok(()) => {
            cancel_running_turn(cx);
            cx.quit();
        }
        Err(error) => {
            tracing::warn!(%error, "could not relaunch Discard");
            if let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) {
                view.update(cx, |view, cx| {
                    view.show_notice(format!("Could not relaunch Discard: {error}"), cx)
                });
            }
        }
    }
}

/// The confirm notice's Confirm button: run the waiting action.
pub(crate) fn confirm_pending(cx: &mut App) {
    let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) else {
        return;
    };
    let pending = view.update(cx, |view, cx| {
        let pending = view.pending_confirm();
        view.set_pending_confirm(None, cx);
        pending
    });
    if let Some(ending) = pending {
        perform(ending, cx);
    }
}

/// The confirm notice's Cancel button: forget the waiting action.
pub(crate) fn cancel_pending(cx: &mut App) {
    if let Some(view) = cx.try_global::<MainWindow>().and_then(|m| m.view.upgrade()) {
        view.update(cx, |view, cx| view.set_pending_confirm(None, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveStep, Close, Confirm, Plan, TargetedPlan, archive_step, close_plan, plan, plan_targeted, prepare_exit};
    use discard_core::workspace::ArchiveOutcome;
    use discard_core::action::Action;
    use discard_core::settings_page::SettingsPage;
    use std::cell::Cell;

    #[test] fn open_agent_and_workspace_settings_open_their_pages() { assert_eq!(plan(Action::OpenAgentSettings,false,true,false),Plan::OpenSettings(SettingsPage::Agents)); assert_eq!(plan(Action::OpenWorkspaceSettings,false,true,false),Plan::OpenSettings(SettingsPage::Workspaces)); }
    #[test] fn plan_targeted_copies_for_every_copy_action() { for &action in Action::all() { if action.needs_target() { assert_eq!(plan_targeted(action,true,false,false),TargetedPlan::Copy); assert_eq!(plan_targeted(action,false,false,false),TargetedPlan::Nothing); } } }
    #[test] fn the_new_copy_actions_need_a_target() { for a in [Action::CopyDiff,Action::CopyToolInput,Action::CopyToolOutput,Action::CopyAgentName,Action::CopyProjectName,Action::CopyProjectPath] { assert_eq!(plan_targeted(a,false,false,false),TargetedPlan::Nothing); assert_eq!(plan_targeted(a,true,false,false),TargetedPlan::Copy); } }
    #[test]
    fn copy_actions_need_a_target_and_archive_obeys_state() {
        assert_eq!(plan_targeted(Action::CopyCode, false, false, false), TargetedPlan::Nothing);
        assert_eq!(plan_targeted(Action::CopyCode, true, false, false), TargetedPlan::Copy);
        assert_eq!(plan_targeted(Action::ArchiveChannel, false, true, false), TargetedPlan::ArchiveBlocked);
        assert_eq!(plan_targeted(Action::ArchiveChannel, false, false, true), TargetedPlan::Nothing);
        assert_eq!(plan_targeted(Action::ArchiveChannel, false, false, false), TargetedPlan::Archive);
    }

    #[test]
    fn archive_step_maps_results() {
        assert_eq!(archive_step(Ok(ArchiveOutcome::Archived { kept_branch: None })), ArchiveStep::Done("Channel archived. Relaunch to start a new channel.".into()));
        assert_eq!(archive_step(Ok(ArchiveOutcome::Archived { kept_branch: Some("branch".into()) })), ArchiveStep::Done("Channel archived. The branch `branch` is kept. Relaunch to start a new channel.".into()));
        assert_eq!(archive_step(Ok(ArchiveOutcome::NeedsConfirmation(vec!["file".into()]))), ArchiveStep::NeedsConfirm(vec!["file".into()]));
        assert_eq!(archive_step(Err("error".into())), ArchiveStep::Failed("error".into()));
    }

    #[test]
    fn a_failing_plugin_item_sets_the_plugin_notice() {
        assert_eq!(super::plugin_item_notice("notes", "oops"), "notes: oops");
    }

    #[test]
    fn settings_actions_open_their_page_in_any_state() {
        for running in [false, true] {
            for confirm in [false, true] {
                for safe in [false, true] {
                    assert_eq!(
                        plan(Action::OpenSettings, running, confirm, safe),
                        Plan::OpenSettings(SettingsPage::General)
                    );
                    assert_eq!(
                        plan(Action::ReviewPlugins, running, confirm, safe),
                        Plan::OpenSettings(SettingsPage::Plugins)
                    );
                    assert_eq!(
                        plan(Action::ShowPluginErrors, running, confirm, safe),
                        Plan::OpenSettings(SettingsPage::Plugins)
                    );
                }
            }
        }
    }

    #[test]
    fn cancel_turn_does_nothing_while_idle() {
        assert_eq!(plan(Action::CancelTurn, false, true, false), Plan::Nothing);
        assert_eq!(
            plan(Action::CancelTurn, true, true, false),
            Plan::CancelTurn
        );
        assert_eq!(
            plan(Action::CancelTurn, true, false, false),
            Plan::CancelTurn
        );
    }

    #[test]
    fn quit_asks_only_while_working_with_confirm_on() {
        assert_eq!(
            plan(Action::Quit, true, true, false),
            Plan::Ask(Confirm::Quit)
        );
        assert_eq!(
            plan(Action::Quit, true, false, false),
            Plan::Perform(Confirm::Quit)
        );
        assert_eq!(
            plan(Action::Quit, false, true, false),
            Plan::Perform(Confirm::Quit)
        );
        assert_eq!(
            plan(Action::Quit, false, false, true),
            Plan::Perform(Confirm::Quit)
        );
    }

    #[test]
    fn safe_mode_toggle_relaunches_with_the_opposite_flag() {
        assert_eq!(
            plan(Action::ToggleSafeMode, false, true, false),
            Plan::Perform(Confirm::Relaunch { safe_mode: true })
        );
        assert_eq!(
            plan(Action::ToggleSafeMode, false, true, true),
            Plan::Perform(Confirm::Relaunch { safe_mode: false })
        );
        assert_eq!(
            plan(Action::ToggleSafeMode, true, true, true),
            Plan::Ask(Confirm::Relaunch { safe_mode: false })
        );
        assert_eq!(
            plan(Action::ToggleSafeMode, true, false, false),
            Plan::Perform(Confirm::Relaunch { safe_mode: true })
        );
    }

    #[test]
    fn closing_the_main_window_asks_only_while_working_with_confirm_on() {
        assert_eq!(close_plan(true, true, true), Close::Ask(Confirm::Quit));
        assert_eq!(close_plan(true, true, false), Close::Allow);
        assert_eq!(close_plan(true, false, true), Close::Allow);
        assert_eq!(close_plan(true, false, false), Close::Allow);
    }

    #[test]
    fn closing_another_window_never_asks() {
        for running in [false, true] {
            for confirm in [false, true] {
                assert_eq!(close_plan(false, running, confirm), Close::Allow);
            }
        }
    }

    #[test]
    fn window_and_app_actions_need_no_confirm() {
        for running in [false, true] {
            for confirm in [false, true] {
                assert_eq!(
                    plan(Action::HideApp, running, confirm, false),
                    Plan::HideApp
                );
                assert_eq!(
                    plan(Action::HideOthers, running, confirm, false),
                    Plan::HideOthers
                );
                assert_eq!(
                    plan(Action::ShowAll, running, confirm, false),
                    Plan::ShowAll
                );
                assert_eq!(
                    plan(Action::Minimize, running, confirm, false),
                    Plan::Minimize
                );
                assert_eq!(plan(Action::Zoom, running, confirm, false), Plan::Zoom);
                assert_eq!(
                    plan(Action::CloseWindow, running, confirm, false),
                    Plan::CloseWindow
                );
            }
        }
    }

    #[test]
    fn quit_needs_no_relaunch() {
        assert_eq!(prepare_exit(Confirm::Quit, None), Ok(()));
    }

    #[test]
    fn relaunch_calls_back_with_the_target_safe_mode() {
        thread_local!(static SEEN: Cell<Option<bool>> = const { Cell::new(None) });
        let ok = |safe: bool| {
            SEEN.with(|s| s.set(Some(safe)));
            Ok(())
        };
        assert_eq!(
            prepare_exit(Confirm::Relaunch { safe_mode: true }, Some(&ok)),
            Ok(())
        );
        assert_eq!(SEEN.with(Cell::get), Some(true));
        assert_eq!(
            prepare_exit(Confirm::Relaunch { safe_mode: false }, Some(&ok)),
            Ok(())
        );
        assert_eq!(SEEN.with(Cell::get), Some(false));
    }

    #[test]
    fn a_failed_or_missing_relaunch_does_not_quit() {
        let fail = |_: bool| Err("no sh".to_string());
        assert_eq!(
            prepare_exit(Confirm::Relaunch { safe_mode: true }, Some(&fail)),
            Err("no sh".to_string())
        );
        assert!(prepare_exit(Confirm::Relaunch { safe_mode: false }, None).is_err());
    }

    #[test]
    fn confirm_messages_name_the_ending() {
        assert!(Confirm::Quit.message().contains("Quit"));
        assert!(
            Confirm::Relaunch { safe_mode: true }
                .message()
                .contains("safe mode")
        );
        assert!(
            Confirm::Relaunch { safe_mode: false }
                .message()
                .contains("normally")
        );
    }
}
