//! The one place that runs a core `Action`. Menus, keys, and Rust-drawn buttons all call `run`.

use crate::Relaunch;
use crate::settings::open_settings;
use crate::shared::Shared;
use crate::view::DiscardView;
use discard_core::action::{Action, needs_confirm};
use discard_core::settings_page::SettingsPage;
use gpui_kit::{AnyWindowHandle, App, Global, WeakEntity};

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
    Nothing,
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

/// Run `action`.
///
/// Do not call this while the main view is being updated (for example from a `cx.listener` on it):
/// `run` reads the view. Plain `on_click` closures, menu items, and key bindings are safe.
pub fn run(action: Action, cx: &mut App) {
    let main = cx
        .try_global::<MainWindow>()
        .map(|m| (m.handle, m.view.clone()));
    let view = main.as_ref().and_then(|(_, view)| view.upgrade());
    let turn_running = view.as_ref().is_some_and(|v| v.read(cx).turn_running());
    let (confirm_while_working, safe_mode) = match cx.try_global::<Shared>() {
        Some(shared) => (
            shared
                .settings
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .settings()
                .general
                .confirm_while_working,
            shared.load_report.safe_mode,
        ),
        None => (true, false),
    };
    match plan(action, turn_running, confirm_while_working, safe_mode) {
        Plan::OpenSettings(page) => open_settings(page, cx),
        Plan::CancelTurn => {
            if let Some(view) = view {
                view.update(cx, |view, cx| {
                    view.cancel();
                    cx.notify();
                });
            }
        }
        Plan::Perform(ending) => perform(ending, cx),
        Plan::Ask(ending) => match (view, main) {
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
        },
        Plan::Nothing => {}
    }
}

/// End the process. A relaunch first starts the waiting child. If that fails, the app stays open
/// and the main window shows the error.
pub(crate) fn perform(ending: Confirm, cx: &mut App) {
    let relaunch = cx.try_global::<Relauncher>().map(|r| r.0.clone());
    match prepare_exit(ending, relaunch.as_deref()) {
        Ok(()) => cx.quit(),
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
    use super::{Confirm, Plan, plan, prepare_exit};
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
