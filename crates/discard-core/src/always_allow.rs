//! Durable, project-scoped approval rules. Unsupported shell syntax fails closed.
use crate::model::{AgentId, ProjectId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleTarget {
    Command { words: Vec<String> },
    Host { host: String },
}

pub enum RuleRequest<'a> {
    Command(&'a [String]),
    Host(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlwaysAllowRule {
    pub id: String,
    pub agent: AgentId,
    pub project: ProjectId,
    pub target: RuleTarget,
    pub created_at: i64,
}

pub fn command_words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' || c == '\0' {
            return None;
        }
        if quote == Some('\'') {
            if c == '\'' {
                quote = None;
            } else {
                word.push(c);
            }
        } else if c == '\\' {
            let next = chars.next()?;
            if matches!(next, '\n' | '\r' | '\0') {
                return None;
            }
            // In double quotes the shell preserves a backslash before ordinary letters.
            if quote == Some('"') && !matches!(next, '$' | '`' | '"' | '\\') {
                word.push('\\');
            }
            word.push(next);
            started = true;
        } else if matches!(c, '$' | '`') {
            return None;
        } else if quote == Some('"') {
            if c == '"' {
                quote = None;
            } else {
                word.push(c);
            }
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
            started = true;
        } else if matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')') {
            return None;
        } else if c == ' ' || c == '\t' {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        words.push(word);
    }
    let program = words.first()?;
    if program.is_empty()
        || program.contains('=')
        || matches!(
            program.rsplit('/').next()?,
            "sudo" | "doas" | "env" | "exec" | "eval" | "command" | "builtin" | "nohup" | "xargs"
        )
    {
        return None;
    }
    Some(words)
}

pub fn default_cut(words: &[String]) -> usize {
    words
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, word)| !word.starts_with('-'))
        .map_or(1, |(index, _)| index + 1)
}

pub fn normalize_host(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    if host.is_empty()
        || host.starts_with('.')
        || host.ends_with('.')
        || !host
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
    {
        return None;
    }
    Some(host)
}

pub fn matches(target: &RuleTarget, request: &RuleRequest<'_>) -> bool {
    match (target, request) {
        (RuleTarget::Command { words }, RuleRequest::Command(request)) => {
            !words.is_empty() && request.starts_with(words)
        }
        (RuleTarget::Host { host }, RuleRequest::Host(request)) => {
            normalize_host(host).is_some_and(|host| Some(host) == normalize_host(request))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_commands_parse_into_words() {
        assert_eq!(
            command_words("cargo test -p discard-core workspace").unwrap(),
            ["cargo", "test", "-p", "discard-core", "workspace"]
        );
        assert_eq!(
            command_words("git log --format='%h %s'").unwrap(),
            ["git", "log", "--format=%h %s"]
        );
        assert_eq!(
            command_words(r#"echo "a b" c\ d"#).unwrap(),
            ["echo", "a b", "c d"]
        );
    }

    #[test]
    fn shell_syntax_never_parses() {
        for command in [
            "cargo test; rm -rf ~",
            "cargo test && curl x",
            "cargo test || true",
            "cargo test | sh",
            "cargo test > ~/.zshrc",
            "cargo test < input",
            "cargo test $(curl x)",
            "cargo test `id`",
            "cargo test ${HOME}",
            "cargo test $HOME",
            "FOO=1 cargo test",
            "sudo cargo test",
            "cargo test\nrm x",
            "cargo test &",
            "cargo 'test",
            "(cargo test)",
            "cargo test 2>&1",
            "",
        ] {
            assert_eq!(command_words(command), None, "{command:?}");
        }
    }

    #[test]
    fn default_cut_is_program_and_first_non_flag_word() {
        let words = |s: &str| command_words(s).unwrap();
        assert_eq!(
            default_cut(&words("cargo test -p discard-core workspace")),
            2
        );
        assert_eq!(default_cut(&words("ls -la")), 1);
        assert_eq!(default_cut(&words("npm run build")), 2);
        assert_eq!(default_cut(&words("pnpm --dir web test")), 3);
    }

    #[test]
    fn command_rules_match_prefixes_only() {
        let rule = RuleTarget::Command {
            words: vec!["cargo".into(), "test".into()],
        };
        let ok = command_words("cargo test -p x").unwrap();
        let other = command_words("cargo testx").unwrap();
        let short = command_words("cargo").unwrap();
        assert!(matches(&rule, &RuleRequest::Command(&ok)));
        assert!(!matches(&rule, &RuleRequest::Command(&other)));
        assert!(!matches(&rule, &RuleRequest::Command(&short)));
    }

    #[test]
    fn host_rules_match_exact_hosts_only() {
        assert_eq!(
            normalize_host("Registry.NPMjs.org"),
            Some("registry.npmjs.org".into())
        );
        for bad in [
            "*.npmjs.org",
            "",
            "registry.npmjs.org:443",
            "registry.npmjs.org/x",
            ".npmjs.org",
        ] {
            assert_eq!(normalize_host(bad), None, "{bad}");
        }
        let rule = RuleTarget::Host {
            host: "registry.npmjs.org".into(),
        };
        assert!(matches(&rule, &RuleRequest::Host("REGISTRY.npmjs.org")));
        assert!(!matches(
            &rule,
            &RuleRequest::Host("evil.registry.npmjs.org")
        ));
    }
}
