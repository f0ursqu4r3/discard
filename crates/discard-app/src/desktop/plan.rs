//! Pure decisions for desktop actions.
use discard_core::action::{Action, needs_confirm};
use discard_core::workspace::ArchiveOutcome;
use discard_core::settings_page::SettingsPage;

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
pub enum Plan {
    OpenSettings(SettingsPage),
    OpenAgentProfile,
    CancelTurn,
    ToggleMemberList,
    ToggleTerminal,
    /// End the process now.
    Perform(Confirm),
    /// Show the confirm notice in the focused window.
    Ask(Confirm),
    HideApp,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    /// Close the window of the view. The close request goes through `close_plan`.
    CloseWindow,
    /// Show and focus the main window.
    ShowMainWindow,
    Nothing,
}

/// What a request to close a window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// Close the window.
    Allow,
    /// Keep the window and hide it. The main window hides, so its view and its place stay.
    Hide,
    /// End the app now. This is the last app window, and the Quit rule needs no question.
    Quit,
    /// Keep the window open and show the confirm notice.
    Ask(Confirm),
}

/// The kind of a window that asks to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Project,
    /// Any other window, such as Settings.
    Other,
}

/// The kind of the window with this label.
pub fn window_kind(label: &str) -> WindowKind {
    match super::view::ViewId::from_label(label) {
        Some(view) if view.is_main() => WindowKind::Main,
        Some(_) => WindowKind::Project,
        None => WindowKind::Other,
    }
}

/// The visible app windows besides the window with label `closing`, in order. A label that is not an app window, such
/// as Settings, is left out.
pub fn other_app_labels(visible: &[String], closing: &str) -> Vec<String> {
    let mut labels: Vec<String> = visible.iter().filter(|label| label.as_str() != closing && super::view::ViewId::from_label(label).is_some()).cloned().collect();
    labels.sort();
    labels
}

/// How many app windows stay open when the window with label `closing` closes. `visible` lists the labels of the
/// visible windows. A label that is not an app window, such as Settings, does not count.
pub fn other_app_windows(visible: &[String], closing: &str) -> usize { other_app_labels(visible, closing).len() }

/// What the app does so that the focused view belongs to a window the user can see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refocus {
    /// Focus this view. Its window is visible.
    Focus(super::view::ViewId),
    /// No app window is visible: show the main window and focus its view.
    ShowMain,
}

/// Choose how to keep the focus on a visible window. `visible` lists the visible app windows. `None` when the focused
/// view has a visible window. The menu, the shortcuts, and the quit question follow the focused view, so they must
/// not go to a hidden window. The core cannot see the windows: the caller passes what it sees.
pub fn refocus(visible: &[String], focused: &super::view::ViewId) -> Option<Refocus> {
    let visible = other_app_labels(visible, "");
    if visible.iter().any(|label| label == focused.label()) { return None; }
    Some(match visible.first().and_then(|label| super::view::ViewId::from_label(label)) {
        Some(view) => Refocus::Focus(view),
        None => Refocus::ShowMain,
    })
}

/// Choose how to keep the focus after an app window closed that no close request handled. It is `refocus`, with two
/// exceptions that show nothing: the app exits, or the app runs in the background and no app window is visible (spec
/// T1). A quit question needs no such exception: it shows the main window through `refocus` itself (spec T6).
pub fn refocus_after_close(visible: &[String], focused: &super::view::ViewId, background: bool, exiting: bool) -> Option<Refocus> {
    if exiting || (background && other_app_labels(visible, "").is_empty()) { return None; }
    refocus(visible, focused)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedPlan { Copy, Archive, ArchiveBlocked, Nothing }

/// Stopping a turn may target a loaded thread. Every other action needs the active channel.
pub fn channel_target_allowed(action: Action, target_channel: Option<&str>, active_channel: &str) -> bool {
    action == Action::CancelTurn || target_channel.is_none_or(|id| id == active_channel)
}

pub fn plan_targeted(action: Action, has_target: bool, turn_running: bool, archived: bool) -> TargetedPlan {
    if action.needs_target() { return if has_target { TargetedPlan::Copy } else { TargetedPlan::Nothing }; }
    if action == Action::ArchiveChannel {
        return if archived { TargetedPlan::Nothing } else if turn_running { TargetedPlan::ArchiveBlocked } else { TargetedPlan::Archive };
    }
    TargetedPlan::Nothing
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveStep { Done(String), NeedsConfirm(Vec<String>), Failed(String) }

pub fn archive_step(result: Result<ArchiveOutcome, String>) -> ArchiveStep {
    match result {
        Ok(ArchiveOutcome::Archived { kept_branch: Some(branch) }) => ArchiveStep::Done(format!("Channel archived. The branch `{branch}` is kept. Relaunch to start a new channel.")),
        Ok(ArchiveOutcome::Archived { kept_branch: None }) => ArchiveStep::Done("Channel archived. Relaunch to start a new channel.".into()),
        Ok(ArchiveOutcome::NeedsConfirmation(items)) => ArchiveStep::NeedsConfirm(items),
        Err(error) => ArchiveStep::Failed(error),
    }
}

/// Decide what a request to close a window does. A window that is not the last app window closes alone: the main
/// window hides and a project window closes. The last app window follows the Quit rule, unless the app runs in the
/// background: then it closes alone too. `other_app_windows` counts the visible app windows besides this one.
/// Pure, so the rule is testable without a window.
pub fn close_plan(kind: WindowKind, other_app_windows: usize, background: bool, turn_running: bool, confirm_while_working: bool) -> Close {
    if kind == WindowKind::Other { return Close::Allow; }
    if other_app_windows > 0 || background {
        return if kind == WindowKind::Main { Close::Hide } else { Close::Allow };
    }
    if needs_confirm(Action::Quit, turn_running, confirm_while_working) { Close::Ask(Confirm::Quit) } else { Close::Quit }
}

/// Decide what to do. Pure, so the rules are testable without a window.
pub fn plan(
    action: Action,
    turn_running: bool,
    confirm_while_working: bool,
    safe_mode: bool,
) -> Plan {
    let ending = match action {
        Action::ArchiveChannel | Action::CopyMessageText | Action::CopyCode | Action::CopyChannelName | Action::CopyDiff | Action::CopyToolInput | Action::CopyToolOutput | Action::CopyAgentName | Action::CopyProjectName | Action::CopyProjectPath => return Plan::Nothing,
        Action::OpenSettings => return Plan::OpenSettings(SettingsPage::General),
        Action::ToggleMemberList => return Plan::ToggleMemberList,
        Action::ToggleTerminal => return Plan::ToggleTerminal,
        Action::OpenAgentSettings => return Plan::OpenAgentProfile,
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
        Action::ShowMainWindow => return Plan::ShowMainWindow,
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

pub fn prepare_exit(
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

/// A plugin menu failure becomes a notice.
pub fn plugin_item_notice(plugin: &str, error: &str) -> String { format!("{plugin}: {error}") }

#[cfg(test)]
mod tests {
    use super::{ArchiveStep, Close, Confirm, Plan, TargetedPlan, Refocus, WindowKind, archive_step, channel_target_allowed, close_plan, other_app_labels, other_app_windows, refocus, refocus_after_close, plan, plan_targeted, prepare_exit, window_kind};
    use discard_core::workspace::ArchiveOutcome;
    use discard_core::action::Action;
    use discard_core::settings_page::SettingsPage;
    use std::cell::Cell;

    #[test] fn cancel_turn_may_target_a_channel_that_is_not_active() { assert!(channel_target_allowed(Action::CancelTurn, Some("thread"), "main")); }
    #[test] fn other_actions_need_the_active_channel() {
        assert!(!channel_target_allowed(Action::ArchiveChannel, Some("thread"), "main"));
        for &action in Action::all() {
            assert!(channel_target_allowed(action, Some("main"), "main"));
            assert!(channel_target_allowed(action, None, "main"));
        }
    }
    #[test] fn open_agent_profile_and_workspace_settings() { assert_eq!(plan(Action::OpenAgentSettings,false,true,false),Plan::OpenAgentProfile); assert_eq!(plan(Action::OpenWorkspaceSettings,false,true,false),Plan::OpenSettings(SettingsPage::Workspaces)); }
    #[test] fn the_terminal_toggle_plans_a_toggle_even_while_a_turn_runs() { for running in [false, true] { assert_eq!(plan(Action::ToggleTerminal, running, true, false), Plan::ToggleTerminal); } }
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
    fn a_window_that_is_not_the_last_closes_alone() {
        assert_eq!(close_plan(WindowKind::Main, 1, false, true, true), Close::Hide);
        assert_eq!(close_plan(WindowKind::Project, 1, false, true, true), Close::Allow);
        assert_eq!(close_plan(WindowKind::Other, 0, false, true, true), Close::Allow);
    }

    #[test]
    fn the_last_app_window_follows_the_quit_rule() {
        for kind in [WindowKind::Main, WindowKind::Project] {
            assert_eq!(close_plan(kind, 0, false, true, true), Close::Ask(Confirm::Quit));
            assert_eq!(close_plan(kind, 0, false, true, false), Close::Quit);
            assert_eq!(close_plan(kind, 0, false, false, true), Close::Quit);
            assert_eq!(close_plan(kind, 0, false, false, false), Close::Quit);
        }
    }

    #[test]
    fn the_last_app_window_closes_alone_in_background_mode() {
        assert_eq!(close_plan(WindowKind::Main, 0, true, true, true), Close::Hide);
        assert_eq!(close_plan(WindowKind::Project, 0, true, true, true), Close::Allow);
    }

    #[test]
    fn a_window_that_closes_alone_asks_nothing_and_stops_no_turn() {
        for kind in [WindowKind::Main, WindowKind::Project] {
            for (others, background) in [(1, false), (2, false), (0, true), (1, true)] {
                for running in [false, true] {
                    for confirm in [false, true] {
                        let close = close_plan(kind, others, background, running, confirm);
                        assert!(matches!(close, Close::Hide | Close::Allow), "{kind:?} {others} {background} {running} {confirm}: {close:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_window_that_is_not_an_app_window_closes_in_every_case() {
        for others in [0, 1] {
            for background in [false, true] {
                for running in [false, true] {
                    for confirm in [false, true] {
                        assert_eq!(close_plan(WindowKind::Other, others, background, running, confirm), Close::Allow);
                    }
                }
            }
        }
    }

    #[test]
    fn the_window_kind_follows_the_label() {
        assert_eq!(window_kind("main"), WindowKind::Main);
        assert_eq!(window_kind("project-p-1"), WindowKind::Project);
        assert_eq!(window_kind("settings"), WindowKind::Other);
        assert_eq!(window_kind("project-"), WindowKind::Other);
        assert_eq!(window_kind(""), WindowKind::Other);
    }

    #[test]
    fn the_windows_that_count_for_a_close_are_the_visible_app_windows_besides_the_closing_one() {
        let labels = |items: &[&str]| items.iter().map(|label| (*label).to_owned()).collect::<Vec<_>>();
        assert_eq!(other_app_windows(&labels(&["main", "project-a"]), "main"), 1);
        assert_eq!(other_app_windows(&labels(&["main"]), "main"), 0);
        assert_eq!(other_app_windows(&labels(&["project-a"]), "main"), 1, "a hidden main window is not in the list, and a project window counts");
        assert_eq!(other_app_windows(&labels(&["main", "settings", "other"]), "main"), 0, "Settings is not an app window");
        assert_eq!(other_app_windows(&labels(&["main", "project-a", "project-b"]), "project-a"), 2);
        assert_eq!(other_app_windows(&[], "project-a"), 0);
    }

    fn labels(items: &[&str]) -> Vec<String> { items.iter().map(|label| (*label).to_owned()).collect() }
    fn project(label: &str) -> crate::desktop::view::ViewId { crate::desktop::view::ViewId::from_label(label).unwrap() }
    fn main_view() -> crate::desktop::view::ViewId { crate::desktop::view::ViewId::main() }

    #[test]
    fn the_windows_that_stay_are_listed_in_order() {
        assert_eq!(other_app_labels(&labels(&["main", "project-b", "settings", "project-a"]), "project-a"), labels(&["main", "project-b"]));
        assert!(other_app_labels(&labels(&["project-a"]), "project-a").is_empty());
    }

    #[test]
    fn a_project_window_that_closes_with_the_main_window_hidden_shows_the_main_window() {
        // A project window removes its own project while the main window is hidden: the main window is not visible.
        let stay = other_app_labels(&labels(&["project-a"]), "project-a");
        assert_eq!(refocus(&stay, &project("project-a")), Some(Refocus::ShowMain));
        // The same close with another window visible shows nothing.
        let stay = other_app_labels(&labels(&["project-a", "project-b"]), "project-a");
        assert_eq!(refocus(&stay, &project("project-b")), None);
    }

    #[test]
    fn the_focus_moves_to_a_visible_window_when_the_focused_window_is_gone_or_hidden() {
        // The main window hides and a project window stays: the focus moves there.
        assert_eq!(refocus(&labels(&["project-a"]), &main_view()), Some(Refocus::Focus(project("project-a"))));
        // A project window closes and the main window stays.
        assert_eq!(refocus(&labels(&["main", "project-b"]), &project("project-a")), Some(Refocus::Focus(main_view())));
        // The focused window is visible: nothing changes.
        assert_eq!(refocus(&labels(&["main", "project-a"]), &project("project-a")), None);
        assert_eq!(refocus(&labels(&["main"]), &main_view()), None);
    }

    #[test]
    fn a_close_shows_the_main_window_only_when_the_app_does_not_run_in_the_background() {
        // No app window stays. Without background mode the main window shows again (decision 7).
        assert_eq!(refocus_after_close(&[], &project("project-a"), false, false), Some(Refocus::ShowMain));
        // In background mode the app stays with no window (spec T1).
        assert_eq!(refocus_after_close(&[], &project("project-a"), true, false), None);
        assert_eq!(refocus_after_close(&labels(&["settings"]), &main_view(), true, false), None, "Settings is not an app window");
        assert_eq!(refocus_after_close(&labels(&["settings"]), &main_view(), false, false), Some(Refocus::ShowMain));
        // A window stays: the focus rule is the same in both modes.
        for background in [false, true] {
            assert_eq!(refocus_after_close(&labels(&["main", "project-b"]), &project("project-a"), background, false), Some(Refocus::Focus(main_view())));
            assert_eq!(refocus_after_close(&labels(&["main"]), &main_view(), background, false), None);
        }
    }

    #[test]
    fn nothing_shows_while_the_app_exits() {
        for background in [false, true] {
            assert_eq!(refocus_after_close(&[], &project("project-a"), background, true), None);
            assert_eq!(refocus_after_close(&labels(&["project-b"]), &main_view(), background, true), None);
        }
    }

    #[test]
    fn a_question_with_no_visible_app_window_shows_the_main_window() {
        assert_eq!(refocus(&[], &main_view()), Some(Refocus::ShowMain));
        assert_eq!(refocus(&labels(&["settings"]), &main_view()), Some(Refocus::ShowMain), "Settings is not an app window");
    }

    #[test]
    fn the_main_window_action_needs_no_confirm() {
        for running in [false, true] {
            for confirm in [false, true] {
                assert_eq!(plan(Action::ShowMainWindow, running, confirm, false), Plan::ShowMainWindow);
            }
        }
    }

    #[test]
    fn the_close_window_action_closes_the_window_and_the_close_request_decides() {
        // The action asks no quit confirmation: a window that closes alone must not ask it.
        for running in [false, true] {
            for confirm in [false, true] {
                assert_eq!(plan(Action::CloseWindow, running, confirm, false), Plan::CloseWindow);
            }
        }
    }

    #[test]
    fn the_exit_request_follows_the_quit_rule_for_the_last_window() {
        // The same inputs as the exit request: no other app window and no background mode.
        assert_eq!(close_plan(WindowKind::Main, 0, false, true, true), Close::Ask(Confirm::Quit));
        assert_eq!(close_plan(WindowKind::Main, 0, false, true, false), Close::Quit);
        assert_eq!(close_plan(WindowKind::Main, 0, false, false, true), Close::Quit);
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
