//! The one place that runs a core `Action`. Menus, keys, and Rust-drawn buttons all call `run`.

use crate::Relaunch;
use crate::settings::open_settings;
use crate::shared::Shared;
use crate::view::DiscardView;
use discard_core::action::{Action, needs_confirm};
use discard_core::settings_page::SettingsPage;
use gpui_kit::{AnyWindowHandle, App, Entity, Global, WeakEntity, Window};

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
        Action::OpenSettings => return Plan::OpenSettings(SettingsPage::General),
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
    let state = state(cx);
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
    use super::{Close, Confirm, Plan, close_plan, plan, prepare_exit};
    use discard_core::action::Action;
    use discard_core::settings_page::SettingsPage;
    use std::cell::Cell;

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
