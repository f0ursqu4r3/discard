//! Slash commands: the name rule, the parser, the built-in table, and the merged registry.
//! Pure logic. The desktop layer runs the commands.

use crate::action::Action;

/// Longest output of a plugin command, in bytes.
pub const OUTPUT_LIMIT: usize = 16 * 1024;
/// Longest plugin command description, in characters.
pub const DESCRIPTION_LIMIT: usize = 120;
/// Longest plugin command argument hint, in characters.
pub const ARGS_LIMIT: usize = 40;

const NAME_LIMIT: usize = 32;

/// A command name is `[a-z][a-z0-9_-]{0,31}`. Keep aligned with `parseSlash` in web/src/components/slash.ts.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= NAME_LIMIT
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Split `/name args` into the name and the trimmed args. The text must start with the slash.
/// Keep aligned with `parseSlash` in web/src/components/slash.ts.
pub fn parse(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, args) = rest.split_at(end);
    valid_name(name).then(|| (name, args.trim()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin { Help, Stop, Settings, Members, Plugins, Archive, Model, Clear }

impl Builtin {
    pub const ALL: [Builtin; 8] = [Self::Help, Self::Stop, Self::Settings, Self::Members, Self::Plugins, Self::Archive, Self::Model, Self::Clear];

    pub fn name(self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Stop => "stop",
            Self::Settings => "settings",
            Self::Members => "members",
            Self::Plugins => "plugins",
            Self::Archive => "archive",
            // TODO: Change the model of the channel's agent.
            Self::Model => "model",
            // TODO: Start a new agent session in the channel.
            Self::Clear => "clear",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Help => "Show the available commands",
            Self::Stop => "Stop the running turn",
            Self::Settings => "Open Settings",
            Self::Members => "Show or hide the member list",
            Self::Plugins => "Review plugins",
            Self::Archive => "Archive this channel",
            Self::Model => "Change the model of the agent",
            Self::Clear => "Start a new agent session",
        }
    }

    /// The menu action the command runs. `Help` shows the registry. The planned commands have none.
    pub fn action(self) -> Option<Action> {
        match self {
            Self::Stop => Some(Action::CancelTurn),
            Self::Settings => Some(Action::OpenSettings),
            Self::Members => Some(Action::ToggleMemberList),
            Self::Plugins => Some(Action::ReviewPlugins),
            Self::Archive => Some(Action::ArchiveChannel),
            Self::Help | Self::Model | Self::Clear => None,
        }
    }

    /// A planned command shows greyed, tagged "Soon", and does nothing.
    pub fn planned(self) -> bool {
        matches!(self, Self::Model | Self::Clear)
    }

    pub fn from_name(name: &str) -> Option<Builtin> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand { pub plugin: String, pub name: String, pub description: Option<String>, pub args: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry { pub name: String, pub description: String, pub args: Option<String>, pub source: String, pub planned: bool }

/// A plugin command that lost its name. `by` is "builtin" or the plugin that won.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed { pub plugin: String, pub name: String, pub by: String }

/// Built-ins first in `Builtin::ALL` order, then plugin commands in alphabetical order.
/// A built-in beats a plugin command. Among plugins, the first in load order wins.
pub fn registry(plugins: &[PluginCommand]) -> (Vec<Entry>, Vec<Shadowed>) {
    let mut entries: Vec<Entry> = Builtin::ALL
        .iter()
        .map(|b| Entry { name: b.name().into(), description: b.description().into(), args: None, source: "builtin".into(), planned: b.planned() })
        .collect();
    let mut shadowed = Vec::new();
    let mut commands: Vec<Entry> = Vec::new();
    for command in plugins {
        let winner = if Builtin::from_name(&command.name).is_some() { Some("builtin") } else { commands.iter().find(|e| e.name == command.name).map(|e| e.source.as_str()) };
        if let Some(by) = winner {
            shadowed.push(Shadowed { plugin: command.plugin.clone(), name: command.name.clone(), by: by.into() });
        } else {
            commands.push(Entry {
                name: command.name.clone(),
                description: command.description.clone().unwrap_or_else(|| "Plugin command".into()),
                args: command.args.clone(),
                source: command.plugin.clone(),
                planned: false,
            });
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    entries.extend(commands);
    (entries, shadowed)
}

/// One line per entry: `/name[ args] — description[ (Soon)][ (plugin)]`.
pub fn help_text(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(|e| {
            let mut line = format!("/{}", e.name);
            if let Some(args) = &e.args {
                line.push(' ');
                line.push_str(args);
            }
            line.push_str(" — ");
            line.push_str(&e.description);
            if e.planned {
                line.push_str(" (Soon)");
            }
            if e.source != "builtin" {
                line.push_str(&format!(" ({})", e.source));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cut the text to `OUTPUT_LIMIT` bytes at a character boundary and mark the cut.
pub fn truncate_output(text: String) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text;
    }
    let mut end = OUTPUT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… (truncated)", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;

    #[test] fn parses_the_command_form() {
        assert_eq!(parse("/help"), Some(("help", "")));
        assert_eq!(parse("/echo  a b \n c "), Some(("echo", "a b \n c")));
        assert_eq!(parse("/stop\nnow"), Some(("stop", "now")));
    }
    #[test] fn rejects_text_that_is_not_a_command() {
        for text in ["", "/", "//help", " /help", "/Help", "/usr/bin/env", "/1x", "hello /help", &format!("/{}", "a".repeat(33))] {
            assert_eq!(parse(text), None, "{text}");
        }
        assert!(parse(&format!("/{}", "a".repeat(32))).is_some());
    }
    #[test] fn valid_name_matches_the_plugin_rule() {
        assert!(valid_name("a") && valid_name("git-log_2"));
        for name in ["", "A", "1a", "a b", "a/b", &"a".repeat(33)] { assert!(!valid_name(name), "{name}"); }
    }
    #[test] fn builtins_are_in_table_order_and_planned_ones_have_no_action() {
        let names: Vec<_> = Builtin::ALL.iter().map(|b| b.name()).collect();
        assert_eq!(names, ["help", "stop", "settings", "members", "plugins", "archive", "model", "clear"]);
        for b in Builtin::ALL { if b.planned() { assert_eq!(b.action(), None); } }
        assert_eq!(Builtin::Stop.action(), Some(Action::CancelTurn));
        assert_eq!(Builtin::from_name("archive"), Some(Builtin::Archive));
        assert_eq!(Builtin::from_name("nope"), None);
    }
    #[test] fn registry_merges_with_builtin_and_first_plugin_precedence() {
        let p = |plugin: &str, name: &str| PluginCommand { plugin: plugin.into(), name: name.into(), description: None, args: None };
        let (entries, shadowed) = registry(&[p("b", "zed"), p("a", "help"), p("a", "zed"), p("a", "apple")]);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["help", "stop", "settings", "members", "plugins", "archive", "model", "clear", "apple", "zed"]);
        assert_eq!(entries.last().unwrap().source, "b");
        assert_eq!(entries.last().unwrap().description, "Plugin command");
        assert_eq!(entries[0].source, "builtin");
        assert!(entries[6].planned && !entries[0].planned);
        assert_eq!(shadowed, [
            Shadowed { plugin: "a".into(), name: "help".into(), by: "builtin".into() },
            Shadowed { plugin: "a".into(), name: "zed".into(), by: "b".into() },
        ]);
    }
    #[test] fn help_lists_every_entry() {
        let (entries, _) = registry(&[PluginCommand { plugin: "p".into(), name: "echo".into(), description: Some("Repeat".into()), args: Some("<text>".into()) }]);
        let text = help_text(&entries);
        assert!(text.contains("/help — Show the available commands"));
        assert!(text.contains("/model — Change the model of the agent (Soon)"));
        assert!(text.contains("/echo <text> — Repeat (p)"));
        assert_eq!(text.lines().count(), 9);
    }
    #[test] fn truncates_output_on_a_character_boundary() {
        assert_eq!(truncate_output("short".into()), "short");
        let long = "é".repeat(OUTPUT_LIMIT); // 2 bytes each
        let out = truncate_output(format!("x{long}")); // the cut lands inside an é
        assert!(out.ends_with("… (truncated)"));
        assert!(out.len() <= OUTPUT_LIMIT + "… (truncated)".len());
        assert_eq!(truncate_output("a".repeat(OUTPUT_LIMIT)).len(), OUTPUT_LIMIT);
    }
}
