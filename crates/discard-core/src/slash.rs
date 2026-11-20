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
            Self::Model => "model",
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
            Self::Model => "Change the model of an agent",
            Self::Clear => "Start a new agent session with no earlier context",
        }
    }

    /// The argument hint of the command, if it takes arguments.
    pub fn args(self) -> Option<&'static str> {
        match self {
            Self::Model => Some("@agent [model]"),
            _ => None,
        }
    }

    /// The menu action the command runs. `Help`, `Model`, and `Clear` have none: the desktop layer runs them.
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

    /// A planned command shows greyed, tagged "Soon", and does nothing. No command is planned now. Mark a future
    /// one here and add a `TODO` that states its missing behavior (AGENTS.md).
    pub fn planned(self) -> bool {
        match self {
            Self::Help | Self::Stop | Self::Settings | Self::Members | Self::Plugins | Self::Archive | Self::Model | Self::Clear => false,
        }
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
        .map(|b| Entry { name: b.name().into(), description: b.description().into(), args: b.args().map(Into::into), source: "builtin".into(), planned: b.planned() })
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

/// The model ID of `/model @agent [model]`, from the text after the command name. The text holds exactly one
/// mention, in the form of `mentions::names`, in any position.
pub fn parse_model_args(args: &str) -> Result<Option<String>, String> {
    const USAGE: &str = "/model @name <model>";
    match crate::mentions::names(args).len() {
        0 => return Err(format!("Mention the agent: {USAGE}")),
        1 => {}
        _ => return Err(format!("Mention one agent: {USAGE}")),
    }
    let pattern = regex::Regex::new(r#"(?:^|[\s(])(@(?:"[^"]+"|[\p{L}\p{N}_-]+))"#).expect("mention pattern");
    let Some(mention) = pattern.captures(args).and_then(|c| c.get(1)) else { return Err(format!("Mention the agent: {USAGE}")) };
    let rest = format!("{} {}", &args[..mention.start()], &args[mention.end()..]);
    let mut words = rest.split_whitespace();
    let model = words.next().map(String::from);
    if words.next().is_some() { return Err(format!("Give one model ID: {USAGE}")); }
    Ok(model)
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
    #[test] fn builtins_are_in_table_order_and_none_is_planned() {
        let names: Vec<_> = Builtin::ALL.iter().map(|b| b.name()).collect();
        assert_eq!(names, ["help", "stop", "settings", "members", "plugins", "archive", "model", "clear"]);
        assert!(Builtin::ALL.iter().all(|b| !b.planned()), "/model and /clear are implemented");
        assert_eq!((Builtin::Model.action(), Builtin::Clear.action()), (None, None), "they run in the desktop layer, not through the menu");
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
        assert!(entries.iter().all(|e| !e.planned));
        assert_eq!(entries[6].args.as_deref(), Some("@agent [model]"));
        assert_eq!(shadowed, [
            Shadowed { plugin: "a".into(), name: "help".into(), by: "builtin".into() },
            Shadowed { plugin: "a".into(), name: "zed".into(), by: "b".into() },
        ]);
    }
    #[test] fn help_lists_every_entry() {
        let (entries, _) = registry(&[PluginCommand { plugin: "p".into(), name: "echo".into(), description: Some("Repeat".into()), args: Some("<text>".into()) }]);
        let text = help_text(&entries);
        assert!(text.contains("/help — Show the available commands"));
        assert!(text.contains("/model @agent [model] — Change the model of an agent"));
        assert!(text.contains("/clear — Start a new agent session with no earlier context"));
        assert!(!text.contains("Soon"));
        assert!(text.contains("/echo <text> — Repeat (p)"));
        assert_eq!(text.lines().count(), 9);
    }
    #[test] fn model_args_need_one_mention_and_at_most_one_model() {
        let usage = "Mention the agent: /model @name <model>";
        for (args, model) in [("@Alice gpt-5", Some("gpt-5")), ("gpt-5 @Alice", Some("gpt-5")), ("@Alice", None), ("  @Alice\tm-1:x  ", Some("m-1:x")), ("@\"Ann Lee\" m", Some("m")), ("@\"Ann Lee\"", None)] {
            assert_eq!(parse_model_args(args), Ok(model.map(String::from)), "{args:?}");
        }
        assert_eq!(parse_model_args(""), Err(usage.into()));
        assert_eq!(parse_model_args("gpt-5"), Err(usage.into()));
        assert_eq!(parse_model_args("mail@Alice.com m"), Err(usage.into()));
        assert_eq!(parse_model_args("@A @B m"), Err("Mention one agent: /model @name <model>".into()));
        assert_eq!(parse_model_args("@Alice a b"), Err("Give one model ID: /model @name <model>".into()));
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
