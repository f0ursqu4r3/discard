//! Read-only structured log viewing and tailing.
use crate::log_file::{ACTIVE_NAME, log_files};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "TRACE" => Some(Self::Trace),
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARN" => Some(Self::Warn),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Provider,
    Plugin,
    Approval,
    Storage,
    Mcp,
    Workspace,
    Other,
}
impl Component {
    pub fn label(self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::Plugin => "Plugin",
            Self::Approval => "Approval",
            Self::Storage => "Storage",
            Self::Mcp => "MCP",
            Self::Workspace => "Workspace",
            Self::Other => "Other",
        }
    }
}
pub fn component_of(s: &str) -> Component {
    if s.starts_with("discard_providers") {
        Component::Provider
    } else if s.starts_with("discard_lua") || s.starts_with("discard_app::plugin") {
        Component::Plugin
    } else if s.starts_with("discard_mcp") {
        Component::Mcp
    } else if s.starts_with("discard_core::approval") {
        Component::Approval
    } else if s.starts_with("discard_core::store") {
        Component::Storage
    } else if s.starts_with("discard_core::workspace") || s.starts_with("discard_core::worktree") {
        Component::Workspace
    } else {
        Component::Other
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub component: Component,
    pub message: String,
    pub session: Option<String>,
    pub turn: Option<String>,
    pub plugin: Option<String>,
    pub raw: String,
}
pub fn parse_line(raw: &str) -> LogLine {
    let json = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .filter(|v| v.is_object());
    let field = |name: &str| {
        json.as_ref()
            .and_then(|v| v.get("fields"))
            .and_then(|v| v.get(name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    LogLine {
        time: json
            .as_ref()
            .and_then(|v| v.get("timestamp"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        level: json
            .as_ref()
            .and_then(|v| v.get("level"))
            .and_then(|v| v.as_str())
            .and_then(Level::parse)
            .unwrap_or(Level::Info),
        component: component_of(
            json.as_ref()
                .and_then(|v| v.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        ),
        message: field("message").unwrap_or_else(|| {
            if json.is_none() {
                raw.into()
            } else {
                String::new()
            }
        }),
        session: field("session"),
        turn: field("turn"),
        plugin: field("plugin"),
        raw: raw.into(),
    }
}
pub struct LogFilter {
    pub min_level: Level,
    pub component: Option<Component>,
    pub id: String,
}
impl Default for LogFilter {
    fn default() -> Self {
        Self {
            min_level: Level::Info,
            component: None,
            id: String::new(),
        }
    }
}
impl LogFilter {
    pub fn matches(&self, l: &LogLine) -> bool {
        l.level >= self.min_level
            && self.component.is_none_or(|c| c == l.component)
            && (self.id.is_empty()
                || [&l.session, &l.turn, &l.plugin]
                    .into_iter()
                    .any(|s| s.as_ref().is_some_and(|s| s.contains(&self.id))))
    }
}
pub const MAX_LINES: usize = 2000;
pub const MAX_DISPLAY_CHARS: usize = 400;
pub fn shorten(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(MAX_DISPLAY_CHARS).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
/// How much of the start of the file `LogTail` reads to find the first line.
const HEAD_BYTES: u64 = 64 * 1024;

pub struct LogTail {
    dir: PathBuf,
    offset: u64,
    first: Option<Vec<u8>>,
    initial: bool,
    generation: u64,
}
impl LogTail {
    pub fn open(dir: &Path) -> Self {
        Self {
            dir: dir.into(),
            offset: 0,
            first: None,
            initial: true,
            generation: 0,
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn poll(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.initial {
            for path in log_files(&self.dir)
                .into_iter()
                .filter(|p| p.file_name().is_some_and(|n| n != ACTIVE_NAME))
            {
                if let Ok(bytes) = std::fs::read(path) {
                    lines.extend(
                        bytes
                            .split(|b| *b == b'\n')
                            .filter(|s| !s.is_empty())
                            .map(|s| String::from_utf8_lossy(s).into_owned()),
                    );
                }
            }
            self.initial = false;
        }
        let Ok(mut file) = std::fs::File::open(self.dir.join(ACTIVE_NAME)) else {
            self.offset = 0;
            self.first = None;
            if lines.len() > MAX_LINES {
                lines.drain(..lines.len() - MAX_LINES);
            }
            return lines;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return lines;
        };
        // The first line tells a rotated file from a grown one. Read only the head of the file.
        let mut head = Vec::new();
        if file.by_ref().take(HEAD_BYTES).read_to_end(&mut head).is_err() {
            return lines;
        }
        let first = head
            .iter()
            .position(|b| *b == b'\n')
            .map(|end| head[..end].to_vec());
        if len < self.offset
            || self
                .first
                .as_ref()
                .zip(first.as_ref())
                .is_some_and(|(a, b)| a != b)
        {
            if len == 0 {
                self.generation += 1;
            }
            self.offset = 0;
        }
        if self.first.is_none() || self.offset == 0 {
            self.first = first;
        }
        // Read only what is new since the last poll.
        let mut rest = Vec::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_end(&mut rest).is_err() {
            return lines;
        }
        if let Some(end) = rest.iter().rposition(|b| *b == b'\n') {
            lines.extend(
                rest[..end]
                    .split(|b| *b == b'\n')
                    .map(|s| String::from_utf8_lossy(s).into_owned()),
            );
            self.offset += (end + 1) as u64;
        }
        if lines.len() > MAX_LINES {
            lines.drain(..lines.len() - MAX_LINES);
        }
        lines
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn parse_line_reads_a_tracing_json_event() {
        let l = parse_line(
            r#"{"timestamp":"now","level":"WARN","target":"discard_providers::x","fields":{"message":"hi","session":"s","turn":"t","plugin":"p"}}"#,
        );
        assert_eq!(
            (l.time.as_str(), l.level, l.component, l.message.as_str()),
            ("now", Level::Warn, Component::Provider, "hi")
        );
        assert_eq!(
            (l.session.as_deref(), l.turn.as_deref(), l.plugin.as_deref()),
            (Some("s"), Some("t"), Some("p"))
        );
    }
    #[test]
    fn parse_line_keeps_a_line_that_is_not_json_as_raw_text() {
        let l = parse_line("bad");
        assert_eq!(
            (l.message.as_str(), l.level, l.component),
            ("bad", Level::Info, Component::Other)
        );
    }
    #[test]
    fn parse_line_survives_a_missing_fields_object() {
        assert_eq!(parse_line("{}").level, Level::Info);
    }
    #[test]
    fn component_of_maps_each_target_prefix() {
        for (s, c) in [
            ("discard_providers::a", Component::Provider),
            ("discard_lua", Component::Plugin),
            ("discard_app::plugin", Component::Plugin),
            ("discard_mcp", Component::Mcp),
            ("discard_core::approval", Component::Approval),
            ("discard_core::store", Component::Storage),
            ("discard_core::workspace", Component::Workspace),
            ("discard_core::worktree", Component::Workspace),
            ("other", Component::Other),
        ] {
            assert_eq!(component_of(s), c);
        }
    }
    #[test]
    fn filter_applies_level_component_and_id_together() {
        let l = parse_line(r#"{"level":"WARN","target":"discard_mcp","fields":{"session":"abc"}}"#);
        let mut f = LogFilter {
            min_level: Level::Warn,
            component: Some(Component::Mcp),
            id: "bc".into(),
        };
        assert!(f.matches(&l));
        f.min_level = Level::Error;
        assert!(!f.matches(&l));
    }
    #[test]
    fn the_id_filter_matches_session_turn_or_plugin_as_a_substring_and_is_case_sensitive() {
        let l = parse_line(r#"{"fields":{"session":"Session","turn":"Turn","plugin":"Plugin"}}"#);
        for id in ["ess", "urn", "lugi"] {
            assert!(
                LogFilter {
                    id: id.into(),
                    ..Default::default()
                }
                .matches(&l)
            );
        }
        assert!(
            !LogFilter {
                id: "session".into(),
                ..Default::default()
            }
            .matches(&l)
        );
    }
    #[test]
    fn shorten_cuts_long_text_on_a_character_boundary() {
        let s = "é😀".repeat(300);
        assert_eq!(shorten(&s).chars().count(), MAX_DISPLAY_CHARS + 1);
    }
    #[test]
    fn tail_returns_only_complete_lines_and_the_rest_on_a_later_poll() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("discard.log"), "one\ntw").unwrap();
        let mut t = LogTail::open(d.path());
        assert_eq!(t.poll(), ["one"]);
        std::fs::OpenOptions::new()
            .append(true)
            .open(d.path().join("discard.log"))
            .unwrap()
            .write_all(b"o\n")
            .unwrap();
        assert_eq!(t.poll(), ["two"]);
    }
    #[test]
    fn tail_restarts_after_the_file_shrinks() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("discard.log");
        std::fs::write(&p, "long line\n").unwrap();
        let mut t = LogTail::open(d.path());
        t.poll();
        std::fs::write(&p, "new\n").unwrap();
        assert_eq!(t.poll(), ["new"]);
    }
    #[test]
    fn tail_restarts_after_a_rotation() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("discard.log");
        std::fs::write(&p, "first\n").unwrap();
        let mut t = LogTail::open(d.path());
        t.poll();
        std::fs::rename(&p, d.path().join("discard.log.1")).unwrap();
        std::fs::write(&p, "second line\n").unwrap();
        assert_eq!(t.poll(), ["second line"]);
    }
    #[test]
    fn tail_first_poll_returns_at_most_max_lines_of_the_newest_lines() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("discard.log"), "a\n".repeat(MAX_LINES + 5)).unwrap();
        assert_eq!(LogTail::open(d.path()).poll().len(), MAX_LINES);
    }
    #[test]
    fn tail_reports_a_reset_when_the_active_file_is_cleared() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("discard.log");
        std::fs::write(&p, "old line\n").unwrap();
        let mut t = LogTail::open(d.path());
        t.poll();
        let before = t.generation();
        std::fs::write(&p, "").unwrap();
        t.poll();
        assert!(t.generation() > before);
    }
    #[test]
    fn tail_limits_rotated_lines_when_the_active_file_is_missing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("discard.log.1"), "a\n".repeat(MAX_LINES + 5)).unwrap();
        assert_eq!(LogTail::open(d.path()).poll().len(), MAX_LINES);
    }
    #[test]
    fn tail_with_no_folder_returns_nothing() {
        let d = tempfile::tempdir().unwrap();
        assert!(LogTail::open(&d.path().join("no")).poll().is_empty());
    }
}
