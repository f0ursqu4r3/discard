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
pub enum Close {
    /// Close the window.
    Allow,
    /// Keep the window open and show the confirm notice.
    Ask(Confirm),
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

/// Decide whether a window may close. Closing the main window ends the app, so it follows the Quit rule.
/// Pure, so the rule is testable without a window.
pub fn close_plan(is_main: bool, turn_running: bool, confirm_while_working: bool) -> Close {
    if is_main && needs_confirm(Action::Quit, turn_running, confirm_while_working) {
        Close::Ask(Confirm::Quit)
    } else {
        Close::Allow
    }
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
    use super::{ArchiveStep, Close, Confirm, Plan, TargetedPlan, archive_step, channel_target_allowed, close_plan, plan, plan_targeted, prepare_exit};
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
