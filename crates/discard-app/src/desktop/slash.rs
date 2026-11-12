//! What each built-in slash command does. Pure; the Tauri command executes the step.
use super::core::CommandChannel;
use discard_core::{action::Action, slash::{Builtin, Entry, help_text}};

/// What a built-in command does. Pure; the Tauri command executes it.
#[derive(Debug, PartialEq)]
pub enum Step { Output(String), Dispatch(Action), Error(String) }

pub fn builtin_step(builtin: Builtin, channel: &CommandChannel, entries: &[Entry]) -> Step {
    let name = builtin.name();
    if builtin.planned() { return Step::Error(format!("/{name} is not available yet.")); }
    match builtin {
        Builtin::Help => Step::Output(help_text(entries)),
        Builtin::Stop if !channel.turn_running => Step::Error("No turn is running.".into()),
        Builtin::Archive if channel.is_thread => Step::Error("/archive does not apply to a thread.".into()),
        Builtin::Archive if channel.turn_running => Step::Error("Stop the turn before archiving the channel".into()),
        other => match other.action() {
            Some(action) => Step::Dispatch(action),
            None => Step::Error(format!("/{name} is not available yet.")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(turn_running: bool, is_thread: bool) -> CommandChannel { CommandChannel { turn_running, archived: false, is_thread } }
    fn entries() -> Vec<Entry> { discard_core::slash::registry(&[]).0 }

    #[test] fn planned_commands_return_the_not_available_error() {
        assert_eq!(builtin_step(Builtin::Model, &ch(false, false), &entries()), Step::Error("/model is not available yet.".into()));
        assert_eq!(builtin_step(Builtin::Clear, &ch(true, true), &entries()), Step::Error("/clear is not available yet.".into()));
    }
    #[test] fn stop_needs_a_running_turn() {
        assert_eq!(builtin_step(Builtin::Stop, &ch(false, false), &entries()), Step::Error("No turn is running.".into()));
        assert_eq!(builtin_step(Builtin::Stop, &ch(true, true), &entries()), Step::Dispatch(Action::CancelTurn));
    }
    #[test] fn archive_is_refused_in_a_thread_and_during_a_turn() {
        assert_eq!(builtin_step(Builtin::Archive, &ch(false, true), &entries()), Step::Error("/archive does not apply to a thread.".into()));
        assert_eq!(builtin_step(Builtin::Archive, &ch(true, false), &entries()), Step::Error("Stop the turn before archiving the channel".into()));
        assert_eq!(builtin_step(Builtin::Archive, &ch(false, false), &entries()), Step::Dispatch(Action::ArchiveChannel));
    }
    #[test] fn help_outputs_the_registry_and_the_rest_dispatch() {
        assert!(matches!(builtin_step(Builtin::Help, &ch(false, false), &entries()), Step::Output(text) if text.contains("/stop")));
        assert_eq!(builtin_step(Builtin::Settings, &ch(true, false), &entries()), Step::Dispatch(Action::OpenSettings));
        assert_eq!(builtin_step(Builtin::Members, &ch(false, true), &entries()), Step::Dispatch(Action::ToggleMemberList));
        assert_eq!(builtin_step(Builtin::Plugins, &ch(false, false), &entries()), Step::Dispatch(Action::ReviewPlugins));
    }
}
