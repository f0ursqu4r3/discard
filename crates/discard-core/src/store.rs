//! SQLite store. Discard's own history is the source of truth for the UI.
//! The provider's native session ID is kept only as a resume handle.

use crate::agent::AgentConfig;
use crate::always_allow::{AlwaysAllowRule, RuleTarget, RuleRequest};
use crate::approval::{Approval, ApprovalState, AuditRecord, Decider, Decision};
use crate::event::{ApprovalId, Event, EventId, EventKind, SessionId, TurnId};
use crate::model::{
    AgentId, Block, Channel, ChannelId, ChannelTopic, ChannelTopicRevision, Message, MessageAuthor,
    MessageId, PostedSource, Project, ProjectChannelKind, ProjectChannelMetadata, ProjectId, Role,
    ThreadInfo, ThreadKind, ThreadState, TopicAuthorKind, WorkspaceMode,
};
use crate::turn::{Turn, TurnState};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

mod agent_sessions;
mod rail;
mod review;
pub use agent_sessions::AgentSession;
pub use rail::{RailGroup, RailItem, RailLayout, RailTarget};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("toml: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("invalid turn transition {from:?} -> {to:?}")]
    InvalidTransition { from: TurnState, to: TurnState },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("corrupt row: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub turns_failed: Vec<TurnId>,
    pub approvals_staled: Vec<ApprovalId>,
}

const SCHEMA_V1: &str = "
CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL);
CREATE TABLE agents (id TEXT PRIMARY KEY, config_toml TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE channels (
    ord INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT REFERENCES projects(id),
    name TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    workspace_mode TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK(is_default=0 OR project_id IS NULL)
);
CREATE UNIQUE INDEX one_default_conversation ON channels(agent_id) WHERE is_default = 1;
CREATE TRIGGER fixed_conversation_context BEFORE UPDATE OF project_id ON channels
WHEN NEW.project_id IS NOT OLD.project_id BEGIN SELECT RAISE(ABORT, 'Conversation context is fixed.'); END;
CREATE TABLE sessions (
    ord INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    channel_id TEXT NOT NULL REFERENCES channels(id),
    resume_handle TEXT
);
CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    state TEXT NOT NULL
);
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL,
    kind_json TEXT NOT NULL,
    raw_json TEXT,
    UNIQUE (session_id, seq)
);
CREATE TABLE approvals (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id),
    tool TEXT NOT NULL,
    input_json TEXT NOT NULL,
    state TEXT NOT NULL
);
CREATE TABLE approval_audit (
    approval_id TEXT NOT NULL REFERENCES approvals(id),
    n INTEGER NOT NULL,
    decision TEXT NOT NULL,
    decider_json TEXT NOT NULL,
    policy TEXT NOT NULL,
    PRIMARY KEY (approval_id, n)
);
CREATE TABLE conversation_context (channel_id TEXT PRIMARY KEY REFERENCES channels(id), state_json TEXT NOT NULL);
CREATE TABLE response_memory (channel_id TEXT NOT NULL REFERENCES channels(id), response_id TEXT NOT NULL REFERENCES messages(id), change_key TEXT NOT NULL, change_json TEXT NOT NULL, PRIMARY KEY(channel_id, change_key));
CREATE TABLE messages (
    ord INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    channel_id TEXT NOT NULL REFERENCES channels(id),
    role TEXT NOT NULL,
    blocks_json TEXT NOT NULL
);
";

const SCHEMA_V2: &str = "
CREATE TABLE worktrees (
    channel_id TEXT PRIMARY KEY REFERENCES channels(id),
    path TEXT NOT NULL,
    branch TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    repo TEXT NOT NULL
);
ALTER TABLE channels ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
";

const SCHEMA_V4: &str = "
CREATE TABLE project_channels (
    channel_id TEXT PRIMARY KEY REFERENCES channels(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    kind TEXT NOT NULL CHECK(kind IN ('general','branch','archived_legacy')),
    branch_name TEXT,
    archived_into TEXT REFERENCES channels(id)
);
CREATE UNIQUE INDEX one_project_general_channel ON project_channels(project_id) WHERE kind='general';
CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id, branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;
CREATE TABLE project_main_agents (project_id TEXT PRIMARY KEY REFERENCES projects(id), agent_id TEXT NOT NULL REFERENCES agents(id));
CREATE TABLE project_agents (project_id TEXT NOT NULL REFERENCES projects(id), agent_id TEXT NOT NULL REFERENCES agents(id), PRIMARY KEY(project_id,agent_id));
CREATE TABLE channel_agents (channel_id TEXT NOT NULL REFERENCES channels(id), agent_id TEXT NOT NULL REFERENCES agents(id), PRIMARY KEY(channel_id,agent_id));
CREATE TABLE project_last_channels (project_id TEXT PRIMARY KEY REFERENCES projects(id), channel_id TEXT NOT NULL REFERENCES channels(id));
CREATE TABLE project_channel_views (channel_id TEXT PRIMARY KEY REFERENCES channels(id), viewed_order INTEGER NOT NULL);
CREATE TABLE channel_topic_revisions (
    channel_id TEXT NOT NULL REFERENCES channels(id), version INTEGER NOT NULL CHECK(version > 0),
    text TEXT NOT NULL, author_kind TEXT NOT NULL CHECK(author_kind IN ('user','agent')), author_id TEXT REFERENCES agents(id),
    PRIMARY KEY(channel_id,version), CHECK((author_kind='user' AND author_id IS NULL) OR (author_kind='agent' AND author_id IS NOT NULL))
);
";

/// A channel's git worktree, as recorded when it was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRecord {
    pub channel_id: ChannelId,
    pub path: String,
    pub branch: String,
    pub base_commit: String,
    pub repo: String,
    pub owned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteAgentOptions {
    pub delete_conversations: bool,
    pub delete_private_memory: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateMemoryScope {
    Agent(AgentId),
    AgentProject { agent: AgentId, project: ProjectId },
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeleteAgentResult {
    pub private_scopes: Vec<PrivateMemoryScope>,
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMatch {
    pub channel_id: String,
    pub channel_name: String,
    pub message_id: String,
    pub role: String,
    pub preview: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySearch {
    pub results: Vec<HistoryMatch>,
    pub has_more: bool,
}

/// What a response replays: messages oldest first, with the side maps replay needs.
#[derive(Debug, Default)]
pub struct ChannelHistory {
    pub messages: Vec<Message>,
    pub authors: std::collections::HashMap<MessageId, MessageAuthor>,
    /// Posted reference rows among `messages`, by reference ID.
    pub posted: std::collections::HashMap<MessageId, PostedSource>,
}

/// The messages after the `/clear` marker. Without a marker, or when the marker is not among them, all of them.
fn after_reset(mut messages: Vec<Message>, reset: Option<&MessageId>) -> Vec<Message> {
    if let Some(index) = reset.and_then(|id| messages.iter().position(|m| &m.id == id)) { messages.drain(..=index); }
    messages
}

/// Characters kept from the first thread message when it names the thread.
pub const THREAD_NAME_CHARS: usize = 40;

/// Name a thread from its first message: whitespace collapsed, cut at `THREAD_NAME_CHARS` characters
/// on a character boundary. Text with no visible characters gives `Thread`.
pub fn thread_name(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(THREAD_NAME_CHARS) {
        Some((end, _)) => flat[..end].trim_end().to_owned(),
        None if flat.is_empty() => "Thread".into(),
        None => flat,
    }
}

/// Longest thread name a user can set, in characters.
pub const THREAD_NAME_MAX: usize = 80;

/// Check a name that the user gave a thread: line breaks become spaces, the ends are trimmed, and the length is 1 to
/// `THREAD_NAME_MAX` characters. Duplicate names are allowed.
pub fn normalize_thread_name(input: &str) -> std::result::Result<String, String> {
    let flat: String = input.replace("\r\n", "\n").chars().map(|c| if matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}') { ' ' } else { c }).collect();
    let name = flat.trim();
    if name.is_empty() || name.chars().count() > THREAD_NAME_MAX {
        return Err(format!("Enter a thread name of 1 to {THREAD_NAME_MAX} characters."));
    }
    Ok(name.to_owned())
}

/// Normalize the name of a text channel: lowercase ASCII words joined by single hyphens.
pub fn normalize_text_channel_name(input: &str) -> std::result::Result<String, String> {
    let lowered = input.trim().to_lowercase();
    let mut name = String::new();
    for word in lowered.split_whitespace() {
        if !name.is_empty() { name.push('-'); }
        name.push_str(word);
    }
    let mut collapsed = String::new();
    for c in name.chars() {
        if c == '-' && collapsed.ends_with('-') { continue; }
        collapsed.push(c);
    }
    let name = collapsed.trim_matches('-').to_owned();
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("Use letters, numbers, hyphens, and underscores.".into());
    }
    if name.is_empty() || name.len() > 50 {
        return Err("Enter a channel name of 1 to 50 characters.".into());
    }
    Ok(name)
}

/// Name a sub-agent thread from the task title, or from the start of the brief when the title is empty.
pub fn sub_agent_thread_name(title: &str, brief: &str) -> String {
    let named = thread_name(title);
    if title.trim().is_empty() { thread_name(brief) } else { named }
}

/// What `Store::delete_thread` changed outside the thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadDeletion {
    /// The channel the thread hung under.
    pub parent: ChannelId,
    /// The reference rows of the parent that became normal messages, in channel order.
    pub posted: Vec<MessageId>,
}

/// Rows that hang off one channel, in an order that leaves no dangling reference. Posted references in other
/// channels go with their source message, so a caller that keeps them must turn them into messages first.
const CHANNEL_PURGE_SQL: [&str; 21] = [
    "DELETE FROM project_last_channels WHERE channel_id=?1",
    "DELETE FROM project_channel_views WHERE channel_id=?1",
    "DELETE FROM project_channels WHERE channel_id=?1",
    "DELETE FROM channel_topic_revisions WHERE channel_id=?1",
    "DELETE FROM channel_agents WHERE channel_id=?1",
    "DELETE FROM agent_commits WHERE channel_id=?1",
    "DELETE FROM autorun_responses WHERE channel_id=?1",
    "DELETE FROM approval_audit WHERE approval_id IN (SELECT a.id FROM approvals a JOIN turns t ON a.turn_id=t.id JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
    "DELETE FROM approvals WHERE turn_id IN (SELECT t.id FROM turns t JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
    "DELETE FROM turns WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
    "DELETE FROM events WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
    "DELETE FROM conversation_context WHERE channel_id=?1",
    "DELETE FROM response_memory WHERE channel_id=?1",
    "DELETE FROM agent_sessions WHERE channel_id=?1",
    "DELETE FROM sessions WHERE channel_id=?1",
    "DELETE FROM messages WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?1)",
    "DELETE FROM messages WHERE channel_id=?1",
    "DELETE FROM worktrees WHERE channel_id=?1",
    "DELETE FROM channel_branches WHERE channel_id=?1",
    "DELETE FROM review_decisions WHERE channel_id=?1",
    "DELETE FROM channels WHERE id=?1",
];

const THREAD_COLUMNS: &str = "id,parent_channel_id,anchor_message_id,thread_kind,followed,name,thread_state,agent_id";

type RawThread = (String, String, Option<String>, Option<String>, bool, String, Option<String>, String);

fn raw_thread(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawThread> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
}

fn decision_str(d: Decision) -> &'static str {
    match d {
        Decision::Approve => "approve",
        Decision::Deny => "deny",
        Decision::AlwaysAllow => "always_allow",
    }
}

fn parse_decision(s: &str) -> Option<Decision> {
    match s {
        "approve" => Some(Decision::Approve),
        "deny" => Some(Decision::Deny),
        "always_allow" => Some(Decision::AlwaysAllow),
        _ => None,
    }
}

fn approval_state_str(s: ApprovalState) -> &'static str {
    match s {
        ApprovalState::Pending => "pending",
        ApprovalState::Stale => "stale",
        ApprovalState::Decided(d) => decision_str(d),
    }
}

fn parse_approval_state(s: &str) -> Option<ApprovalState> {
    match s {
        "pending" => Some(ApprovalState::Pending),
        "stale" => Some(ApprovalState::Stale),
        other => parse_decision(other).map(ApprovalState::Decided),
    }
}

impl Store {
    /// Open the database file. A sub-agent run cannot survive a restart, so threads still `Running` become `Cancelled`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self::init(conn)?;
        store.cancel_running_sub_agents()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        if self.schema_version()? < 1 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        if self.schema_version()? < 2 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        if self.schema_version()? < 3 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE worktrees ADD COLUMN owned INTEGER NOT NULL DEFAULT 1;
                CREATE TABLE channel_branches (channel_id TEXT PRIMARY KEY REFERENCES channels(id), branch TEXT NOT NULL);")?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        if self.schema_version()? < 4 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V4)?;
            let mut stmt = tx.prepare(
                "SELECT c.id,c.project_id,c.name,c.archived,COALESCE(b.branch,w.branch),
                        COALESCE((SELECT MAX(s.ord) FROM sessions s WHERE s.channel_id=c.id),0) AS newest_session
                 FROM channels c LEFT JOIN channel_branches b ON b.channel_id=c.id
                 LEFT JOIN worktrees w ON w.channel_id=c.id
                 WHERE c.project_id IS NOT NULL ORDER BY c.project_id,c.id"
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?;
            let mut legacy = Vec::new();
            for row in rows {
                legacy.push(row?);
            }
            drop(stmt);
            type ChannelGroups = std::collections::HashMap<(String, String), Vec<(String, bool, i64)>>;
            let mut groups: ChannelGroups =
                std::collections::HashMap::new();
            for (id, project, name, archived, branch, session_ord) in legacy {
                let (kind, key) = if let Some(branch) = branch {
                    (ProjectChannelKind::Branch, branch)
                } else if name.trim().eq_ignore_ascii_case("general") {
                    (ProjectChannelKind::General, String::new())
                } else {
                    (ProjectChannelKind::ArchivedLegacy, String::new())
                };
                groups
                    .entry((
                        project.clone(),
                        if kind == ProjectChannelKind::General {
                            "general".into()
                        } else if kind == ProjectChannelKind::Branch {
                            format!("branch:{key}")
                        } else {
                            format!("legacy:{id}")
                        },
                    ))
                    .or_default()
                    .push((id.clone(), archived, session_ord));
                // The branch string is recoverable from the grouping key.
                let _ = (kind, key);
            }
            for ((project, group), mut channels) in groups {
                channels.sort_by(|a, b| {
                    a.1.cmp(&b.1)
                        .then_with(|| b.2.cmp(&a.2))
                        .then_with(|| b.0.cmp(&a.0))
                });
                let canonical = channels.iter().find(|c| !c.1).map(|c| c.0.clone());
                for (id, was_archived, _) in channels {
                    let is_canonical = canonical.as_deref() == Some(id.as_str());
                    let (kind, branch) = if group == "general" && canonical.is_some() {
                        (
                            if is_canonical {
                                "general"
                            } else {
                                "archived_legacy"
                            },
                            None,
                        )
                    } else if let Some(branch) = group.strip_prefix("branch:") {
                        (
                            if is_canonical {
                                "branch"
                            } else {
                                "archived_legacy"
                            },
                            Some(branch),
                        )
                    } else {
                        ("archived_legacy", None)
                    };
                    let link = if is_canonical {
                        None
                    } else {
                        canonical.clone()
                    };
                    tx.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name,archived_into) VALUES(?1,?2,?3,?4,?5)", params![id,project,kind,branch,link])?;
                    if group.starts_with("legacy:")
                        || (canonical.is_some() && !is_canonical)
                        || was_archived
                    {
                        tx.execute("UPDATE channels SET archived=1 WHERE id=?1", params![id])?;
                    }
                }
            }
            tx.execute_batch("INSERT OR IGNORE INTO channel_agents(channel_id,agent_id)
                SELECT c.id,c.agent_id FROM channels c WHERE c.project_id IS NOT NULL;
                INSERT OR IGNORE INTO project_agents(project_id,agent_id)
                SELECT c.project_id,c.agent_id FROM channels c WHERE c.project_id IS NOT NULL;
                INSERT OR IGNORE INTO project_main_agents(project_id,agent_id)
                SELECT p.id,(SELECT c.agent_id FROM channels c JOIN agents a ON a.id=c.agent_id
                    WHERE c.project_id=p.id AND a.archived=0 AND a.deleted=0
                    ORDER BY CASE WHEN c.id=(SELECT pc.channel_id FROM project_channels pc WHERE pc.project_id=c.project_id AND pc.kind='general') THEN 0 ELSE 1 END,c.ord LIMIT 1)
                FROM projects p WHERE EXISTS(SELECT 1 FROM channels c JOIN agents a ON a.id=c.agent_id
                    WHERE c.project_id=p.id AND a.archived=0 AND a.deleted=0);")?;
            tx.pragma_update(None, "user_version", 4)?;
            tx.commit()?;
        }
        if self.schema_version()? < 5 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE project_channels RENAME TO project_channels_v4;
                CREATE TABLE project_channels (
                    channel_id TEXT PRIMARY KEY REFERENCES channels(id),
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    kind TEXT NOT NULL CHECK(kind IN ('general','branch','archived_legacy','legacy')),
                    branch_name TEXT,
                    archived_into TEXT REFERENCES channels(id));
                INSERT INTO project_channels(channel_id,project_id,kind,branch_name,archived_into)
                    SELECT channel_id,project_id,kind,branch_name,archived_into FROM project_channels_v4;
                DROP TABLE project_channels_v4;
                DROP INDEX IF EXISTS one_project_general_channel;
                DROP INDEX IF EXISTS one_project_active_branch;
                CREATE UNIQUE INDEX one_project_general_channel ON project_channels(project_id) WHERE kind='general';
                CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id,branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;
                INSERT INTO project_channels(channel_id,project_id,kind)
                    SELECT c.id,c.project_id,'legacy' FROM channels c
                    WHERE c.project_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM project_channels pc WHERE pc.channel_id=c.id);
                INSERT OR IGNORE INTO channel_agents(channel_id,agent_id)
                    SELECT c.id,c.agent_id FROM channels c JOIN project_channels pc ON pc.channel_id=c.id;
                INSERT OR IGNORE INTO project_agents(project_id,agent_id)
                    SELECT c.project_id,c.agent_id FROM channels c JOIN project_channels pc ON pc.channel_id=c.id;")?;
            tx.pragma_update(None, "user_version", 5)?;
            tx.commit()?;
        }
        if self.schema_version()? < 6 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE projects ADD COLUMN avatar TEXT;")?;
            tx.pragma_update(None, "user_version", 6)?;
            tx.commit()?;
        }
        if self.schema_version()? < 7 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("ALTER TABLE project_channels ADD COLUMN repository TEXT NOT NULL DEFAULT '';
                DROP INDEX one_project_active_branch;
                CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id,repository,branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;")?;
            tx.pragma_update(None, "user_version", 7)?;
            tx.commit()?;
        }
        let has_column = |table: &str, column: &str| -> Result<bool> {
            Ok(self.conn.prepare(&format!("PRAGMA table_info({table})"))?.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?.iter().any(|name| name == column))
        };
        if self.schema_version()? < 8 || !has_column("approvals", "kind")? {
            let tx = self.conn.unchecked_transaction()?;
            if !has_column("messages", "author_json")? { tx.execute_batch("ALTER TABLE messages ADD COLUMN author_json TEXT;")?; }
            if !has_column("approvals", "kind")? { tx.execute_batch("ALTER TABLE approvals ADD COLUMN kind TEXT NOT NULL DEFAULT 'provider_tool';")?; }
            tx.execute_batch("CREATE TABLE IF NOT EXISTS agent_commits (
                commit_id TEXT NOT NULL, repo TEXT NOT NULL, channel_id TEXT NOT NULL REFERENCES channels(id),
                agent_id TEXT NOT NULL REFERENCES agents(id), response TEXT REFERENCES messages(id),
                PRIMARY KEY(repo,commit_id));")?;
            tx.pragma_update(None, "user_version", 8)?;
            tx.commit()?;
        }
        if self.schema_version()? < 9 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("CREATE TABLE always_allow_rules (
                id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents(id), project_id TEXT NOT NULL,
                target_json TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE(agent_id, project_id, target_json));")?;
            tx.pragma_update(None, "user_version", 9)?;
            tx.commit()?;
        }
        if self.schema_version()? < 10 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS autorun_responses (
                response_id TEXT PRIMARY KEY REFERENCES messages(id), channel_id TEXT NOT NULL REFERENCES channels(id),
                agent_id TEXT NOT NULL REFERENCES agents(id), provider TEXT NOT NULL, root TEXT NOT NULL, created_at INTEGER NOT NULL);")?;
            tx.pragma_update(None, "user_version", 10)?;
            tx.commit()?;
        }
        if self.schema_version()? < 11 {
            let tx = self.conn.unchecked_transaction()?;
            for (column, definition) in [
                ("parent_channel_id", "TEXT"),
                ("anchor_message_id", "TEXT"),
                ("thread_kind", "TEXT CHECK(thread_kind IN ('question','sub_agent'))"),
                ("followed", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                if !has_column("channels", column)? { tx.execute_batch(&format!("ALTER TABLE channels ADD COLUMN {column} {definition};"))?; }
            }
            if !has_column("messages", "source_message_id")? { tx.execute_batch("ALTER TABLE messages ADD COLUMN source_message_id TEXT;")?; }
            tx.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS one_thread_per_anchor ON channels(anchor_message_id) WHERE anchor_message_id IS NOT NULL;
                CREATE UNIQUE INDEX IF NOT EXISTS one_reference_per_source ON messages(source_message_id) WHERE source_message_id IS NOT NULL;
                CREATE INDEX IF NOT EXISTS threads_by_parent ON channels(parent_channel_id) WHERE parent_channel_id IS NOT NULL;")?;
            tx.pragma_update(None, "user_version", 11)?;
            tx.commit()?;
        }
        if self.schema_version()? < 12 {
            let tx = self.conn.unchecked_transaction()?;
            for (column, definition) in [
                ("thread_state", "TEXT CHECK(thread_state IN ('running','done','failed','cancelled','timed_out'))"),
                ("origin_message_id", "TEXT"),
            ] {
                if !has_column("channels", column)? { tx.execute_batch(&format!("ALTER TABLE channels ADD COLUMN {column} {definition};"))?; }
            }
            // Only question threads are limited to one for each anchor. A response can hold several sub-agent threads.
            tx.execute_batch("DROP INDEX IF EXISTS one_thread_per_anchor;
                CREATE UNIQUE INDEX one_thread_per_anchor ON channels(anchor_message_id) WHERE anchor_message_id IS NOT NULL AND thread_kind='question';")?;
            tx.pragma_update(None, "user_version", 12)?;
            tx.commit()?;
        }
        if self.schema_version()? < 13 {
            let tx = self.conn.unchecked_transaction()?;
            // Rebuild project_channels so the kind CHECK allows text channels. Every column, row, and index stays.
            tx.execute_batch("ALTER TABLE project_channels RENAME TO project_channels_v12;
                CREATE TABLE project_channels (
                    channel_id TEXT PRIMARY KEY REFERENCES channels(id),
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    kind TEXT NOT NULL CHECK(kind IN ('general','text','branch','archived_legacy','legacy')),
                    branch_name TEXT,
                    archived_into TEXT REFERENCES channels(id),
                    repository TEXT NOT NULL DEFAULT '');
                INSERT INTO project_channels(channel_id,project_id,kind,branch_name,archived_into,repository)
                    SELECT channel_id,project_id,kind,branch_name,archived_into,repository FROM project_channels_v12;
                DROP TABLE project_channels_v12;
                DROP INDEX IF EXISTS one_project_general_channel;
                DROP INDEX IF EXISTS one_project_active_branch;
                CREATE UNIQUE INDEX one_project_general_channel ON project_channels(project_id) WHERE kind='general';
                CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id,repository,branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;")?;
            if !has_column("projects", "show_archived")? { tx.execute_batch("ALTER TABLE projects ADD COLUMN show_archived INTEGER NOT NULL DEFAULT 0;")?; }
            tx.pragma_update(None, "user_version", 13)?;
            tx.commit()?;
        }
        if self.schema_version()? < 14 {
            let tx = self.conn.unchecked_transaction()?;
            // A muted channel shows no unread marker. The flag is off for every saved channel.
            if !has_column("channels", "muted")? { tx.execute_batch("ALTER TABLE channels ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;")?; }
            tx.pragma_update(None, "user_version", 14)?;
            tx.commit()?;
        }
        if self.schema_version()? < 15 {
            let tx = self.conn.unchecked_transaction()?;
            // Saved rail order and groups. Projects without a saved row keep their `visible_projects` order.
            tx.execute_batch(rail::RAIL_SCHEMA)?;
            tx.pragma_update(None, "user_version", 15)?;
            tx.commit()?;
        }
        if self.schema_version()? < 16 {
            let tx = self.conn.unchecked_transaction()?;
            // Native provider sessions, one per channel and agent.
            tx.execute_batch(agent_sessions::AGENT_SESSIONS_SCHEMA)?;
            tx.pragma_update(None, "user_version", 16)?;
            tx.commit()?;
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS direct_message_list (agent_id TEXT PRIMARY KEY REFERENCES agents(id));")?;
        self.conn.execute_batch(review::REVIEW_SCHEMA)?;
        Ok(())
    }

    fn require_active_agent(&self, agent: &AgentId) -> Result<()> {
        if self.agent_archived(agent)? {
            return Err(StoreError::Invalid("Restore the agent first.".into()));
        }
        Ok(())
    }

    /// Point the active general channel and every active text channel of the project at the main agent.
    fn reconcile_general_agent(
        conn: &Connection,
        project: &ProjectId,
        agent: &AgentId,
    ) -> Result<()> {
        let channels: Vec<(String, String, bool)> = {
            let mut stmt = conn.prepare(
                "SELECT pc.channel_id,c.agent_id,a.deleted FROM project_channels pc
                 JOIN channels c ON c.id=pc.channel_id JOIN agents a ON a.id=c.agent_id
                 WHERE pc.project_id=?1 AND pc.kind IN ('general','text') AND c.archived=0 ORDER BY c.ord",
            )?;
            stmt.query_map(params![project.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        for (channel, current_agent, current_agent_deleted) in channels {
            if !current_agent_deleted {
                conn.execute(
                    "INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)",
                    params![project.0, current_agent],
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)",
                    params![channel, current_agent],
                )?;
            }
            conn.execute(
                "INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)",
                params![project.0, agent.0],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)",
                params![channel, agent.0],
            )?;
            if current_agent != agent.0 {
                conn.execute(
                    "UPDATE channels SET agent_id=?2 WHERE id=?1",
                    params![channel, agent.0],
                )?;
                conn.execute(
                    "INSERT INTO sessions(id,channel_id) VALUES(?1,?2)",
                    params![SessionId::new().0, channel],
                )?;
            }
        }
        Ok(())
    }

    pub fn project_channel_metadata(
        &self,
        channel: &ChannelId,
    ) -> Result<Option<ProjectChannelMetadata>> {
        let row = self.conn.query_row("SELECT project_id,kind,branch_name,archived_into,repository FROM project_channels WHERE channel_id=?1", params![channel.0], |r| {
            Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?))
        }).optional()?;
        let Some((project, kind, branch, archived_into, repository)) = row else {
            return Ok(None);
        };
        let kind = ProjectChannelKind::parse(&kind)
            .ok_or_else(|| StoreError::Corrupt(format!("project channel kind {kind}")))?;
        Ok(Some(ProjectChannelMetadata {
            repository,
            project_id: ProjectId(project),
            channel_id: channel.clone(),
            kind,
            branch,
            archived_into: archived_into.map(ChannelId),
        }))
    }

    /// Find the active canonical channel for a branch in the project root repository.
    pub fn project_branch_channel(&self, project: &ProjectId, branch: &str) -> Result<Option<Channel>> {
        self.project_repository_branch_channel(project, "", branch)
    }

    pub fn project_repository_branch_channel(&self, project: &ProjectId, repository: &str, branch: &str) -> Result<Option<Channel>> {
        let id: Option<String> = self.conn.query_row(
            "SELECT pc.channel_id FROM project_channels pc JOIN channels c ON c.id=pc.channel_id
             WHERE pc.project_id=?1 AND pc.repository=?2 AND pc.kind='branch' AND pc.branch_name=?3 AND c.archived=0",
            params![project.0, repository, branch],
            |r| r.get(0),
        ).optional()?;
        id.map(|id| self.load_channel(&ChannelId(id))).transpose()
    }

    /// Return the unique project general channel, or create it with a validated active agent.
    pub fn ensure_project_general_channel(
        &self,
        project: &ProjectId,
        default_agent: &AgentId,
    ) -> Result<Channel> {
        self.require_active_agent(default_agent)?;
        let tx = self.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            params![project.0],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound(format!("project {}", project.0)));
        }
        let canonical: Option<String> = tx
            .query_row(
                "SELECT pc.channel_id FROM project_channels pc JOIN channels c ON c.id=pc.channel_id
                 WHERE pc.project_id=?1 AND pc.kind='general' AND c.archived=0",
                params![project.0],
                |r| r.get(0),
            )
            .optional()?;
        let stale_general: Option<String> = if canonical.is_none() {
            let stale = tx.query_row(
                "SELECT channel_id FROM project_channels WHERE project_id=?1 AND kind='general'",
                params![project.0],
                |r| r.get(0),
            ).optional()?;
            tx.execute("UPDATE project_channels SET kind='archived_legacy' WHERE project_id=?1 AND kind='general'", params![project.0])?;
            stale
        } else {
            None
        };
        let main_agent: Option<String> = tx
            .query_row(
                "SELECT agent_id FROM project_main_agents WHERE project_id=?1",
                params![project.0],
                |r| r.get(0),
            )
            .optional()?;
        let main_agent = main_agent
            .map(AgentId)
            .unwrap_or_else(|| default_agent.clone());
        self.require_active_agent(&main_agent)?;
        let channel_id = if let Some(id) = canonical {
            id
        } else {
            let candidate: Option<String> = tx.query_row(
                "SELECT c.id FROM channels c LEFT JOIN project_channels pc ON pc.channel_id=c.id
                 WHERE c.project_id=?1 AND c.archived=0 AND c.parent_channel_id IS NULL AND lower(trim(c.name))='general' AND pc.channel_id IS NULL
                 ORDER BY (SELECT COALESCE(MAX(s.ord),0) FROM sessions s WHERE s.channel_id=c.id) DESC,c.id DESC LIMIT 1",
                params![project.0], |r| r.get(0)).optional()?;
            if let Some(id) = candidate {
                tx.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,'general')", params![id,project.0])?;
                id
            } else {
                let channel = Channel {
                    id: ChannelId::new(),
                    project_id: Some(project.clone()),
                    name: "general".into(),
                    agent_id: main_agent.clone(),
                    workspace_mode: WorkspaceMode::MainCheckout,
                };
                self.insert_channel(&channel)?;
                self.create_session(&SessionId::new(), &channel.id)?;
                tx.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,'general')", params![channel.id.0,project.0])?;
                channel.id.0
            }
        };
        if let Some(stale) = stale_general {
            tx.execute(
                "UPDATE project_channels SET archived_into=?2 WHERE channel_id=?1",
                params![stale, channel_id],
            )?;
        }
        Self::reconcile_general_agent(&tx, project, &main_agent)?;
        tx.execute(
            "INSERT OR IGNORE INTO project_main_agents(project_id,agent_id) VALUES(?1,?2)",
            params![project.0, main_agent.0],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)",
            params![project.0, main_agent.0],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)",
            params![channel_id, main_agent.0],
        )?;
        tx.commit()?;
        self.load_channel(&ChannelId(channel_id))
    }

    pub fn set_project_main_agent(&self, project: &ProjectId, agent: &AgentId) -> Result<()> {
        self.require_active_agent(agent)?;
        let tx = self.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            params![project.0],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound(format!("project {}", project.0)));
        }
        tx.execute("INSERT INTO project_main_agents(project_id,agent_id) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET agent_id=excluded.agent_id", params![project.0,agent.0])?;
        tx.execute(
            "INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)",
            params![project.0, agent.0],
        )?;
        Self::reconcile_general_agent(&tx, project, agent)?;
        tx.commit()?;
        Ok(())
    }

    /// Whether the project shows its Archived section in the channel list.
    pub fn project_show_archived(&self, project: &ProjectId) -> Result<bool> {
        self.conn.query_row("SELECT show_archived FROM projects WHERE id=?1", params![project.0], |r| r.get::<_, bool>(0))
            .optional()?.ok_or_else(|| StoreError::NotFound(format!("project {}", project.0)))
    }

    pub fn set_project_show_archived(&self, project: &ProjectId, show: bool) -> Result<()> {
        if self.conn.execute("UPDATE projects SET show_archived=?2 WHERE id=?1", params![project.0, show])? == 0 {
            return Err(StoreError::NotFound(format!("project {}", project.0)));
        }
        Ok(())
    }

    /// True when the channel, or a thread of the channel, has a saved message.
    pub fn channel_has_messages(&self, channel: &ChannelId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE channel_id=?1 OR channel_id IN (SELECT id FROM channels WHERE parent_channel_id=?1))",
            params![channel.0],
            |r| r.get(0),
        )?)
    }

    /// Create an active text channel for the project main agent. The channel has no branch and works in the main checkout.
    pub fn create_project_text_channel(&self, project: &ProjectId, name: &str) -> Result<Channel> {
        let name = normalize_text_channel_name(name).map_err(StoreError::Invalid)?;
        let tx = self.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)", params![project.0], |r| r.get(0))?;
        if !exists {
            return Err(StoreError::NotFound(format!("project {}", project.0)));
        }
        let main_agent = self.project_main_agent(project)?
            .ok_or_else(|| StoreError::Invalid("This project has no main agent. Open the project first.".into()))?;
        self.require_active_agent(&main_agent)?;
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_channels pc JOIN channels c ON c.id=pc.channel_id
             WHERE pc.project_id=?1 AND pc.kind IN ('general','text') AND c.archived=0 AND lower(c.name)=?2)",
            params![project.0, name],
            |r| r.get(0),
        )?;
        if taken || name == "general" {
            return Err(StoreError::Invalid("A channel with this name already exists.".into()));
        }
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.clone()), name, agent_id: main_agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        self.insert_channel(&channel)?;
        self.create_session(&SessionId::new(), &channel.id)?;
        tx.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,'text')", params![channel.id.0, project.0])?;
        tx.execute("INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)", params![channel.id.0, main_agent.0])?;
        tx.execute("INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)", params![project.0, main_agent.0])?;
        tx.commit()?;
        Ok(channel)
    }

    pub fn project_main_agent(&self, project: &ProjectId) -> Result<Option<AgentId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT agent_id FROM project_main_agents WHERE project_id=?1",
                params![project.0],
                |r| r.get::<_, String>(0).map(AgentId),
            )
            .optional()?)
    }

    pub fn project_agents(&self, project: &ProjectId) -> Result<Vec<AgentId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT agent_id FROM project_agents WHERE project_id=?1 ORDER BY rowid")?;
        Ok(stmt
            .query_map(params![project.0], |r| r.get::<_, String>(0).map(AgentId))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn channel_agents(&self, channel: &ChannelId) -> Result<Vec<AgentId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT agent_id FROM channel_agents WHERE channel_id=?1 ORDER BY rowid")?;
        Ok(stmt
            .query_map(params![channel.0], |r| r.get::<_, String>(0).map(AgentId))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Add an active configured agent to both membership sets. This does not start a response.
    pub fn invite_agent_to_channel(
        &self,
        project: &ProjectId,
        channel: &ChannelId,
        agent: &AgentId,
    ) -> Result<()> {
        self.require_active_agent(agent)?;
        let tx = self.conn.unchecked_transaction()?;
        let channel_project: Option<String> = tx
            .query_row(
                "SELECT project_id FROM channels WHERE id=?1",
                params![channel.0],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if channel_project.as_deref() != Some(project.0.as_str()) {
            return Err(StoreError::Invalid(
                "Channel does not belong to this project.".into(),
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)",
            params![project.0, agent.0],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)",
            params![channel.0, agent.0],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn select_project_channel(&self, project: &ProjectId, channel: &ChannelId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND project_id=?2 AND archived=0 AND parent_channel_id IS NULL)",
            params![channel.0, project.0],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(StoreError::Invalid(
                "Select an active channel from this project.".into(),
            ));
        }
        tx.execute("INSERT INTO project_last_channels(project_id,channel_id) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET channel_id=excluded.channel_id", params![project.0,channel.0])?;
        tx.execute("INSERT INTO project_channel_views(channel_id,viewed_order) VALUES(?1,COALESCE((SELECT MAX(viewed_order)+1 FROM project_channel_views),1)) ON CONFLICT(channel_id) DO UPDATE SET viewed_order=excluded.viewed_order", params![channel.0])?;
        tx.commit()?;
        Ok(())
    }

    pub fn last_project_channel(&self, project: &ProjectId) -> Result<Option<Channel>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT channel_id FROM project_last_channels WHERE project_id=?1",
                params![project.0],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(None);
        };
        let valid: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND project_id=?2 AND archived=0 AND parent_channel_id IS NULL)",
            params![id, project.0],
            |r| r.get(0),
        )?;
        if !valid {
            self.conn.execute(
                "DELETE FROM project_last_channels WHERE project_id=?1",
                params![project.0],
            )?;
            return Ok(None);
        }
        Ok(Some(self.load_channel(&ChannelId(id))?))
    }

    /// Return the saved active channel, or ensure the project's general channel as a fallback.
    pub fn project_channel_or_fallback(
        &self,
        project: &ProjectId,
        default_agent: &AgentId,
    ) -> Result<Channel> {
        if let Some(channel) = self.last_project_channel(project)? {
            return Ok(channel);
        }
        self.ensure_project_general_channel(project, default_agent)
    }

    /// Change a project's canonical channel. Existing channel history remains attached to its ID.
    pub fn set_canonical_project_channel(
        &self,
        project: &ProjectId,
        channel: &ChannelId,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let selected_agent: Option<String> = tx.query_row(
            "SELECT c.agent_id FROM channels c LEFT JOIN project_channels pc ON pc.channel_id=c.id
             WHERE c.id=?1 AND c.project_id=?2 AND c.parent_channel_id IS NULL AND lower(trim(c.name))='general'
             AND (c.archived=0 OR pc.kind='archived_legacy') AND (pc.channel_id IS NULL OR pc.kind NOT IN ('branch','text'))",
            params![channel.0, project.0],
            |r| r.get(0),
        ).optional()?;
        let Some(selected_agent) = selected_agent else {
            return Err(StoreError::Invalid(
                "Select a general channel from this project.".into(),
            ));
        };
        let saved_main: Option<String> = tx.query_row(
            "SELECT agent_id FROM project_main_agents WHERE project_id=?1",
            params![project.0],
            |r| r.get(0),
        ).optional()?;
        let main_agent = saved_main.unwrap_or(selected_agent);
        self.require_active_agent(&AgentId(main_agent.clone()))?;
        tx.execute("UPDATE project_channels SET kind='archived_legacy',branch_name=NULL,archived_into=?2 WHERE project_id=?1 AND kind='general' AND channel_id!=?2",params![project.0,channel.0])?;
        tx.execute("UPDATE channels SET archived=1 WHERE project_id=?1 AND id!=?2 AND id IN (SELECT channel_id FROM project_channels WHERE project_id=?1 AND kind='archived_legacy' AND archived_into=?2)",params![project.0,channel.0])?;
        tx.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,'general') ON CONFLICT(channel_id) DO UPDATE SET project_id=excluded.project_id,kind='general',branch_name=NULL,archived_into=NULL",params![channel.0,project.0])?;
        tx.execute("UPDATE channels SET archived=0 WHERE id=?1",params![channel.0])?;
        tx.execute("INSERT INTO project_main_agents(project_id,agent_id) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET agent_id=excluded.agent_id",params![project.0,main_agent])?;
        Self::reconcile_general_agent(&tx,project,&AgentId(main_agent))?;
        tx.commit()?;
        Ok(())
    }

    /// Select one channel as the canonical channel for a branch. Retired duplicates keep their history.
    pub fn set_canonical_project_branch(
        &self,
        project: &ProjectId,
        branch: &str,
        channel: &ChannelId,
    ) -> Result<()> {
        let branch = branch.trim();
        if branch.is_empty() || branch.len() > 255 {
            return Err(StoreError::Invalid(
                "Enter a branch name of 1 to 255 bytes.".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let repository = self.project_channel_metadata(channel)?.map(|metadata| metadata.repository).unwrap_or_default();
        let valid: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM channels c LEFT JOIN project_channels pc ON pc.channel_id=c.id
            WHERE c.id=?1 AND c.project_id=?2 AND c.parent_channel_id IS NULL AND (pc.channel_id IS NULL OR (pc.kind='archived_legacy' AND pc.branch_name=?3) OR (pc.kind='branch' AND pc.branch_name=?3)))",params![channel.0,project.0,branch],|r|r.get(0))?;
        if !valid {
            return Err(StoreError::Invalid(
                "Select a channel from this project that belongs to this branch.".into(),
            ));
        }
        tx.execute("UPDATE project_channels SET kind='archived_legacy',archived_into=?3 WHERE project_id=?1 AND kind='branch' AND branch_name=?2 AND channel_id!=?3 AND repository=?4",params![project.0,branch,channel.0,repository])?;
        tx.execute("UPDATE channels SET archived=1 WHERE id IN (SELECT channel_id FROM project_channels WHERE project_id=?1 AND kind='archived_legacy' AND branch_name=?2 AND archived_into=?3 AND repository=?4)",params![project.0,branch,channel.0,repository])?;
        tx.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name,repository) VALUES(?1,?2,'branch',?3,?4)
            ON CONFLICT(channel_id) DO UPDATE SET project_id=excluded.project_id,kind='branch',branch_name=excluded.branch_name,repository=excluded.repository,archived_into=NULL",params![channel.0,project.0,branch,repository])?;
        tx.execute(
            "UPDATE channels SET archived=0 WHERE id=?1",
            params![channel.0],
        )?;
        tx.execute("INSERT INTO channel_branches(channel_id,branch) VALUES(?1,?2) ON CONFLICT(channel_id) DO UPDATE SET branch=excluded.branch",params![channel.0,branch])?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_channel_topic(
        &self,
        channel: &ChannelId,
        expected_version: u64,
        text: &str,
        author: TopicAuthorKind,
        author_id: Option<&AgentId>,
    ) -> Result<ChannelTopic> {
        if text.len() > 4096 {
            return Err(StoreError::Invalid(
                "Topic must be 4096 bytes or fewer.".into(),
            ));
        }
        if (author == TopicAuthorKind::User && author_id.is_some())
            || (author == TopicAuthorKind::Agent && author_id.is_none())
        {
            return Err(StoreError::Invalid(
                "Topic author does not match its author kind.".into(),
            ));
        }
        if let Some(agent) = author_id {
            self.require_active_agent(agent)?;
        }
        let tx = self.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1)",
            params![channel.0],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
        let current: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version),0) FROM channel_topic_revisions WHERE channel_id=?1",
            params![channel.0],
            |r| r.get(0),
        )?;
        if current as u64 != expected_version {
            return Err(StoreError::Invalid(format!(
                "Topic changed. Expected version {expected_version}, found {current}."
            )));
        }
        let version = current + 1;
        tx.execute("INSERT INTO channel_topic_revisions(channel_id,version,text,author_kind,author_id) VALUES(?1,?2,?3,?4,?5)",params![channel.0,version,text,if author==TopicAuthorKind::User{"user"}else{"agent"},author_id.map(|a|a.0.as_str())])?;
        tx.commit()?;
        Ok(ChannelTopic {
            channel_id: channel.clone(),
            text: text.into(),
            version: version as u64,
        })
    }

    pub fn channel_topic(&self, channel: &ChannelId) -> Result<ChannelTopic> {
        let row=self.conn.query_row("SELECT version,text FROM channel_topic_revisions WHERE channel_id=?1 ORDER BY version DESC LIMIT 1",params![channel.0],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).optional()?;
        Ok(row
            .map(|(version, text)| ChannelTopic {
                channel_id: channel.clone(),
                text,
                version: version as u64,
            })
            .unwrap_or(ChannelTopic {
                channel_id: channel.clone(),
                text: String::new(),
                version: 0,
            }))
    }

    pub fn channel_topic_history(&self, channel: &ChannelId) -> Result<Vec<ChannelTopicRevision>> {
        let mut stmt=self.conn.prepare("SELECT version,text,author_kind,author_id FROM channel_topic_revisions WHERE channel_id=?1 ORDER BY version")?;
        let rows = stmt.query_map(params![channel.0], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut history = Vec::new();
        for row in rows {
            let (version, text, kind, author) = row?;
            let author_kind = match kind.as_str() {
                "user" => TopicAuthorKind::User,
                "agent" => TopicAuthorKind::Agent,
                _ => return Err(StoreError::Corrupt(format!("topic author kind {kind}"))),
            };
            history.push(ChannelTopicRevision {
                channel_id: channel.clone(),
                text,
                version: version as u64,
                author_kind,
                author_id: author.map(AgentId),
            });
        }
        Ok(history)
    }

    pub fn rename_project_branch(
        &self,
        project: &ProjectId,
        channel: &ChannelId,
        new_branch: &str,
    ) -> Result<()> {
        let branch = new_branch.trim();
        if branch.is_empty() || branch.len() > 255 {
            return Err(StoreError::Invalid(
                "Enter a branch name of 1 to 255 bytes.".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let n=tx.execute("UPDATE project_channels SET branch_name=?3 WHERE project_id=?1 AND channel_id=?2 AND kind='branch'",params![project.0,channel.0,branch])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!(
                "active branch channel {}",
                channel.0
            )));
        }
        tx.execute("INSERT INTO channel_branches(channel_id,branch) VALUES(?1,?2) ON CONFLICT(channel_id) DO UPDATE SET branch=excluded.branch",params![channel.0,branch])?;
        if tx.execute(
            "UPDATE worktrees SET branch=?2 WHERE channel_id=?1",
            params![channel.0, branch],
        )? == 0
        { /* branch metadata is still authoritative */ }
        tx.commit()?;
        Ok(())
    }

    /// Retire a branch explicitly. This releases its name for reuse and keeps all history.
    pub fn retire_project_branch(&self, project: &ProjectId, channel: &ChannelId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let n=tx.execute("UPDATE project_channels SET kind='archived_legacy' WHERE project_id=?1 AND channel_id=?2 AND kind='branch'",params![project.0,channel.0])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!(
                "active branch channel {}",
                channel.0
            )));
        }
        tx.execute(
            "UPDATE channels SET archived=1 WHERE id=?1",
            params![channel.0],
        )?;
        tx.execute(
            "DELETE FROM channel_branches WHERE channel_id=?1",
            params![channel.0],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Ensure a bootstrap-created project channel has catalogue metadata without changing its identity or runtime mode.
    pub fn register_bootstrap_project_channel(&self, channel: &ChannelId) -> Result<()> {
        let value = self.load_channel(channel)?;
        let Some(project) = value.project_id.as_ref() else { return Ok(()) };
        let tx = self.conn.unchecked_transaction()?;
        let has_metadata: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM project_channels WHERE channel_id=?1)", params![channel.0], |r| r.get(0))?;
        if !has_metadata {
            let has_general: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM project_channels WHERE project_id=?1 AND kind='general')", params![project.0], |r| r.get(0))?;
            let kind = if value.name.eq_ignore_ascii_case("general") && !has_general { "general" } else { "legacy" };
            tx.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,?3)", params![channel.0, project.0, kind])?;
        }
        tx.execute("INSERT OR IGNORE INTO channel_agents(channel_id,agent_id) VALUES(?1,?2)", params![channel.0, value.agent_id.0])?;
        tx.execute("INSERT OR IGNORE INTO project_agents(project_id,agent_id) VALUES(?1,?2)", params![project.0, value.agent_id.0])?;
        tx.commit()?;
        Ok(())
    }

    pub fn bind_branch(&self, channel: &ChannelId, branch: &str) -> Result<()> {
        self.bind_repository_branch(channel, "", branch)
    }

    pub fn bind_repository_branch(&self, channel: &ChannelId, repository: &str, branch: &str) -> Result<()> {
        self.conn.execute_batch("SAVEPOINT bind_branch")?;
        let result = (|| {
            self.conn.execute(
                "INSERT INTO channel_branches (channel_id, branch) VALUES (?1, ?2)",
                params![channel.0, branch],
            )?;
            let project: Option<String> = self.conn.query_row(
                "SELECT project_id FROM channels WHERE id=?1",
                params![channel.0],
                |r| r.get(0),
            )?;
            if let Some(project) = project {
                self.conn.execute(
                    "UPDATE project_channels SET kind='archived_legacy',archived_into=?3
                     WHERE project_id=?1 AND kind='branch' AND branch_name=?2 AND channel_id!=?3 AND repository=?4",
                    params![project, branch, channel.0, repository],
                )?;
                self.conn.execute(
                    "UPDATE channels SET archived=1 WHERE id IN (
                        SELECT channel_id FROM project_channels
                        WHERE project_id=?1 AND kind='archived_legacy' AND branch_name=?2 AND archived_into=?3 AND repository=?4
                    )",
                    params![project, branch, channel.0, repository],
                )?;
                self.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name,repository) VALUES(?1,?2,'branch',?3,?4)
                    ON CONFLICT(channel_id) DO UPDATE SET project_id=excluded.project_id,kind='branch',branch_name=excluded.branch_name,repository=excluded.repository,archived_into=NULL",params![channel.0,project,branch,repository])?;
                self.conn.execute("UPDATE channels SET archived=0 WHERE id=?1",params![channel.0])?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.conn.execute_batch("RELEASE bind_branch")?,
            Err(error) => {
                self.conn.execute_batch("ROLLBACK TO bind_branch; RELEASE bind_branch")?;
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn channel_branch(&self, channel: &ChannelId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT branch FROM channel_branches WHERE channel_id = ?1",
                params![channel.0],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn visible_projects(&self) -> Result<Vec<Project>> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS hidden_projects (project_id TEXT PRIMARY KEY REFERENCES projects(id));")?;
        let hidden: Vec<String> = self
            .conn
            .prepare("SELECT project_id FROM hidden_projects")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(self
            .list_projects()?
            .into_iter()
            .filter(|p| !hidden.contains(&p.id.0))
            .collect())
    }
    pub fn hide_project(&self, id: &ProjectId) -> Result<()> {
        self.visible_projects()?;
        self.selected_project()?;
        self.conn.execute("DELETE FROM always_allow_rules WHERE project_id=?1", params![id.0])?;
        self.conn.execute(
            "INSERT OR IGNORE INTO hidden_projects VALUES (?1)",
            params![id.0],
        )?;
        self.conn.execute(
            "DELETE FROM project_selection WHERE project_id=?1",
            params![id.0],
        )?;
        // A hidden project leaves its rail group. A group without members is deleted.
        self.rail_normalize()?;
        Ok(())
    }
    pub fn selected_project(&self) -> Result<Option<Project>> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS project_selection (singleton INTEGER PRIMARY KEY CHECK(singleton=1), project_id TEXT NOT NULL REFERENCES projects(id));")?;
        Ok(self.conn.query_row("SELECT p.id,p.name,p.path FROM projects p JOIN project_selection s ON s.project_id=p.id WHERE s.singleton=1", [], |r| Ok(Project { id: ProjectId(r.get(0)?), name: r.get(1)?, path: r.get(2)? })).optional()?)
    }
    pub fn select_project(&self, project: &ProjectId) -> Result<()> {
        self.visible_projects()?;
        self.conn.execute(
            "DELETE FROM hidden_projects WHERE project_id=?1",
            params![project.0],
        )?;
        self.selected_project()?;
        self.conn.execute("INSERT INTO project_selection VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET project_id=excluded.project_id", params![project.0])?;
        Ok(())
    }
    pub fn import_project_folder(&self, path: &Path) -> Result<Project> {
        let canonical = path
            .canonicalize()
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if !canonical.is_dir() {
            return Err(StoreError::Corrupt("Select a folder.".into()));
        }
        for project in self.list_projects()? {
            if Path::new(&project.path).canonicalize().ok().as_ref() == Some(&canonical) {
                self.select_project(&project.id)?;
                return Ok(project);
            }
        }
        let project = Project {
            id: ProjectId::new(),
            name: canonical
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: canonical.to_string_lossy().into_owned(),
        };
        let tx = self.conn.unchecked_transaction()?;
        self.insert_project(&project)?;
        self.select_project(&project.id)?;
        tx.commit()?;
        Ok(project)
    }
    pub fn change_project_folder(&self, id: &ProjectId, path: &Path) -> Result<()> {
        let canonical = path
            .canonicalize()
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if !canonical.is_dir() {
            return Err(StoreError::Corrupt("Select a folder.".into()));
        }
        let projects = self.list_projects()?;
        let current = projects
            .iter()
            .find(|p| p.id == *id)
            .ok_or_else(|| StoreError::NotFound(id.0.clone()))?;
        if Path::new(&current.path).canonicalize().ok().as_ref() == Some(&canonical) {
            return Ok(());
        }
        let destination = projects.iter().find(|p| {
            p.id != *id && Path::new(&p.path).canonicalize().ok().as_ref() == Some(&canonical)
        });
        if destination.is_some() {
            return Err(StoreError::Invalid(
                "This folder belongs to another project.".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "UPDATE projects SET path=?2 WHERE id=?1",
            params![id.0, canonical.to_string_lossy()],
        )?;
        // A folder change retains the project and conversation identities.
        for (channel, archived) in self.all_channels_for_project(id)? {
            if !archived {
                self.delete_worktree(&channel.id)?;
                self.conn.execute(
                    "DELETE FROM channel_branches WHERE channel_id=?1",
                    params![channel.id.0],
                )?;
                self.create_session(&SessionId::new(), &channel.id)?;
                if !crate::worktree::WorktreeManager::is_repo(&canonical) {
                    self.set_workspace_mode(&channel.id, WorkspaceMode::MainCheckout)?;
                }
            }
        }
        self.select_project(id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn insert_always_allow_rule(&self, agent: &AgentId, project: &ProjectId, target: &RuleTarget) -> Result<AlwaysAllowRule> {
        self.require_active_agent(agent)?;
        if !self.list_projects()?.iter().any(|p| p.id == *project) { return Err(StoreError::NotFound(project.0.clone())); }
        let target = match target {
            RuleTarget::Host { host } => RuleTarget::Host {
                host: crate::always_allow::normalize_host(host).ok_or_else(|| StoreError::Invalid("Invalid network host.".into()))?,
            },
            RuleTarget::Command { words } if !words.is_empty() && !words[0].is_empty() => target.clone(),
            _ => return Err(StoreError::Invalid("Choose a command prefix.".into())),
        };
        let json = serde_json::to_string(&target)?;
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        self.conn.execute("INSERT OR IGNORE INTO always_allow_rules(id,agent_id,project_id,target_json,created_at) VALUES(?1,?2,?3,?4,?5)", params![id, agent.0, project.0, json, created_at])?;
        self.always_allow_rules(project)?.into_iter().find(|rule| rule.agent == *agent && rule.target == target).ok_or_else(|| StoreError::Corrupt("Rule insert failed.".into()))
    }

    pub fn always_allow_rules(&self, project: &ProjectId) -> Result<Vec<AlwaysAllowRule>> {
        let mut stmt = self.conn.prepare("SELECT id,agent_id,target_json,created_at FROM always_allow_rules WHERE project_id=?1 ORDER BY created_at,id")?;
        let rows = stmt.query_map(params![project.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)))?;
        rows.map(|row| {
            let (id, agent, target, created_at) = row?;
            Ok(AlwaysAllowRule {
                id,
                agent: AgentId(agent),
                project: project.clone(),
                target: serde_json::from_str(&target)?,
                created_at,
            })
        }).collect()
    }

    pub fn delete_always_allow_rule(&self, project: &ProjectId, id: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM always_allow_rules WHERE project_id=?1 AND id=?2", params![project.0, id])? > 0)
    }

    pub fn matching_always_allow_rule(&self, agent: &AgentId, project: &ProjectId, request: &RuleRequest<'_>) -> Result<Option<AlwaysAllowRule>> {
        Ok(self.always_allow_rules(project)?.into_iter().find(|rule| rule.agent == *agent && crate::always_allow::matches(&rule.target, request)))
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, name, path) VALUES (?1, ?2, ?3)",
            params![p.id.0, p.name, p.path],
        )?;
        Ok(())
    }

    pub fn project_avatar(&self, id: &ProjectId) -> Result<Option<String>> {
        self.conn.query_row("SELECT avatar FROM projects WHERE id=?1", params![id.0], |r| r.get(0))
            .optional()?.ok_or_else(|| StoreError::NotFound(id.0.clone()))
    }

    pub fn update_project_profile(&self, id: &ProjectId, name: &str, avatar: Option<&str>) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(StoreError::Invalid("Use a project name between 1 and 100 characters.".into()));
        }
        if let Some(value) = avatar {
            let parts: Vec<_> = value.split(':').collect();
            if parts.len() != 3 || parts[0] != "lucide"
                || !parts[1].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                || !(matches!(parts[2], "ansi-1" | "ansi-2" | "ansi-3" | "ansi-4" | "ansi-5" | "ansi-6")
                    || (parts[2].len() == 7 && parts[2].starts_with('#')
                        && parts[2][1..].chars().all(|c| c.is_ascii_hexdigit()))) {
                return Err(StoreError::Invalid("Choose a valid project icon and color.".into()));
            }
        }
        if self.conn.execute("UPDATE projects SET name=?2,avatar=?3 WHERE id=?1", params![id.0, name, avatar])? == 0 {
            return Err(StoreError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, name, path FROM projects ORDER BY rowid")?;
        let rows = statement.query_map([], |r| {
            Ok(Project {
                id: ProjectId(r.get(0)?),
                name: r.get(1)?,
                path: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn create_agent(&self, cfg: &AgentConfig) -> Result<AgentId> {
        cfg.validate().map_err(StoreError::Invalid)?;
        if cfg.model.is_none() {
            return Err(StoreError::Invalid("Select a model.".into()));
        }
        let tx = self.conn.unchecked_transaction()?;
        let id = AgentId::new();
        self.insert_agent(&id, cfg)?;
        self.ensure_default_conversation(&id)?;
        tx.commit()?;
        Ok(id)
    }

    /// Create the default chat for internal bootstrap agents, or return its stable ID.
    pub fn ensure_default_conversation(&self, agent: &AgentId) -> Result<Channel> {
        match self.default_conversation(agent) {
            Ok(channel) => return Ok(channel),
            Err(StoreError::NotFound(_)) => (),
            Err(error) => return Err(error),
        }
        self.conn.execute_batch("SAVEPOINT default_conversation")?;
        let result = (|| {
            let chat = self.create_conversation(agent, None, "General")?;
            self.conn.execute(
                "UPDATE channels SET is_default=1 WHERE id=?1",
                params![chat.id.0],
            )?;
            Ok(chat)
        })();
        match result {
            Ok(chat) => {
                self.conn.execute_batch("RELEASE default_conversation")?;
                Ok(chat)
            }
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO default_conversation; RELEASE default_conversation",
                )?;
                Err(error)
            }
        }
    }

    pub fn default_conversation(&self, agent: &AgentId) -> Result<Channel> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM channels WHERE agent_id=?1 AND is_default=1",
                params![agent.0],
                |r| r.get(0),
            )
            .optional()?;
        self.conversations_for_agent(agent)?
            .into_iter()
            .find(|c| Some(&c.id.0) == id.as_ref())
            .ok_or_else(|| StoreError::NotFound("default conversation".into()))
    }

    /// DM list membership is independent of creating an agent or its empty default chat.
    pub fn add_direct_message(&self, agent: &AgentId) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO direct_message_list (agent_id) VALUES (?1)",
            params![agent.0],
        )?;
        Ok(())
    }

    pub fn direct_message_agents(&self) -> Result<Vec<AgentId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT agent_id FROM direct_message_list ORDER BY rowid")?;
        Ok(stmt
            .query_map([], |r| r.get::<_, String>(0).map(AgentId))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn set_agent_archived(&self, id: &AgentId, archived: bool) -> Result<()> {
        if self.conn.execute(
            "UPDATE agents SET archived=?2 WHERE id=?1 AND deleted=0",
            params![id.0, archived],
        )? == 0
        {
            return Err(StoreError::NotFound(id.0.clone()));
        }
        Ok(())
    }

    pub fn agent_archived(&self, id: &AgentId) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT archived FROM agents WHERE id=?1 AND deleted=0",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(id.0.clone()))
    }

    /// Include archived agents for Settings. Exclude deleted tombstones.
    pub fn all_agents(&self) -> Result<Vec<(AgentId, AgentConfig, bool)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,config_toml,archived FROM agents WHERE deleted=0 ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?;
        rows.map(|r| {
            let (id, text, archived) = r?;
            Ok((AgentId(id), AgentConfig::from_toml(&text)?, archived))
        })
        .collect()
    }

    pub fn delete_agent(
        &self,
        id: &AgentId,
        options: DeleteAgentOptions,
    ) -> Result<DeleteAgentResult> {
        self.agent_archived(id)?;
        let tx = self.conn.unchecked_transaction()?;
        let channels = self.conversations_for_agent(id)?;
        let projects: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT project_id FROM project_agents WHERE agent_id=?1
                 UNION SELECT project_id FROM project_main_agents WHERE agent_id=?1
                 UNION SELECT project_id FROM channels WHERE agent_id=?1 AND project_id IS NOT NULL",
            )?;
            stmt.query_map(params![id.0], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut result = DeleteAgentResult::default();
        if options.delete_private_memory {
            result
                .private_scopes
                .push(PrivateMemoryScope::Agent(id.clone()));
            let projects: HashSet<ProjectId> = channels
                .iter()
                .filter_map(|c| c.project_id.clone())
                .collect();
            result
                .private_scopes
                .extend(
                    projects
                        .into_iter()
                        .map(|project| PrivateMemoryScope::AgentProject {
                            agent: id.clone(),
                            project,
                        }),
                );
        }
        if options.delete_conversations {
            tx.execute(
                "UPDATE project_channels SET archived_into=NULL
                 WHERE archived_into IN (SELECT id FROM channels WHERE agent_id=?1)",
                params![id.0],
            )?;
            for channel in self.channels_to_delete(id, &channels)? {
                self.purge_channel(&channel)?;
            }
        }
        tx.execute(
            "DELETE FROM channel_agents WHERE agent_id=?1",
            params![id.0],
        )?;
        tx.execute(
            "DELETE FROM project_agents WHERE agent_id=?1",
            params![id.0],
        )?;
        tx.execute(
            "DELETE FROM project_main_agents WHERE agent_id=?1",
            params![id.0],
        )?;
        tx.execute("DELETE FROM always_allow_rules WHERE agent_id=?1", params![id.0])?;
        // The tombstone has no provider, so its native sessions can never resume.
        tx.execute("DELETE FROM agent_sessions WHERE agent_id=?1", params![id.0])?;
        // Keep a minimal tombstone so retained history still has a readable author.
        let tombstone = AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
            reasoning_effort: None,
            name: "Deleted agent".into(),
            avatar: None,
            provider: String::new(),
            model: None,
            system_prompt: String::new(),
            policy: crate::capability::ApprovalPolicy::DenyAll,
            plugin_write: false,
        };
        self.conn.execute(
            "UPDATE agents SET config_toml=?2,archived=1,deleted=1 WHERE id=?1",
            params![id.0, tombstone.to_toml()?],
        )?;
        for project in projects {
            tx.execute(
                "INSERT OR IGNORE INTO project_main_agents(project_id,agent_id)
                SELECT ?1,pa.agent_id FROM project_agents pa JOIN agents a ON a.id=pa.agent_id
                WHERE pa.project_id=?1 AND a.archived=0 AND a.deleted=0 ORDER BY pa.rowid LIMIT 1",
                params![project],
            )?;
            let replacement: Option<String> = tx.query_row(
                "SELECT agent_id FROM project_main_agents WHERE project_id=?1",
                params![project],
                |r| r.get(0),
            ).optional()?;
            if let Some(agent) = replacement {
                Self::reconcile_general_agent(&tx, &ProjectId(project), &AgentId(agent))?;
            }
        }
        tx.commit()?;
        Ok(result)
    }

    /// Channels removed with an agent's conversations: those conversations plus the agent's threads and the
    /// threads of those conversations. Threads come first, so no thread outlives its parent.
    fn channels_to_delete(&self, agent: &AgentId, conversations: &[Channel]) -> Result<Vec<ChannelId>> {
        let mut stmt = self.conn.prepare("SELECT id,parent_channel_id,agent_id FROM channels WHERE parent_channel_id IS NOT NULL ORDER BY ord")?;
        let threads = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut doomed: Vec<ChannelId> = threads.into_iter()
            .filter(|(_, parent, owner)| owner == &agent.0 || conversations.iter().any(|c| c.id.0 == *parent))
            .map(|(id, ..)| ChannelId(id)).collect();
        doomed.extend(conversations.iter().map(|c| c.id.clone()));
        Ok(doomed)
    }

    pub fn create_conversation(
        &self,
        agent: &AgentId,
        project: Option<&ProjectId>,
        name: &str,
    ) -> Result<Channel> {
        if name.trim().is_empty() {
            return Err(StoreError::Invalid("Enter a conversation name.".into()));
        }
        if self.agent_archived(agent)? {
            return Err(StoreError::Invalid("Restore the agent first.".into()));
        }
        let channel = Channel {
            id: ChannelId::new(),
            project_id: project.cloned(),
            name: name.trim().into(),
            agent_id: agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        // This may be called inside create_agent's transaction.
        // Use a savepoint so create_agent can own the outer transaction.
        self.conn.execute_batch("SAVEPOINT create_conversation")?;
        let result = (|| {
            self.insert_channel(&channel)?;
            self.create_session(&SessionId::new(), &channel.id)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.conn
                .execute_batch("ROLLBACK TO create_conversation; RELEASE create_conversation")?;
            return Err(error);
        }
        self.conn.execute_batch("RELEASE create_conversation")?;
        Ok(channel)
    }

    /// All saved conversations, including retained histories of deleted agents. Threads are not conversations.
    pub fn all_conversations(&self) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare("SELECT id FROM channels WHERE parent_channel_id IS NULL ORDER BY ord")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| self.load_channel(&ChannelId(id)))
            .collect()
    }

    pub fn conversations_for_agent(&self, agent: &AgentId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,project_id,name,workspace_mode FROM channels WHERE agent_id=?1 AND parent_channel_id IS NULL ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![agent.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|r| {
            let (id, project, name, mode) = r?;
            Ok(Channel {
                id: ChannelId(id),
                project_id: project.map(ProjectId),
                name,
                agent_id: agent.clone(),
                workspace_mode: WorkspaceMode::parse(&mode)
                    .ok_or_else(|| StoreError::Corrupt(mode))?,
            })
        })
        .collect()
    }

    /// Internal bootstrap insertion permits an absent model. Public creation and edits require one.
    pub fn insert_agent(&self, id: &AgentId, cfg: &AgentConfig) -> Result<()> {
        cfg.validate().map_err(StoreError::Invalid)?;
        self.conn.execute(
            "INSERT INTO agents (id, config_toml) VALUES (?1, ?2)",
            params![id.0, cfg.to_toml()?],
        )?;
        Ok(())
    }

    pub fn load_agent(&self, id: &AgentId) -> Result<AgentConfig> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT config_toml FROM agents WHERE id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?;
        let text = text.ok_or_else(|| StoreError::NotFound(format!("agent {}", id.0)))?;
        Ok(AgentConfig::from_toml(&text)?)
    }

    pub fn list_agents(&self) -> Result<Vec<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare(
            "SELECT id, config_toml FROM agents WHERE archived=0 AND deleted=0 ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut agents = Vec::new();
        for row in rows {
            let (id, config) = row?;
            agents.push((AgentId(id), AgentConfig::from_toml(&config)?));
        }
        Ok(agents)
    }

    /// The first agent that a channel of `project` uses (archived channels count) with this provider and
    /// `plugin_write` value. Bootstrap uses this lookup; user-created conversations select an agent ID.
    pub fn find_project_agent(
        &self,
        project: &ProjectId,
        provider: &str,
        plugin_write: bool,
    ) -> Result<Option<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.config_toml FROM channels c JOIN agents a ON a.id = c.agent_id WHERE c.project_id = ?1 AND c.parent_channel_id IS NULL AND a.archived=0 AND a.deleted=0 ORDER BY c.ord",
        )?;
        let rows = statement.query_map(params![project.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, config) = row?;
            let config = AgentConfig::from_toml(&config)?;
            if config.provider == provider && config.plugin_write == plugin_write {
                return Ok(Some((AgentId(id), config)));
            }
        }
        Ok(None)
    }

    pub fn update_agent(&self, id: &AgentId, cfg: &AgentConfig) -> Result<()> {
        cfg.validate().map_err(StoreError::Invalid)?;
        if cfg.model.is_none() {
            return Err(StoreError::Invalid("Select a model.".into()));
        }
        let updated = self.conn.execute(
            "UPDATE agents SET config_toml = ?1 WHERE id = ?2 AND deleted=0",
            params![cfg.to_toml()?, id.0],
        )?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("agent {}", id.0)));
        }
        Ok(())
    }

    pub fn load_channel(&self, id: &ChannelId) -> Result<Channel> {
        let (project, name, agent, mode) = self.conn.query_row(
            "SELECT project_id,name,agent_id,workspace_mode FROM channels WHERE id=?1",
            params![id.0],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )?;
        let workspace_mode = WorkspaceMode::parse(&mode)
            .ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
        Ok(Channel {
            id: id.clone(),
            project_id: project.map(ProjectId),
            name,
            agent_id: AgentId(agent),
            workspace_mode,
        })
    }
    pub fn channel_archived(&self, id: &ChannelId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT archived FROM channels WHERE id=?1",
            params![id.0],
            |r| r.get(0),
        )?)
    }
    pub fn insert_channel(&self, c: &Channel) -> Result<()> {
        self.conn.execute(
            "INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.id.0, c.project_id.as_ref().map(|id| &id.0), c.name, c.agent_id.0, c.workspace_mode.as_str()],
        )?;
        Ok(())
    }

    /// Persist a complete channel before starting its runtime. Roll back on any storage failure.
    pub fn create_named_channel(
        &self,
        channel: &Channel,
        branch: Option<&str>,
        worktree: Option<&WorktreeRecord>,
    ) -> Result<()> {
        self.create_named_repository_channel(channel, "", branch, worktree)
    }

    pub fn create_named_repository_channel(
        &self,
        channel: &Channel,
        repository: &str,
        branch: Option<&str>,
        worktree: Option<&WorktreeRecord>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.insert_channel(channel)?;
        self.create_session(&SessionId::new(), &channel.id)?;
        if let Some(branch) = branch {
            self.bind_repository_branch(&channel.id, repository, branch)?;
        }
        if let Some(worktree) = worktree {
            self.save_worktree(worktree)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn channels_for_project(&self, p: &ProjectId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, agent_id, workspace_mode FROM channels WHERE project_id = ?1 AND archived = 0 AND parent_channel_id IS NULL ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![p.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, project_id, name, agent_id, mode) = row?;
            let workspace_mode = WorkspaceMode::parse(&mode)
                .ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
            out.push(Channel {
                id: ChannelId(id),
                project_id: project_id.map(ProjectId),
                name,
                agent_id: AgentId(agent_id),
                workspace_mode,
            });
        }
        Ok(out)
    }

    pub fn all_channels_for_project(&self, p: &ProjectId) -> Result<Vec<(Channel, bool)>> {
        let mut stmt = self.conn.prepare("SELECT id, project_id, name, agent_id, workspace_mode, archived FROM channels WHERE project_id = ?1 AND parent_channel_id IS NULL ORDER BY ord")?;
        let rows = stmt.query_map(params![p.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, project_id, name, agent_id, mode, archived) = row?;
            let workspace_mode = WorkspaceMode::parse(&mode)
                .ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
            out.push((
                Channel {
                    id: ChannelId(id),
                    project_id: project_id.map(ProjectId),
                    name,
                    agent_id: AgentId(agent_id),
                    workspace_mode,
                },
                archived,
            ));
        }
        Ok(out)
    }

    pub fn save_worktree(&self, w: &WorktreeRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO worktrees (channel_id, path, branch, base_commit, repo, owned) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(channel_id) DO UPDATE SET path = excluded.path, branch = excluded.branch,
               base_commit = excluded.base_commit, repo = excluded.repo, owned = excluded.owned",
            params![w.channel_id.0, w.path, w.branch, w.base_commit, w.repo, w.owned],
        )?;
        Ok(())
    }

    pub fn worktree_for(&self, channel: &ChannelId) -> Result<Option<WorktreeRecord>> {
        let row = self
            .conn
            .query_row(
                "SELECT path, branch, base_commit, repo, owned FROM worktrees WHERE channel_id = ?1",
                params![channel.0],
                |r| Ok(WorktreeRecord { channel_id: channel.clone(), path: r.get(0)?, branch: r.get(1)?, base_commit: r.get(2)?, repo: r.get(3)?, owned: r.get(4)? }),
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_worktree(&self, channel: &ChannelId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM worktrees WHERE channel_id = ?1",
            params![channel.0],
        )?;
        Ok(())
    }

    pub fn set_workspace_mode(&self, channel: &ChannelId, mode: WorkspaceMode) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE channels SET workspace_mode = ?2 WHERE id = ?1",
            params![channel.0, mode.as_str()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
        Ok(())
    }

    /// Hide a channel from its project. Its history stays in the database.
    pub fn restore_channel(&self, channel: &ChannelId) -> Result<()> {
        let row = self.load_channel(channel)?;
        if self.project_channel_metadata(channel)?.is_some_and(|metadata| metadata.kind == ProjectChannelKind::ArchivedLegacy) {
            return Err(StoreError::Invalid("This project channel is retained history. Choose it through canonical channel details to make it active.".into()));
        }
        if self.agent_archived(&row.agent_id)? {
            return Err(StoreError::Invalid("Restore the agent first.".into()));
        }
        self.conn.execute(
            "UPDATE channels SET archived=0 WHERE id=?1",
            params![channel.0],
        )?;
        Ok(())
    }
    /// Whether the channel is muted. A muted channel shows no unread marker.
    pub fn channel_muted(&self, channel: &ChannelId) -> Result<bool> {
        Ok(self.conn.query_row("SELECT muted FROM channels WHERE id=?1", params![channel.0], |r| r.get(0))?)
    }

    /// Mute or unmute a channel. A thread cannot be muted. Nothing else about the channel changes.
    pub fn set_channel_muted(&self, channel: &ChannelId, muted: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<bool> = tx.query_row("SELECT parent_channel_id IS NOT NULL FROM channels WHERE id=?1", params![channel.0], |r| r.get(0)).optional()?;
        match row {
            None => return Err(StoreError::NotFound(format!("channel {}", channel.0))),
            Some(true) => return Err(StoreError::Invalid("A thread cannot be muted.".into())),
            Some(false) => {}
        }
        tx.execute("UPDATE channels SET muted=?2 WHERE id=?1", params![channel.0, muted])?;
        tx.commit()?;
        Ok(())
    }

    /// Delete the worktree records of the given channels in one transaction. The channels stay.
    pub fn delete_worktrees(&self, channels: &[ChannelId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for channel in channels { tx.execute("DELETE FROM worktrees WHERE channel_id=?1", params![channel.0])?; }
        tx.commit()?;
        Ok(())
    }

    pub fn archive_channel(&self, channel: &ChannelId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE channels SET archived = 1 WHERE id = ?1",
            params![channel.0],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
        tx.execute("DELETE FROM agent_sessions WHERE channel_id=?1", params![channel.0])?;
        tx.commit()?;
        Ok(())
    }

    pub fn project_by_path(&self, path: &str) -> Result<Option<Project>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, path FROM projects WHERE path = ?1",
                params![path],
                |r| {
                    Ok(Project {
                        id: ProjectId(r.get(0)?),
                        name: r.get(1)?,
                        path: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// The first channel created in a project, if any.
    pub fn first_channel(&self, p: &ProjectId) -> Result<Option<Channel>> {
        Ok(self.channels_for_project(p)?.into_iter().next())
    }

    /// The most recently created session of a channel, if any.
    pub fn latest_session(&self, c: &ChannelId) -> Result<Option<SessionId>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE channel_id = ?1 ORDER BY ord DESC LIMIT 1",
                params![c.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.map(SessionId))
    }

    pub fn create_session(&self, id: &SessionId, channel: &ChannelId) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, channel_id) VALUES (?1, ?2)",
            params![id.0, channel.0],
        )?;
        Ok(())
    }

    pub fn set_resume_handle(&self, id: &SessionId, handle: Option<&str>) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE sessions SET resume_handle = ?2 WHERE id = ?1",
            params![id.0, handle],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("session {}", id.0)));
        }
        Ok(())
    }

    pub fn resume_handle(&self, id: &SessionId) -> Result<Option<String>> {
        let row: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT resume_handle FROM sessions WHERE id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?;
        row.ok_or_else(|| StoreError::NotFound(format!("session {}", id.0)))
    }

    pub fn insert_turn(&self, t: &Turn) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turns (id, session_id, state) VALUES (?1, ?2, ?3)",
            params![t.id.0, t.session_id.0, t.state.as_str()],
        )?;
        Ok(())
    }

    pub fn turn_state(&self, id: &TurnId) -> Result<TurnState> {
        let s: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM turns WHERE id = ?1",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?;
        let s = s.ok_or_else(|| StoreError::NotFound(format!("turn {}", id.0)))?;
        TurnState::parse(&s).ok_or_else(|| StoreError::Corrupt(format!("turn state {s}")))
    }

    pub fn update_turn_state(&self, id: &TurnId, to: TurnState) -> Result<()> {
        let from = self.turn_state(id)?;
        if !from.can_transition_to(to) {
            return Err(StoreError::InvalidTransition { from, to });
        }
        self.conn.execute(
            "UPDATE turns SET state = ?2 WHERE id = ?1",
            params![id.0, to.as_str()],
        )?;
        Ok(())
    }

    pub fn append_event(&self, e: &Event) -> Result<()> {
        let raw = e.raw.as_ref().map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "INSERT INTO events (id, session_id, seq, kind_json, raw_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![e.id.0, e.session_id.0, e.seq as i64, serde_json::to_string(&e.kind)?, raw],
        )?;
        Ok(())
    }

    pub fn events_for_session(&self, s: &SessionId) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, seq, kind_json, raw_json FROM events WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![s.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, seq, kind, raw) = row?;
            out.push(Event {
                id: EventId(id),
                session_id: s.clone(),
                seq: seq as u64,
                kind: serde_json::from_str::<EventKind>(&kind)?,
                raw: raw.map(|r| serde_json::from_str(&r)).transpose()?,
            });
        }
        Ok(out)
    }

    pub fn last_seq(&self, s: &SessionId) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM events WHERE session_id = ?1",
            params![s.0],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    pub fn record_agent_commit(&self, commit: &str, repo: &str, channel: &ChannelId, agent: &AgentId) -> Result<()> {
        self.conn.execute("INSERT INTO agent_commits(commit_id,repo,channel_id,agent_id) VALUES(?1,?2,?3,?4)", params![commit, repo, channel.0, agent.0])?;
        Ok(())
    }
    /// Audit row for a response that ran shell commands under sandbox autorun.
    pub fn record_autorun_response(&self, response: &MessageId, channel: &ChannelId, agent: &AgentId, provider: &str, root: &Path) -> Result<()> {
        self.conn.execute(
            "INSERT INTO autorun_responses(response_id,channel_id,agent_id,provider,root,created_at) VALUES(?1,?2,?3,?4,?5,strftime('%s','now'))",
            params![response.0, channel.0, agent.0, provider, root.to_string_lossy()],
        )?;
        Ok(())
    }
    pub fn autorun_responses(&self, channel: &ChannelId) -> Result<Vec<(MessageId, AgentId, String, PathBuf)>> {
        let mut stmt = self.conn.prepare("SELECT response_id,agent_id,provider,root FROM autorun_responses WHERE channel_id=?1 ORDER BY created_at, rowid")?;
        let rows = stmt.query_map(params![channel.0], |r| Ok((MessageId(r.get(0)?), AgentId(r.get(1)?), r.get(2)?, PathBuf::from(r.get::<_, String>(3)?))))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn link_agent_commits(&self, channel: &ChannelId, response: &MessageId) -> Result<usize> {
        Ok(self.conn.execute("UPDATE agent_commits SET response=?2 WHERE channel_id=?1 AND response IS NULL", params![channel.0, response.0])?)
    }

    pub fn save_approval(&self, a: &Approval) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO approvals (id, turn_id, tool, input_json, state, kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET state = excluded.state",
            params![a.id.0, a.turn_id.0, a.tool, serde_json::to_string(&a.input)?, approval_state_str(a.state), a.kind.as_str()],
        )?;
        // Audit rows are append-only: existing rows are never rewritten.
        for (n, rec) in a.audit.iter().enumerate() {
            tx.execute(
                "INSERT OR IGNORE INTO approval_audit (approval_id, n, decision, decider_json, policy) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![a.id.0, n as i64, decision_str(rec.decision), serde_json::to_string(&rec.decider)?, rec.policy],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_approval(&self, id: &ApprovalId) -> Result<Approval> {
        let row: Option<(String, String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT turn_id, tool, input_json, state, kind FROM approvals WHERE id = ?1",
                params![id.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (turn_id, tool, input, state, kind) =
            row.ok_or_else(|| StoreError::NotFound(format!("approval {}", id.0)))?;
        let state = parse_approval_state(&state)
            .ok_or_else(|| StoreError::Corrupt(format!("approval state {state}")))?;
        let mut stmt = self.conn.prepare(
            "SELECT decision, decider_json, policy FROM approval_audit WHERE approval_id = ?1 ORDER BY n",
        )?;
        let rows = stmt.query_map(params![id.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut audit = Vec::new();
        for row in rows {
            let (decision, decider, policy) = row?;
            audit.push(AuditRecord {
                approval_id: id.clone(),
                decision: parse_decision(&decision)
                    .ok_or_else(|| StoreError::Corrupt(format!("decision {decision}")))?,
                decider: serde_json::from_str::<Decider>(&decider)?,
                policy,
            });
        }
        Ok(Approval {
            kind: serde_json::from_value(serde_json::Value::String(kind))?,
            id: id.clone(),
            turn_id: TurnId(turn_id),
            tool,
            input: serde_json::from_str(&input)?,
            state,
            audit,
        })
    }

    pub fn conversation_context(
        &self,
        channel: &ChannelId,
    ) -> Result<crate::conversation_context::ConversationContext> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT state_json FROM conversation_context WHERE channel_id=?1",
                params![channel.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json
            .map(|j| serde_json::from_str(&j))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn set_conversation_context(
        &self,
        channel: &ChannelId,
        state: &crate::conversation_context::ConversationContext,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO conversation_context VALUES (?1,?2) ON CONFLICT(channel_id) DO UPDATE SET state_json=excluded.state_json", params![channel.0, serde_json::to_string(state)?])?;
        Ok(())
    }
    pub fn response_memory(
        &self,
        channel: &ChannelId,
    ) -> Result<Vec<(MessageId, crate::memory_items::MemoryChange)>> {
        let mut stmt=self.conn.prepare("SELECT response_id,change_json FROM response_memory WHERE channel_id=?1 ORDER BY rowid")?;
        let rows = stmt.query_map(params![channel.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (id, json) = row?;
            Ok((MessageId(id), serde_json::from_str(&json)?))
        })
        .collect()
    }
    pub fn record_response_memory(
        &self,
        channel: &ChannelId,
        response: &MessageId,
        change: &crate::memory_items::MemoryChange,
    ) -> Result<bool> {
        Ok(self
            .record_response_memory_at(channel, response, change)?
            .is_some())
    }
    /// Prefer a proposal's original response over a later turn that observes its approval.
    pub fn record_response_memory_at(
        &self,
        channel: &ChannelId,
        response: &MessageId,
        change: &crate::memory_items::MemoryChange,
    ) -> Result<Option<MessageId>> {
        use crate::memory_items::MemoryChange;
        let mut target = response.clone();
        if let MemoryChange::Saved { item, .. } = change
            && item.origin == crate::memory_items::MutationOrigin::Approved
            && let Some(proposal_id) = &item.proposal_id
            && let Some((id, _)) = self
                .response_memory(channel)?
                .into_iter()
                .find(|(_, c)| matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id))
            {
                target = id;
            }
        let key = match change {
            MemoryChange::Saved { key, .. } => key.clone(),
            MemoryChange::Proposed(p) => p.id.clone(),
        };
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO response_memory VALUES (?1,?2,?3,?4)",
            params![channel.0, target.0, key, serde_json::to_string(change)?],
        )? > 0;
        Ok(inserted.then_some(target))
    }
    /// Retry approval links using immutable receipts and already-persisted proposal rows.
    /// No response ID is inferred or created when the proposal row is still absent.
    pub fn reconcile_response_memory(
        &self,
        channel: &ChannelId,
        approvals: &[crate::memory_items::MemoryChange],
    ) -> Result<Vec<(MessageId, crate::memory_items::MemoryChange)>> {
        use crate::memory_items::MemoryChange;
        let records = self.response_memory(channel)?;
        let mut recorded = Vec::new();
        for approval in approvals {
            let MemoryChange::Saved { key, item } = approval else {
                continue;
            };
            if records
                .iter()
                .any(|(_, c)| matches!(c,MemoryChange::Saved{key:existing,..} if existing==key))
            {
                continue;
            }
            let Some(proposal_id) = &item.proposal_id else {
                continue;
            };
            if let Some((response, _)) = records
                .iter()
                .find(|(_, c)| matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id))
                && let Some(id) = self.record_response_memory_at(channel, response, approval)?
            {
                recorded.push((id, approval.clone()));
            }
        }
        Ok(recorded)
    }

    pub fn insert_message(&self, m: &Message) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, channel_id, role, blocks_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                m.id.0,
                m.channel_id.0,
                m.role.as_str(),
                serde_json::to_string(&m.blocks)?
            ],
        )?;
        Ok(())
    }

    pub fn insert_authored_message(&self, message: &Message, author: &crate::model::MessageAuthor) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.insert_message(message)?;
        tx.execute("UPDATE messages SET author_json=?2 WHERE id=?1", params![message.id.0, serde_json::to_string(author)?])?;
        tx.commit()?;
        Ok(())
    }

    /// Authors by message ID. A posted reference row takes its source message's author.
    pub fn message_authors(&self, channel: &ChannelId) -> Result<std::collections::HashMap<MessageId, MessageAuthor>> {
        let mut statement = self.conn.prepare("SELECT m.id,COALESCE(s.author_json,m.author_json) FROM messages m LEFT JOIN messages s ON s.id=m.source_message_id
            WHERE m.channel_id=?1 AND COALESCE(s.author_json,m.author_json) IS NOT NULL")?;
        let rows = statement.query_map(params![channel.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut authors = std::collections::HashMap::new();
        for row in rows { let (id, json) = row?; authors.insert(MessageId(id), serde_json::from_str(&json)?); }
        Ok(authors)
    }

    /// The saved author of one message, if it has one. User messages have none.
    pub fn message_author(&self, message: &MessageId) -> Result<Option<MessageAuthor>> {
        let json: Option<String> = self.conn.query_row("SELECT author_json FROM messages WHERE id=?1", params![message.0], |r| r.get(0)).optional()?.flatten();
        Ok(json.map(|json| serde_json::from_str(&json)).transpose()?)
    }

    /// The ID of the last saved message of a channel, of any role. A channel without messages gives `None`.
    pub fn last_message_id(&self, c: &ChannelId) -> Result<Option<MessageId>> {
        Ok(self.conn.query_row("SELECT id FROM messages WHERE channel_id = ?1 ORDER BY ord DESC LIMIT 1", params![c.0], |r| r.get::<_, String>(0)).optional()?.map(MessageId))
    }

    /// Saved messages, oldest first. A posted reference row loads with its source message's role and blocks.
    pub fn messages_for_channel(&self, c: &ChannelId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, COALESCE(s.role, m.role), COALESCE(s.blocks_json, m.blocks_json) FROM messages m
             LEFT JOIN messages s ON s.id = m.source_message_id WHERE m.channel_id = ?1 ORDER BY m.ord",
        )?;
        let rows = stmt.query_map(params![c.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, role, blocks) = row?;
            out.push(Message {
                id: MessageId(id),
                channel_id: c.clone(),
                role: Role::parse(&role)
                    .ok_or_else(|| StoreError::Corrupt(format!("role {role}")))?,
                blocks: serde_json::from_str::<Vec<Block>>(&blocks)?,
            });
        }
        Ok(out)
    }

    fn thread_from(&self, raw: RawThread) -> Result<ThreadInfo> {
        let (id, parent, anchor, kind, followed, name, state, agent) = raw;
        let kind = kind.as_deref().and_then(ThreadKind::parse).ok_or_else(|| StoreError::Corrupt(format!("thread_kind {kind:?}")))?;
        let state = state.as_deref().map(|s| ThreadState::parse(s).ok_or_else(|| StoreError::Corrupt(format!("thread_state {s}")))).transpose()?;
        let agent = AgentId(agent);
        let agent_name = self.load_agent(&agent).map(|config| config.name).unwrap_or_default();
        Ok(ThreadInfo { channel_id: ChannelId(id), parent_channel_id: ChannelId(parent), anchor_message_id: anchor.map(MessageId), kind, followed, name, state, agent_id: agent, agent_name })
    }

    fn threads_where(&self, condition: &str, value: &str) -> Result<Vec<ThreadInfo>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {THREAD_COLUMNS} FROM channels WHERE {condition} ORDER BY ord"))?;
        let rows = stmt.query_map(params![value], raw_thread)?.collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(|raw| self.thread_from(raw)).collect()
    }

    /// Thread columns of a channel, or `None` for a normal channel.
    pub fn thread_info(&self, channel: &ChannelId) -> Result<Option<ThreadInfo>> {
        Ok(self.threads_where("id=?1 AND parent_channel_id IS NOT NULL", &channel.0)?.into_iter().next())
    }

    /// The channel whose worktree and branch binding `channel` reads. A thread has no binding of its own and reads its
    /// parent's, so a read-only thread sees the same files as a read-only response in the parent. Any other channel
    /// reads its own. This is for read tools only: a write grant, a worktree lock, and the `worktree_*` tools use the
    /// channel's own binding, so a thread never has them.
    pub fn binding_channel(&self, channel: &ChannelId) -> Result<ChannelId> {
        Ok(self.thread_info(channel)?.map_or_else(|| channel.clone(), |thread| thread.parent_channel_id))
    }

    /// The question thread that starts from this message, if any. Sub-agent threads on the message are not
    /// counted; `threads_at_anchor` lists every thread.
    pub fn thread_for_anchor(&self, anchor: &MessageId) -> Result<Option<ThreadInfo>> {
        Ok(self.threads_at_anchor(anchor)?.into_iter().find(|thread| thread.kind == ThreadKind::Question))
    }

    /// Every thread on this message: its question thread and the sub-agent threads of its response, oldest first.
    pub fn threads_at_anchor(&self, anchor: &MessageId) -> Result<Vec<ThreadInfo>> {
        self.threads_where("anchor_message_id=?1", &anchor.0)
    }

    /// Every thread of a parent channel, oldest first, followed or not. A sub-agent thread whose parent turn
    /// is still running has no anchor yet.
    pub fn threads_for_channel(&self, parent: &ChannelId) -> Result<Vec<ThreadInfo>> {
        self.threads_where("parent_channel_id=?1", &parent.0)
    }

    /// Start a question thread from a saved message of a normal channel. The thread inherits the project,
    /// starts followed, and gets its own session. `agent` is the default responder; `name` comes from `thread_name`.
    pub fn create_thread(&self, parent: &ChannelId, anchor: &MessageId, name: &str, agent: &AgentId) -> Result<Channel> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("Enter a thread name.".into()));
        }
        self.require_active_agent(agent)?;
        let tx = self.conn.unchecked_transaction()?;
        let parent_channel = self.load_channel(parent)?;
        if self.thread_info(parent)?.is_some() {
            return Err(StoreError::Invalid("A thread cannot start another thread.".into()));
        }
        let row: Option<(String, String, Option<String>)> = self.conn
            .query_row("SELECT channel_id,role,source_message_id FROM messages WHERE id=?1", params![anchor.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((channel, role, source)) = row.filter(|(channel, ..)| channel == &parent.0) else {
            return Err(StoreError::Invalid("Start a thread from a saved message in this channel.".into()));
        };
        if source.is_some() {
            return Err(StoreError::Invalid("A message posted from a thread cannot start another thread.".into()));
        }
        if Role::parse(&role) == Some(Role::System) {
            return Err(StoreError::Invalid("Start a thread from a user or agent message.".into()));
        }
        if self.thread_for_anchor(anchor)?.is_some() {
            return Err(StoreError::Invalid("This message already has a thread.".into()));
        }
        let thread = Channel { id: ChannelId::new(), project_id: parent_channel.project_id, name: name.into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        self.insert_channel(&thread)?;
        self.conn.execute(
            "UPDATE channels SET parent_channel_id=?2,anchor_message_id=?3,thread_kind=?4,followed=1 WHERE id=?1",
            params![thread.id.0, channel, anchor.0, ThreadKind::Question.as_str()],
        )?;
        self.create_session(&SessionId::new(), &thread.id)?;
        tx.commit()?;
        Ok(thread)
    }

    /// The latest user message that someone wrote in a channel. Posted references do not count.
    pub fn last_user_message_id(&self, channel: &ChannelId) -> Result<Option<MessageId>> {
        Ok(self.conn.query_row("SELECT id FROM messages WHERE channel_id=?1 AND role='user' AND source_message_id IS NULL ORDER BY ord DESC LIMIT 1", params![channel.0], |r| r.get::<_, String>(0)).optional()?.map(MessageId))
    }

    /// The latest agent message of a channel, for the result of a sub-agent run.
    pub fn last_agent_message(&self, channel: &ChannelId) -> Result<Option<Message>> {
        Ok(self.messages_for_channel(channel)?.into_iter().rev().find(|m| m.role == Role::Agent))
    }

    /// Start a sub-agent thread under a normal channel. It inherits the project, has its own session, is not
    /// followed, and is `Running`. It has no anchor: `anchor_sub_agent_threads` sets it when the parent turn ends.
    /// `agent` is the sub-agent. The thread remembers the parent's latest user message, which anchors it when the
    /// parent turn saves no response. The brief is not stored here: the engine saves it as the first message.
    pub fn create_sub_agent_thread(&self, parent: &ChannelId, name: &str, agent: &AgentId) -> Result<Channel> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("Enter a thread name.".into()));
        }
        self.require_active_agent(agent)?;
        let tx = self.conn.unchecked_transaction()?;
        let parent_channel = self.load_channel(parent)?;
        if self.thread_info(parent)?.is_some() {
            return Err(StoreError::Invalid("A thread cannot start another thread.".into()));
        }
        let origin = self.last_user_message_id(parent)?.ok_or_else(|| StoreError::Invalid("A sub-agent needs a user message in the parent channel.".into()))?;
        let thread = Channel { id: ChannelId::new(), project_id: parent_channel.project_id, name: name.into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        self.insert_channel(&thread)?;
        self.conn.execute(
            "UPDATE channels SET parent_channel_id=?2,thread_kind=?3,followed=0,thread_state=?4,origin_message_id=?5 WHERE id=?1",
            params![thread.id.0, parent.0, ThreadKind::SubAgent.as_str(), ThreadState::Running.as_str(), origin.0],
        )?;
        self.create_session(&SessionId::new(), &thread.id)?;
        tx.commit()?;
        Ok(thread)
    }

    /// End a sub-agent run. Only a `Running` thread changes, so the first final state wins. Returns whether this
    /// call changed it. A question thread, an unknown thread, or `Running` as the new state is an error.
    pub fn finish_sub_agent_thread(&self, thread: &ChannelId, state: ThreadState) -> Result<bool> {
        if state == ThreadState::Running {
            return Err(StoreError::Invalid("Choose a final state.".into()));
        }
        let changed = self.conn.execute("UPDATE channels SET thread_state=?2 WHERE id=?1 AND thread_kind='sub_agent' AND thread_state='running'", params![thread.0, state.as_str()])?;
        if changed == 0 && self.thread_info(thread)?.is_none_or(|info| info.kind != ThreadKind::SubAgent) {
            return Err(StoreError::NotFound(format!("sub-agent thread {}", thread.0)));
        }
        Ok(changed > 0)
    }

    /// At the end of a parent turn, hang the sub-agent threads that the turn started under their anchor. A thread
    /// of the turn that started from the parent's latest user message takes `response`; with no response, and for a
    /// thread of an older turn that was never anchored, the anchor is the user message the thread remembers.
    /// Returns the threads that changed.
    pub fn anchor_sub_agent_threads(&self, parent: &ChannelId, response: Option<&MessageId>) -> Result<Vec<ThreadInfo>> {
        let current = self.last_user_message_id(parent)?;
        let tx = self.conn.unchecked_transaction()?;
        let mut stmt = self.conn.prepare("SELECT id,origin_message_id FROM channels WHERE parent_channel_id=?1 AND thread_kind='sub_agent' AND anchor_message_id IS NULL ORDER BY ord")?;
        let pending = stmt.query_map(params![parent.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut changed = Vec::new();
        for (id, origin) in pending {
            let anchor = match (response, &origin) {
                (Some(response), Some(origin)) if current.as_ref().is_some_and(|c| &c.0 == origin) => Some(response.0.clone()),
                _ => origin,
            };
            let Some(anchor) = anchor else { continue };
            self.conn.execute("UPDATE channels SET anchor_message_id=?2 WHERE id=?1", params![id, anchor])?;
            changed.push(ChannelId(id));
        }
        tx.commit()?;
        changed.iter().filter_map(|id| self.thread_info(id).transpose()).collect()
    }

    /// Startup recovery: a sub-agent run cannot outlive the app, so every `Running` thread becomes `Cancelled`.
    /// Its partial transcript stays. Returns how many threads changed.
    pub fn cancel_running_sub_agents(&self) -> Result<usize> {
        Ok(self.conn.execute("UPDATE channels SET thread_state='cancelled' WHERE thread_kind='sub_agent' AND thread_state='running'", [])?)
    }

    pub fn set_thread_followed(&self, thread: &ChannelId, followed: bool) -> Result<()> {
        if self.conn.execute("UPDATE channels SET followed=?2 WHERE id=?1 AND parent_channel_id IS NOT NULL", params![thread.0, followed])? == 0 {
            return Err(StoreError::NotFound(format!("thread {}", thread.0)));
        }
        Ok(())
    }

    /// Remove a thread that never received a message, with its session. A thread with messages stays, and this
    /// returns false. The app uses it when the first send of a new thread fails, so no empty thread remains.
    pub fn discard_empty_thread(&self, thread: &ChannelId) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        if self.thread_info(thread)?.is_none() { return Ok(false); }
        let messages: i64 = self.conn.query_row("SELECT COUNT(*) FROM messages WHERE channel_id=?1", params![thread.0], |r| r.get(0))?;
        let turns: i64 = self.conn.query_row("SELECT COUNT(*) FROM turns WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)", params![thread.0], |r| r.get(0))?;
        if messages > 0 || turns > 0 { return Ok(false); }
        for sql in [
            "DELETE FROM events WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
            "DELETE FROM conversation_context WHERE channel_id=?1",
            "DELETE FROM agent_sessions WHERE channel_id=?1",
            "DELETE FROM channel_agents WHERE channel_id=?1",
            "DELETE FROM sessions WHERE channel_id=?1",
            "DELETE FROM channels WHERE id=?1",
        ] {
            self.conn.execute(sql, params![thread.0])?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Remove every row of one channel. A message that other channels reference goes with its references.
    fn purge_channel(&self, channel: &ChannelId) -> Result<()> {
        for sql in CHANNEL_PURGE_SQL {
            self.conn.execute(sql, params![channel.0])?;
        }
        Ok(())
    }

    /// Give a question or sub-agent thread a name that the user chose. Returns the stored name. A name that fails
    /// `normalize_thread_name` changes nothing. Posted references and replay labels read the name from the thread
    /// row, so they show the new name at once.
    pub fn rename_thread(&self, thread: &ChannelId, name: &str) -> Result<String> {
        let name = normalize_thread_name(name).map_err(StoreError::Invalid)?;
        if self.thread_info(thread)?.is_none() {
            return Err(StoreError::NotFound(format!("thread {}", thread.0)));
        }
        self.conn.execute("UPDATE channels SET name=?2 WHERE id=?1 AND parent_channel_id IS NOT NULL", params![thread.0, name])?;
        Ok(name)
    }

    /// Delete a thread with everything it owns, in one transaction. A message that was posted to the parent stays in
    /// the parent as a normal message: same role, author and content, at the position of its reference row, with no
    /// thread link and no replay label. Refused while a turn runs in the thread, while a sub-agent task still runs,
    /// and while an approval waits.
    pub fn delete_thread(&self, thread: &ChannelId) -> Result<ThreadDeletion> {
        let tx = self.conn.unchecked_transaction()?;
        let info = self.thread_info(thread)?.ok_or_else(|| StoreError::NotFound(format!("thread {}", thread.0)))?;
        if info.state == Some(ThreadState::Running) {
            return Err(StoreError::Invalid("A sub-agent task still runs in this thread. Stop it or wait for it before you delete the thread.".into()));
        }
        let turns: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE state IN ('queued','running','awaiting_approval') AND session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
            params![thread.0], |r| r.get(0))?;
        if turns > 0 {
            return Err(StoreError::Invalid("A turn runs in this thread. Stop it before you delete the thread.".into()));
        }
        let approvals: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM approvals WHERE state='pending' AND turn_id IN (SELECT t.id FROM turns t JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
            params![thread.0], |r| r.get(0))?;
        if approvals > 0 {
            return Err(StoreError::Invalid("An approval waits in this thread. Decide it before you delete the thread.".into()));
        }
        let posted: Vec<MessageId> = {
            let mut stmt = self.conn.prepare("SELECT id FROM messages WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?1) ORDER BY ord")?;
            stmt.query_map(params![thread.0], |r| r.get::<_, String>(0).map(MessageId))?.collect::<std::result::Result<_, _>>()?
        };
        // Keep the row, so the message keeps its place. It takes the content and author of its source.
        self.conn.execute(
            "UPDATE messages SET
                role=(SELECT s.role FROM messages s WHERE s.id=messages.source_message_id),
                blocks_json=(SELECT s.blocks_json FROM messages s WHERE s.id=messages.source_message_id),
                author_json=(SELECT s.author_json FROM messages s WHERE s.id=messages.source_message_id),
                source_message_id=NULL
             WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?1)",
            params![thread.0],
        )?;
        self.purge_channel(thread)?;
        tx.commit()?;
        Ok(ThreadDeletion { parent: info.parent_channel_id, posted })
    }

    /// Remove a posted reference row from its parent channel. The message stays in its thread and can be posted
    /// again. Returns where the row pointed. When the parent's replay summary ended at this row, it ends at the
    /// row before it, so the next turn does not replay the whole transcript again.
    pub fn remove_posted_reference(&self, reference: &MessageId) -> Result<PostedSource> {
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<(String, Option<String>)> = self.conn
            .query_row("SELECT channel_id,source_message_id FROM messages WHERE id=?1", params![reference.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (parent, source) = row.ok_or_else(|| StoreError::NotFound(format!("message {}", reference.0)))?;
        if source.is_none() {
            return Err(StoreError::Invalid("Only a message that was posted from a thread can be removed from the channel.".into()));
        }
        let parent = ChannelId(parent);
        let posted = self.posted_sources(&parent)?.remove(reference).ok_or_else(|| StoreError::Corrupt(format!("reference {}", reference.0)))?;
        let mut state = self.conversation_context(&parent)?;
        if state.through.as_ref().is_some_and(|through| through.0.split(":part:").next() == Some(reference.0.as_str())) {
            let history = self.channel_history(&parent)?;
            let replay = crate::conversation_context::replay_with_posted(&history.messages, &history.authors, &history.posted);
            let first = replay.iter().position(|(id, _)| id.0.split(":part:").next() == Some(reference.0.as_str()));
            state.through = first.and_then(|index| index.checked_sub(1)).map(|index| replay[index].0.clone());
            self.set_conversation_context(&parent, &state)?;
        }
        self.conn.execute("DELETE FROM messages WHERE id=?1", params![reference.0])?;
        tx.commit()?;
        Ok(posted)
    }

    /// Rename an active text channel. The name follows `normalize_text_channel_name` and must be unique among the
    /// active general and text channels of the project, like a new channel. `#general` and branch channels cannot
    /// be renamed. A name that equals the current one changes nothing.
    pub fn rename_text_channel(&self, project: &ProjectId, channel: &ChannelId, name: &str) -> Result<Channel> {
        let name = normalize_text_channel_name(name).map_err(StoreError::Invalid)?;
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<(String, bool)> = self.conn
            .query_row(
                "SELECT pc.kind,c.archived FROM project_channels pc JOIN channels c ON c.id=pc.channel_id WHERE pc.channel_id=?1 AND pc.project_id=?2",
                params![channel.0, project.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (kind, archived) = row.ok_or_else(|| StoreError::NotFound(format!("channel {}", channel.0)))?;
        if kind != "text" {
            return Err(StoreError::Invalid("Only a text channel can be renamed here.".into()));
        }
        if archived {
            return Err(StoreError::Invalid("Restore this channel before you rename it.".into()));
        }
        let current = self.load_channel(channel)?;
        if current.name == name {
            return Ok(current);
        }
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_channels pc JOIN channels c ON c.id=pc.channel_id
             WHERE pc.project_id=?1 AND pc.kind IN ('general','text') AND c.archived=0 AND c.id<>?2 AND lower(c.name)=?3)",
            params![project.0, channel.0, name], |r| r.get(0))?;
        if taken || name == "general" {
            return Err(StoreError::Invalid("A channel with this name already exists.".into()));
        }
        tx.execute("UPDATE channels SET name=?2 WHERE id=?1", params![channel.0, name])?;
        tx.commit()?;
        Ok(Channel { name, ..current })
    }

    /// Show a user or agent thread message in the thread's parent channel. The message stays stored once, in the
    /// thread. The parent gets a reference row at the current position. Posting again returns the existing
    /// reference. The flag is true when this call created it.
    pub fn post_thread_message(&self, message: &MessageId) -> Result<(Message, bool)> {
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<(String, String, Option<String>)> = self.conn
            .query_row("SELECT channel_id,role,source_message_id FROM messages WHERE id=?1", params![message.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let (channel, role, source) = row.ok_or_else(|| StoreError::NotFound(format!("message {}", message.0)))?;
        let thread = self.thread_info(&ChannelId(channel))?
            .filter(|_| source.is_none())
            .ok_or_else(|| StoreError::Invalid("Only a message in a thread can be posted to its channel.".into()))?;
        let role = Role::parse(&role).ok_or_else(|| StoreError::Corrupt(format!("role {role}")))?;
        if role == Role::System {
            return Err(StoreError::Invalid("Only user and agent messages can be posted to the channel.".into()));
        }
        let existing: Option<String> = self.conn.query_row("SELECT id FROM messages WHERE source_message_id=?1", params![message.0], |r| r.get(0)).optional()?;
        let created = existing.is_none();
        let reference = existing.map(MessageId).unwrap_or_default();
        if created {
            self.conn.execute(
                "INSERT INTO messages (id, channel_id, role, blocks_json, source_message_id) VALUES (?1, ?2, ?3, '[]', ?4)",
                params![reference.0, thread.parent_channel_id.0, role.as_str(), message.0],
            )?;
        }
        tx.commit()?;
        let posted = self.messages_for_channel(&thread.parent_channel_id)?.into_iter().find(|m| m.id == reference)
            .ok_or_else(|| StoreError::Corrupt(format!("reference {}", reference.0)))?;
        Ok((posted, created))
    }

    /// How many messages of a thread were posted to its parent channel. A delete keeps this many in the channel.
    pub fn posted_message_count(&self, thread: &ChannelId) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE source_message_id IN (SELECT id FROM messages WHERE channel_id=?1)",
            params![thread.0], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// Reference rows of a channel: reference ID to the thread message it shows.
    pub fn posted_sources(&self, channel: &ChannelId) -> Result<std::collections::HashMap<MessageId, PostedSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.source_message_id, t.id, t.name FROM messages m
             JOIN messages s ON s.id=m.source_message_id JOIN channels t ON t.id=s.channel_id WHERE m.channel_id=?1",
        )?;
        let rows = stmt.query_map(params![channel.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        rows.map(|row| {
            let (id, source, thread, name) = row?;
            Ok((MessageId(id), PostedSource { source_message_id: MessageId(source), thread_channel_id: ChannelId(thread), thread_name: name }))
        }).collect()
    }

    /// The messages a response in this channel replays, with their authors and posted-source labels.
    /// A question thread replays its parent channel up to and including the anchor message, then its own messages.
    /// A sub-agent thread replays only its own messages.
    pub fn channel_history(&self, channel: &ChannelId) -> Result<ChannelHistory> {
        let mut history = ChannelHistory { messages: after_reset(self.messages_for_channel(channel)?, self.conversation_context(channel)?.reset_from.as_ref()), authors: self.message_authors(channel)?, posted: self.posted_sources(channel)? };
        let Some(thread) = self.thread_info(channel)? else { return Ok(history) };
        // A sub-agent sees only its own messages: the brief, its replies, and the project context the engine adds.
        let Some(anchor) = thread.anchor_message_id.filter(|_| thread.kind == ThreadKind::Question) else { return Ok(history) };
        let parent = self.messages_for_channel(&thread.parent_channel_id)?;
        let end = parent.iter().position(|m| m.id == anchor)
            .ok_or_else(|| StoreError::Corrupt(format!("thread anchor {}", anchor.0)))?;
        // A `/clear` in the parent before the anchor also cuts what the thread sees of the parent.
        let mut messages = after_reset(parent[..=end].to_vec(), self.conversation_context(&thread.parent_channel_id)?.reset_from.as_ref());
        messages.append(&mut history.messages);
        history.messages = messages;
        history.authors.extend(self.message_authors(&thread.parent_channel_id)?);
        history.posted.extend(self.posted_sources(&thread.parent_channel_id)?);
        Ok(history)
    }

    /// Literal, case-insensitive search of saved direct message text, newest first.
    /// Includes archived and retained chats; never reads project conversations or threads.
    pub fn search_direct_history(&self, agent: &AgentId, query: &str) -> Result<HistorySearch> {
        let needle = query.trim().to_lowercase();
        let mut results = Vec::new();
        if needle.is_empty() {
            return Ok(HistorySearch {
                results,
                has_more: false,
            });
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.channel_id, c.name, COALESCE(s.role, m.role), COALESCE(s.blocks_json, m.blocks_json)
             FROM messages m JOIN channels c ON c.id = m.channel_id LEFT JOIN messages s ON s.id = m.source_message_id
             WHERE c.agent_id = ?1 AND c.project_id IS NULL AND c.parent_channel_id IS NULL ORDER BY m.ord DESC",
        )?;
        let rows = stmt.query_map(params![agent.0], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (message_id, channel_id, channel_name, role, json) = row?;
            let blocks = serde_json::from_str::<Vec<Block>>(&json)?;
            let text = crate::context_menu::message_text(&blocks);
            let folded = text.to_lowercase();
            let Some(index) = folded.find(&needle) else {
                continue;
            };
            if results.len() == 50 {
                return Ok(HistorySearch {
                    results,
                    has_more: true,
                });
            }
            // Map the folded byte offset back to the original text before truncation.
            let mut folded_bytes = 0;
            let start = text
                .chars()
                .take_while(|c| {
                    if folded_bytes >= index {
                        return false;
                    }
                    folded_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
                    true
                })
                .count()
                .saturating_sub(60);
            let mut chars = text.chars().skip(start);
            let excerpt: String = chars.by_ref().take(240).collect();
            let preview = format!(
                "{}{}{}",
                if start > 0 { "…" } else { "" },
                excerpt,
                if chars.next().is_some() { "…" } else { "" }
            );
            results.push(HistoryMatch {
                channel_id,
                channel_name,
                message_id,
                role,
                preview,
            });
        }
        Ok(HistorySearch {
            results,
            has_more: false,
        })
    }

    /// Startup reconciliation. A non-terminal turn whose provider process is not live becomes
    /// `failed`. Its pending approvals become `stale`.
    pub fn reconcile(&self, live_sessions: &HashSet<SessionId>) -> Result<ReconcileReport> {
        let tx = self.conn.unchecked_transaction()?;
        let mut report = ReconcileReport::default();
        let orphans: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, session_id FROM turns WHERE state NOT IN ('completed','failed','cancelled') ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        for (turn_id, session_id) in orphans {
            if live_sessions.contains(&SessionId(session_id)) {
                continue;
            }
            tx.execute(
                "UPDATE turns SET state = 'failed' WHERE id = ?1",
                params![turn_id],
            )?;
            let pending: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT id FROM approvals WHERE turn_id = ?1 AND state = 'pending' ORDER BY id",
                )?;
                let rows = stmt.query_map(params![turn_id], |r| r.get(0))?;
                rows.collect::<std::result::Result<_, _>>()?
            };
            for approval_id in pending {
                tx.execute(
                    "UPDATE approvals SET state = 'stale' WHERE id = ?1",
                    params![approval_id],
                )?;
                report.approvals_staled.push(ApprovalId(approval_id));
            }
            report.turns_failed.push(TurnId(turn_id));
        }
        tx.commit()?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::ApprovalPolicy;
    use crate::event::{EventKind, RawEvent, Sequencer};
    use crate::model::{Block, MessageId, Role, WorkspaceMode};
    use serde_json::json;

    use crate::always_allow::{RuleTarget, RuleRequest};

    #[test]
    fn rules_round_trip_and_deduplicate() {
        let f = fixture();
        let target = RuleTarget::Command { words: vec!["cargo".into(), "test".into()] };
        let rule = f.store.insert_always_allow_rule(&f.agent, &f.project.id, &target).unwrap();
        assert_eq!(rule.target, target);
        assert_eq!(f.store.insert_always_allow_rule(&f.agent, &f.project.id, &target).unwrap(), rule);
        assert_eq!(f.store.always_allow_rules(&f.project.id).unwrap(), vec![rule]);
    }

    #[test]
    fn rules_are_scoped_to_agent_and_project() {
        let f = fixture();
        f.store.insert_always_allow_rule(&f.agent, &f.project.id, &RuleTarget::Host { host: "registry.npmjs.org".into() }).unwrap();
        let request = RuleRequest::Host("REGISTRY.NPMJS.ORG");
        assert!(f.store.matching_always_allow_rule(&f.agent, &f.project.id, &request).unwrap().is_some());
        assert!(f.store.matching_always_allow_rule(&AgentId::new(), &f.project.id, &request).unwrap().is_none());
        assert!(f.store.matching_always_allow_rule(&f.agent, &ProjectId::new(), &request).unwrap().is_none());
    }

    #[test]
    fn deleting_a_rule_stops_matching() {
        let f = fixture();
        let rule = f.store.insert_always_allow_rule(&f.agent, &f.project.id, &RuleTarget::Host { host: "example.com".into() }).unwrap();
        assert!(!f.store.delete_always_allow_rule(&ProjectId::new(), &rule.id).unwrap());
        assert!(f.store.delete_always_allow_rule(&f.project.id, &rule.id).unwrap());
        assert!(f.store.matching_always_allow_rule(&f.agent, &f.project.id, &RuleRequest::Host("example.com")).unwrap().is_none());
    }

    #[test]
    fn deleting_an_agent_or_project_removes_its_rules() {
        for delete_agent in [true, false] {
            let f = fixture();
            f.store.insert_always_allow_rule(&f.agent, &f.project.id, &RuleTarget::Host { host: "example.com".into() }).unwrap();
            if delete_agent {
                f.store.delete_agent(&f.agent, DeleteAgentOptions { delete_conversations: false, delete_private_memory: false }).unwrap();
            } else {
                f.store.hide_project(&f.project.id).unwrap();
            }
            assert!(f.store.always_allow_rules(&f.project.id).unwrap().is_empty());
        }
    }

    #[test]
    fn migration_9_adds_the_rule_table_to_a_version_8_database() {
        let f = fixture();
        f.store.conn.execute_batch("DROP TABLE always_allow_rules; PRAGMA user_version=8;").unwrap();
        f.store.migrate().unwrap();
        assert_eq!(f.store.schema_version().unwrap(), 16);
        assert!(f.store.always_allow_rules(&f.project.id).unwrap().is_empty());
    }

    #[test]
    fn migration_creates_autorun_responses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        {
            let store = Store::open(&path).unwrap();
            store.conn.execute_batch("DROP TABLE autorun_responses; PRAGMA user_version=9;").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        let columns: Vec<String> = store.conn.prepare("PRAGMA table_info(autorun_responses)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(columns, ["response_id", "channel_id", "agent_id", "provider", "root", "created_at"]);
    }

    #[test]
    fn autorun_response_rows_record_the_root_and_go_with_the_channel() {
        let f = fixture();
        let response = Message { id: MessageId::new(), channel_id: f.channel.id.clone(), role: Role::Agent, blocks: vec![] };
        f.store.insert_message(&response).unwrap();
        f.store.record_autorun_response(&response.id, &f.channel.id, &f.agent, "claude", std::path::Path::new("/tmp/demo/wt")).unwrap();
        let row: (String, String, String, String) = f.store.conn.query_row(
            "SELECT channel_id, agent_id, provider, root FROM autorun_responses WHERE response_id=?1",
            params![response.id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!(row, (f.channel.id.0.clone(), f.agent.0.clone(), "claude".to_string(), "/tmp/demo/wt".to_string()));
        f.store.delete_agent(&f.agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
        let rows: i64 = f.store.conn.query_row("SELECT COUNT(*) FROM autorun_responses", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn project_profile_accepts_palette_slots() {
        let f = fixture();
        f.store.update_project_profile(&f.project.id, "Project", Some("lucide:folder:ansi-4")).unwrap();
        assert_eq!(f.store.project_avatar(&f.project.id).unwrap().as_deref(), Some("lucide:folder:ansi-4"));
        for invalid in ["lucide:folder:ansi-0", "lucide:folder:ansi-7"] {
            assert!(matches!(f.store.update_project_profile(&f.project.id, "Project", Some(invalid)), Err(StoreError::Invalid(_))));
        }
    }

    struct Fixture {
        store: Store,
        project: Project,
        agent: AgentId,
        channel: Channel,
        session: SessionId,
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "demo".into(),
            path: "/tmp/demo".into(),
        };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store
            .insert_agent(
                &agent,
                &AgentConfig {
                    workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
                    reasoning_effort: None,
                    name: "Scout".into(),
                    avatar: None,
                    provider: "claude".into(),
                    model: None,
                    system_prompt: String::new(),
                    policy: ApprovalPolicy::Ask,
                    plugin_write: false,
                },
            )
            .unwrap();
        let channel = Channel {
            id: ChannelId::new(),
            project_id: Some(project.id.clone()),
            name: "general".into(),
            agent_id: agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        store.insert_channel(&channel).unwrap();
        let session = SessionId::new();
        store.create_session(&session, &channel.id).unwrap();
        Fixture {
            store,
            project,
            agent,
            channel,
            session,
        }
    }

    #[test]
    fn list_projects_returns_projects_in_insert_order() {
        let f = fixture();
        let p = Project {
            id: ProjectId::new(),
            name: "second".into(),
            path: "/second".into(),
        };
        f.store.insert_project(&p).unwrap();
        assert_eq!(f.store.list_projects().unwrap(), vec![f.project, p]);
    }
    #[test]
    fn all_channels_include_archived_ones_with_the_flag() {
        let f = fixture();
        f.store.archive_channel(&f.channel.id).unwrap();
        let channels = f.store.all_channels_for_project(&f.project.id).unwrap();
        assert_eq!(channels, vec![(f.channel, true)]);
    }
    #[test]
    fn list_agents_returns_agents_in_insert_order() {
        let f = fixture();
        let mut config = f.store.load_agent(&f.agent).unwrap();
        let second = AgentId::new();
        config.plugin_write = true;
        f.store.insert_agent(&second, &config).unwrap();
        let agents = f.store.list_agents().unwrap();
        assert_eq!(
            agents.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            vec![&f.agent, &second]
        );
    }

    #[test]
    fn find_project_agent_matches_provider_and_plugin_write_within_the_project_only() {
        let f = fixture();
        let mut builder = f.store.load_agent(&f.agent).unwrap();
        builder.plugin_write = true;
        let builder_id = AgentId::new();
        f.store.insert_agent(&builder_id, &builder).unwrap();
        f.store
            .insert_channel(&Channel {
                id: ChannelId::new(),
                project_id: Some(f.project.id.clone()),
                name: "b".into(),
                agent_id: builder_id.clone(),
                workspace_mode: WorkspaceMode::MainCheckout,
            })
            .unwrap();
        let found = |provider, plugin_write| {
            f.store
                .find_project_agent(&f.project.id, provider, plugin_write)
                .unwrap()
                .map(|(id, _)| id)
        };
        assert_eq!(found("claude", false), Some(f.agent.clone()));
        assert_eq!(found("claude", true), Some(builder_id));
        assert_eq!(found("unknown", false), None);
        // An agent that no channel of the project uses is never found, and neither is another project's agent.
        assert!(
            f.store
                .find_project_agent(&ProjectId::new(), "claude", false)
                .unwrap()
                .is_none()
        );
        // Archived channels still count.
        f.store.archive_channel(&f.channel.id).unwrap();
        assert_eq!(found("claude", false), Some(f.agent.clone()));
    }

    #[test]
    fn update_agent_replaces_the_config_and_unknown_id_is_not_found() {
        let f = fixture();
        let mut config = f.store.load_agent(&f.agent).unwrap();
        config.model = Some("new-model".into());
        f.store.update_agent(&f.agent, &config).unwrap();
        assert_eq!(f.store.load_agent(&f.agent).unwrap(), config);
        assert!(matches!(
            f.store.update_agent(&AgentId::new(), &config),
            Err(StoreError::NotFound(_))
        ));
    }

    fn stamp(seq: &mut Sequencer, s: &SessionId, text: &str) -> Event {
        seq.stamp(
            s,
            RawEvent {
                kind: EventKind::TextDelta { text: text.into() },
                raw: Some(json!({"v": 1})),
            },
        )
    }

    #[test]
    fn migration_sets_schema_version_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        let v1 = Store::open(&path).unwrap().schema_version().unwrap();
        let v2 = Store::open(&path).unwrap().schema_version().unwrap();
        assert!(v1 >= 2);
        assert_eq!(v1, 16);
        assert_eq!(v1, v2);
    }

    #[test]
    fn lookups_find_the_project_channel_and_session() {
        let f = fixture();
        assert_eq!(
            f.store.project_by_path("/tmp/demo").unwrap(),
            Some(f.project.clone())
        );
        assert_eq!(f.store.project_by_path("/nope").unwrap(), None);
        assert_eq!(
            f.store.first_channel(&f.project.id).unwrap(),
            Some(f.channel.clone())
        );
        assert_eq!(
            f.store.latest_session(&f.channel.id).unwrap(),
            Some(f.session.clone())
        );
        let newer = SessionId::new();
        f.store.create_session(&newer, &f.channel.id).unwrap();
        assert_eq!(f.store.latest_session(&f.channel.id).unwrap(), Some(newer));
        assert_eq!(f.store.latest_session(&ChannelId::new()).unwrap(), None);
    }

    #[test]
    fn a_version_one_database_upgrades_in_place_and_keeps_its_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            // Build a database exactly as milestone M1 left it.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path) VALUES ('p', 'old', '/old')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO agents (id, config_toml) VALUES ('a', 'name=\"A\"\nprovider=\"claude\"\npolicy=\"ask\"')", []).unwrap();
            conn.execute("INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES ('c', 'p', 'general', 'a', 'main_checkout')", []).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        let channels = store.channels_for_project(&ProjectId("p".into())).unwrap();
        assert_eq!(channels.len(), 1);
        assert_eq!(
            channels[0].workspace_mode,
            WorkspaceMode::MainCheckout,
            "old channels stay in the main checkout"
        );
        assert_eq!(store.worktree_for(&ChannelId("c".into())).unwrap(), None);
    }

    #[test]
    fn worktree_records_round_trip_update_and_delete() {
        let f = fixture();
        let rec = WorktreeRecord {
            channel_id: f.channel.id.clone(),
            path: "/w/x".into(),
            branch: "discard/x".into(),
            base_commit: "abc".into(),
            repo: "/r".into(),
            owned: true,
        };
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), None);
        f.store.save_worktree(&rec).unwrap();
        assert_eq!(
            f.store.worktree_for(&f.channel.id).unwrap(),
            Some(rec.clone())
        );
        let moved = WorktreeRecord {
            path: "/w/y".into(),
            ..rec
        };
        f.store.save_worktree(&moved).unwrap();
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), Some(moved));
        f.store.delete_worktree(&f.channel.id).unwrap();
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), None);
    }

    #[test]
    fn the_workspace_mode_can_be_changed_and_the_change_persists() {
        let f = fixture();
        f.store
            .set_workspace_mode(&f.channel.id, WorkspaceMode::Worktree)
            .unwrap();
        assert_eq!(
            f.store.channels_for_project(&f.project.id).unwrap()[0].workspace_mode,
            WorkspaceMode::Worktree
        );
        assert!(matches!(
            f.store
                .set_workspace_mode(&ChannelId::new(), WorkspaceMode::Worktree),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn archived_channels_disappear_from_the_project_but_keep_their_history() {
        let f = fixture();
        let m = Message {
            id: MessageId::new(),
            channel_id: f.channel.id.clone(),
            role: Role::User,
            blocks: vec![Block::Markdown { text: "hi".into() }],
        };
        f.store.insert_message(&m).unwrap();
        f.store.archive_channel(&f.channel.id).unwrap();
        assert!(
            f.store
                .channels_for_project(&f.project.id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(f.store.first_channel(&f.project.id).unwrap(), None);
        assert_eq!(
            f.store.messages_for_channel(&f.channel.id).unwrap().len(),
            1
        );
        assert!(matches!(
            f.store.archive_channel(&ChannelId::new()),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn channel_records_workspace_mode_and_agent() {
        let f = fixture();
        let chans = f.store.channels_for_project(&f.project.id).unwrap();
        assert_eq!(chans, vec![f.channel.clone()]);
        assert_eq!(f.store.load_agent(&f.agent).unwrap().name, "Scout");
    }

    #[test]
    fn events_are_stored_in_sequence_order_with_raw_payload() {
        let f = fixture();
        let mut seq = Sequencer::new();
        let e1 = stamp(&mut seq, &f.session, "a");
        let e2 = stamp(&mut seq, &f.session, "b");
        f.store.append_event(&e2).unwrap();
        f.store.append_event(&e1).unwrap();
        assert_eq!(
            f.store.events_for_session(&f.session).unwrap(),
            vec![e1, e2]
        );
    }

    #[test]
    fn duplicate_sequence_numbers_are_rejected() {
        let f = fixture();
        let mut seq = Sequencer::new();
        let e = stamp(&mut seq, &f.session, "a");
        f.store.append_event(&e).unwrap();
        let mut dup = e.clone();
        dup.id = crate::event::EventId::new();
        assert!(f.store.append_event(&dup).is_err());
    }

    #[test]
    fn last_seq_lets_the_sequencer_resume_after_restart() {
        let f = fixture();
        assert_eq!(f.store.last_seq(&f.session).unwrap(), 0);
        let mut seq = Sequencer::new();
        f.store
            .append_event(&stamp(&mut seq, &f.session, "a"))
            .unwrap();
        f.store
            .append_event(&stamp(&mut seq, &f.session, "b"))
            .unwrap();
        assert_eq!(f.store.last_seq(&f.session).unwrap(), 2);
    }

    #[test]
    fn resume_handle_round_trips_and_can_be_cleared() {
        let f = fixture();
        assert_eq!(f.store.resume_handle(&f.session).unwrap(), None);
        f.store
            .set_resume_handle(&f.session, Some("claude-abc"))
            .unwrap();
        assert_eq!(
            f.store.resume_handle(&f.session).unwrap().as_deref(),
            Some("claude-abc")
        );
        f.store.set_resume_handle(&f.session, None).unwrap();
        assert_eq!(f.store.resume_handle(&f.session).unwrap(), None);
    }

    #[test]
    fn turn_updates_validate_transitions() {
        let f = fixture();
        let t = Turn::new(f.session.clone());
        f.store.insert_turn(&t).unwrap();
        f.store
            .update_turn_state(&t.id, TurnState::Running)
            .unwrap();
        let err = f
            .store
            .update_turn_state(&t.id, TurnState::Queued)
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidTransition { .. }));
        assert_eq!(f.store.turn_state(&t.id).unwrap(), TurnState::Running);
    }

    #[test]
    fn existing_approval_rows_read_as_provider_tool_after_migration() {
        let f = fixture(); let turn = Turn::new(f.session.clone()); f.store.insert_turn(&turn).unwrap();
        let a = Approval::new(turn.id, "Bash", json!({})); f.store.save_approval(&a).unwrap();
        f.store.conn.execute_batch("ALTER TABLE approvals DROP COLUMN kind; DROP TABLE agent_commits; DROP TABLE always_allow_rules;").unwrap();
        assert_eq!(f.store.schema_version().unwrap(), 16);
        f.store.migrate().unwrap();
        assert_eq!(f.store.load_approval(&a.id).unwrap().kind, crate::approval::ApprovalKind::ProviderTool);
        f.store.migrate().unwrap();
        assert_eq!(f.store.schema_version().unwrap(), 16);
    }

    #[test]
    fn approvals_round_trip_with_audit_records() {
        let f = fixture();
        let t = Turn::new(f.session.clone());
        f.store.insert_turn(&t).unwrap();
        let mut a = Approval::new(t.id.clone(), "Bash", json!({"command": "touch x"}));
        f.store.save_approval(&a).unwrap();
        assert_eq!(
            f.store.load_approval(&a.id).unwrap().state,
            ApprovalState::Pending
        );
        a.decide(Decision::Approve, Decider::User, "ask").unwrap();
        f.store.save_approval(&a).unwrap();
        let loaded = f.store.load_approval(&a.id).unwrap();
        assert_eq!(loaded.state, ApprovalState::Decided(Decision::Approve));
        assert_eq!(loaded.audit, a.audit);
        // saving twice must not duplicate audit rows
        f.store.save_approval(&a).unwrap();
        assert_eq!(f.store.load_approval(&a.id).unwrap().audit.len(), 1);
    }

    #[test]
    fn messages_keep_typed_blocks_in_insert_order() {
        let f = fixture();
        let m1 = Message {
            id: MessageId::new(),
            channel_id: f.channel.id.clone(),
            role: Role::User,
            blocks: vec![Block::Markdown { text: "hi".into() }],
        };
        let m2 = Message {
            id: MessageId::new(),
            channel_id: f.channel.id.clone(),
            role: Role::Agent,
            blocks: vec![
                Block::Code {
                    language: Some("rust".into()),
                    text: "fn main(){}".into(),
                },
                Block::Approval {
                    kind: crate::approval::ApprovalKind::ProviderTool,
                    approval_id: "a1".into(),
                    tool: "Bash".into(),
                    summary: "touch x".into(),
                    call_id: Some("c1".into()),
                    state: None,
                    decided_by: None,
                },
            ],
        };
        f.store.insert_message(&m1).unwrap();
        f.store.insert_message(&m2).unwrap();
        assert_eq!(
            f.store.messages_for_channel(&f.channel.id).unwrap(),
            vec![m1, m2]
        );
    }

    #[test]
    fn direct_history_search_is_scoped_literal_and_includes_archived_chats() {
        let f = fixture();
        let direct = Channel {
            id: ChannelId::new(),
            project_id: None,
            name: "Old chat".into(),
            agent_id: f.agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        f.store.insert_channel(&direct).unwrap();
        f.store.archive_channel(&direct.id).unwrap();
        let other = AgentId::new();
        f.store
            .insert_agent(&other, &f.store.load_agent(&f.agent).unwrap())
            .unwrap();
        let other_chat = Channel {
            id: ChannelId::new(),
            agent_id: other,
            ..direct.clone()
        };
        f.store.insert_channel(&other_chat).unwrap();
        for channel in [&direct, &f.channel, &other_chat] {
            f.store
                .insert_message(&Message {
                    id: MessageId::new(),
                    channel_id: channel.id.clone(),
                    role: Role::User,
                    blocks: vec![Block::Markdown {
                        text: "Café 100%_ready".into(),
                    }],
                })
                .unwrap();
        }
        let found = f.store.search_direct_history(&f.agent, " CAFÉ ").unwrap();
        assert_eq!(found.results.len(), 1);
        assert_eq!(found.results[0].channel_id, direct.id.0);
        assert_eq!(found.results[0].preview, "Café 100%_ready");
        assert_eq!(
            f.store
                .search_direct_history(&f.agent, "%_")
                .unwrap()
                .results
                .len(),
            1
        );
        assert!(
            f.store
                .search_direct_history(&f.agent, "' OR 1=1 --")
                .unwrap()
                .results
                .is_empty()
        );
        assert!(
            f.store
                .search_direct_history(&f.agent, "  ")
                .unwrap()
                .results
                .is_empty()
        );
        assert!(
            f.store
                .search_direct_history(&AgentId::new(), "Café")
                .unwrap()
                .results
                .is_empty()
        );
    }

    #[test]
    fn direct_history_search_bounds_results_and_centers_unicode_previews() {
        let f = fixture();
        let chat = Channel {
            id: ChannelId::new(),
            project_id: None,
            name: "Chat".into(),
            agent_id: f.agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        f.store.insert_channel(&chat).unwrap();
        let text = format!("{}İCAFÉ{}", "🙂".repeat(300), "é".repeat(300));
        let mut last = MessageId::new();
        for _ in 0..51 {
            last = MessageId::new();
            f.store
                .insert_message(&Message {
                    id: last.clone(),
                    channel_id: chat.id.clone(),
                    role: Role::Agent,
                    blocks: vec![Block::Code {
                        language: None,
                        text: text.clone(),
                    }],
                })
                .unwrap();
        }
        let found = f.store.search_direct_history(&f.agent, "café").unwrap();
        assert!(found.has_more);
        assert_eq!(found.results.len(), 50);
        assert_eq!(found.results[0].message_id, last.0);
        assert!(found.results[0].preview.contains("İCAFÉ"));
        assert!(found.results[0].preview.chars().count() <= 242);
    }

    #[test]
    fn dm_list_is_explicit_idempotent_and_persists_after_reopening() {
        let path = std::env::temp_dir().join(format!("discard-dm-list-{}.db", AgentId::new().0));
        let agent = AgentId::new();
        {
            let store = Store::open(&path).unwrap();
            store
                .insert_agent(
                    &agent,
                    &AgentConfig {
                        workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
                        reasoning_effort: None,
                        name: "Friend".into(),
                        avatar: None,
                        provider: "codex".into(),
                        model: Some("exact".into()),
                        system_prompt: String::new(),
                        policy: ApprovalPolicy::Ask,
                        plugin_write: false,
                    },
                )
                .unwrap();
            store.ensure_default_conversation(&agent).unwrap();
            assert!(store.direct_message_agents().unwrap().is_empty());
            store.add_direct_message(&agent).unwrap();
            store.add_direct_message(&agent).unwrap();
            assert_eq!(store.direct_message_agents().unwrap(), vec![agent.clone()]);
        }
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.direct_message_agents().unwrap(), vec![agent]);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reconcile_fails_orphaned_turns_and_stales_their_approvals() {
        let f = fixture();
        let orphan = Turn::new(f.session.clone());
        f.store.insert_turn(&orphan).unwrap();
        f.store
            .update_turn_state(&orphan.id, TurnState::Running)
            .unwrap();
        f.store
            .update_turn_state(&orphan.id, TurnState::AwaitingApproval)
            .unwrap();
        let pending = Approval::new(orphan.id.clone(), "Bash", json!({}));
        f.store.save_approval(&pending).unwrap();

        let mut decided = Approval::new(orphan.id.clone(), "Read", json!({}));
        decided
            .decide(Decision::Approve, Decider::User, "ask")
            .unwrap();
        f.store.save_approval(&decided).unwrap();

        let done = Turn::new(f.session.clone());
        f.store.insert_turn(&done).unwrap();
        f.store
            .update_turn_state(&done.id, TurnState::Running)
            .unwrap();
        f.store
            .update_turn_state(&done.id, TurnState::Completed)
            .unwrap();

        let report = f.store.reconcile(&HashSet::new()).unwrap();
        assert_eq!(report.turns_failed, vec![orphan.id.clone()]);
        assert_eq!(report.approvals_staled, vec![pending.id.clone()]);
        assert_eq!(f.store.turn_state(&orphan.id).unwrap(), TurnState::Failed);
        assert_eq!(f.store.turn_state(&done.id).unwrap(), TurnState::Completed);
        assert_eq!(
            f.store.load_approval(&pending.id).unwrap().state,
            ApprovalState::Stale
        );
        assert_eq!(
            f.store.load_approval(&decided.id).unwrap().state,
            ApprovalState::Decided(Decision::Approve)
        );
    }

    #[test]
    fn reconcile_leaves_turns_of_live_sessions_alone() {
        let f = fixture();
        let t = Turn::new(f.session.clone());
        f.store.insert_turn(&t).unwrap();
        f.store
            .update_turn_state(&t.id, TurnState::Running)
            .unwrap();
        let live: HashSet<SessionId> = [f.session.clone()].into();
        assert_eq!(
            f.store.reconcile(&live).unwrap(),
            ReconcileReport::default()
        );
        assert_eq!(f.store.turn_state(&t.id).unwrap(), TurnState::Running);
    }

    #[test]
    fn data_survives_reopening_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        let session = SessionId::new();
        {
            let store = Store::open(&path).unwrap();
            let project = Project {
                id: ProjectId::new(),
                name: "p".into(),
                path: "/p".into(),
            };
            store.insert_project(&project).unwrap();
            let agent = AgentId::new();
            store
                .insert_agent(
                    &agent,
                    &AgentConfig {
                        workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
                        reasoning_effort: None,
                        name: "A".into(),
                        avatar: None,
                        provider: "claude".into(),
                        model: None,
                        system_prompt: String::new(),
                        policy: ApprovalPolicy::DenyAll,
                        plugin_write: false,
                    },
                )
                .unwrap();
            let ch = Channel {
                id: ChannelId::new(),
                project_id: Some(project.id),
                name: "c".into(),
                agent_id: agent,
                workspace_mode: WorkspaceMode::Worktree,
            };
            store.insert_channel(&ch).unwrap();
            store.create_session(&session, &ch.id).unwrap();
            let mut seq = Sequencer::new();
            store.append_event(&stamp(&mut seq, &session, "x")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.last_seq(&session).unwrap(), 1);
    }

    fn say(store: &Store, channel: &ChannelId, role: Role, text: &str, author: Option<&AgentId>) -> MessageId {
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role, blocks: vec![Block::Markdown { text: text.into() }] };
        match author {
            Some(agent) => store.insert_authored_message(&message, &MessageAuthor { agent_id: agent.clone(), name: "Scout".into(), provider: "claude".into(), avatar: None }).unwrap(),
            None => store.insert_message(&message).unwrap(),
        }
        message.id
    }

    fn texts(messages: &[Message]) -> Vec<String> {
        messages.iter().map(|m| match &m.blocks[..] { [Block::Markdown { text }] => text.clone(), other => format!("{other:?}") }).collect()
    }

    fn invalid(result: Result<Channel>) -> String {
        match result { Err(StoreError::Invalid(message)) => message, other => panic!("expected an Invalid error, got {other:?}") }
    }

    fn columns(store: &Store, table: &str) -> Vec<(String, String, bool, Option<String>)> {
        store.conn.prepare(&format!("PRAGMA table_info({table})")).unwrap()
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn migration_11_adds_thread_columns_to_a_version_10_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v10.db");
        let (fresh_channels, fresh_messages) = {
            let f = fixture();
            (columns(&f.store, "channels"), columns(&f.store, "messages"))
        };
        let (channel, message) = {
            let f = fixture();
            let store = Store::open(&path).unwrap();
            store.insert_project(&f.project).unwrap();
            store.insert_agent(&f.agent, &store_agent_config()).unwrap();
            store.insert_channel(&f.channel).unwrap();
            let message = say(&store, &f.channel.id, Role::User, "old", None);
            store.conn.execute_batch("DROP INDEX one_thread_per_anchor; DROP INDEX one_reference_per_source; DROP INDEX threads_by_parent;
                ALTER TABLE channels DROP COLUMN muted; ALTER TABLE channels DROP COLUMN parent_channel_id; ALTER TABLE channels DROP COLUMN anchor_message_id;
                ALTER TABLE channels DROP COLUMN thread_kind; ALTER TABLE channels DROP COLUMN followed; ALTER TABLE channels DROP COLUMN thread_state; ALTER TABLE channels DROP COLUMN origin_message_id;
                ALTER TABLE messages DROP COLUMN source_message_id; PRAGMA user_version=10;").unwrap();
            assert!(!columns(&store, "channels").iter().any(|c| c.0 == "followed"));
            (f.channel, message)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        assert_eq!(columns(&store, "channels"), fresh_channels, "an upgraded database matches a fresh one");
        assert_eq!(columns(&store, "messages"), fresh_messages);
        assert_eq!(texts(&store.messages_for_channel(&channel.id).unwrap()), ["old"]);
        assert_eq!(store.thread_info(&channel.id).unwrap(), None);
        let thread = store.create_thread(&channel.id, &message, "old", &channel.agent_id).unwrap();
        assert!(store.thread_info(&thread.id).unwrap().unwrap().followed);
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
    }

    #[test]
    fn migration_12_adds_sub_agent_state_and_limits_the_anchor_index_to_question_threads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v11.db");
        let fresh = columns(&fixture().store, "channels");
        let (channel, answer) = {
            let f = fixture();
            let store = Store::open(&path).unwrap();
            store.insert_project(&f.project).unwrap();
            store.insert_agent(&f.agent, &store_agent_config()).unwrap();
            store.insert_channel(&f.channel).unwrap();
            let answer = say(&store, &f.channel.id, Role::Agent, "answer", Some(&f.agent));
            store.create_thread(&f.channel.id, &answer, "question", &f.agent).unwrap();
            store.conn.execute_batch("DROP INDEX one_thread_per_anchor; ALTER TABLE channels DROP COLUMN muted; ALTER TABLE channels DROP COLUMN thread_state; ALTER TABLE channels DROP COLUMN origin_message_id;
                CREATE UNIQUE INDEX one_thread_per_anchor ON channels(anchor_message_id) WHERE anchor_message_id IS NOT NULL; PRAGMA user_version=11;").unwrap();
            (f.channel, answer)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        assert_eq!(columns(&store, "channels"), fresh, "an upgraded database matches a fresh one");
        let question = store.thread_for_anchor(&answer).unwrap().unwrap();
        assert_eq!((question.kind, question.state, question.followed), (ThreadKind::Question, None, true), "old threads are question threads");
        say(&store, &channel.id, Role::User, "go", None);
        let sub = store.create_sub_agent_thread(&channel.id, "research", &channel.agent_id).unwrap();
        store.conn.execute("UPDATE channels SET anchor_message_id=?2 WHERE id=?1", params![sub.id.0, answer.0]).unwrap();
        assert_eq!(store.threads_at_anchor(&answer).unwrap().len(), 2, "a sub-agent thread may share the anchor of a question thread");
    }

    fn store_agent_config() -> AgentConfig {
        AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Scout".into(), avatar: None, provider: "claude".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }
    }

    #[test]
    fn a_thread_inherits_the_project_starts_followed_and_has_a_session() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::Agent, "answer", Some(&f.agent));
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        assert_eq!(thread.project_id, f.channel.project_id);
        assert_eq!(thread.agent_id, f.agent);
        assert_eq!(thread.name, "Why?");
        let info = f.store.thread_info(&thread.id).unwrap().unwrap();
        assert_eq!(info, ThreadInfo { channel_id: thread.id.clone(), parent_channel_id: f.channel.id.clone(), anchor_message_id: Some(anchor.clone()), kind: ThreadKind::Question, followed: true, name: "Why?".into(), state: None, agent_id: f.agent.clone(), agent_name: "Scout".into() });
        assert_eq!(f.store.thread_for_anchor(&anchor).unwrap(), Some(info.clone()));
        assert_eq!(f.store.threads_for_channel(&f.channel.id).unwrap(), vec![info]);
        assert!(f.store.latest_session(&thread.id).unwrap().is_some());
        assert_eq!(f.store.thread_info(&f.channel.id).unwrap(), None, "a normal channel has no thread info");
        f.store.set_thread_followed(&thread.id, false).unwrap();
        assert!(!f.store.thread_info(&thread.id).unwrap().unwrap().followed);
        assert!(matches!(f.store.set_thread_followed(&f.channel.id, true), Err(StoreError::NotFound(_))), "only a thread can be followed");
    }

    #[test]
    fn thread_creation_rules_reject_bad_anchors_and_nested_threads() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let other = f.store.create_conversation(&f.agent, f.channel.project_id.as_ref(), "other").unwrap();
        let elsewhere = say(&f.store, &other.id, Role::User, "elsewhere", None);
        let marker = say(&f.store, &f.channel.id, Role::System, "marker", None);
        assert!(invalid(f.store.create_thread(&f.channel.id, &MessageId::new(), "x", &f.agent)).contains("saved message"), "unknown anchor");
        assert!(invalid(f.store.create_thread(&f.channel.id, &elsewhere, "x", &f.agent)).contains("saved message"), "anchor in another channel");
        assert!(invalid(f.store.create_thread(&f.channel.id, &marker, "x", &f.agent)).contains("user or agent"));
        assert!(invalid(f.store.create_thread(&f.channel.id, &anchor, "  ", &f.agent)).contains("name"));
        let thread = f.store.create_thread(&f.channel.id, &anchor, "first", &f.agent).unwrap();
        assert!(invalid(f.store.create_thread(&f.channel.id, &anchor, "second", &f.agent)).contains("already has a thread"));
        let inside = say(&f.store, &thread.id, Role::User, "inside", None);
        assert!(invalid(f.store.create_thread(&thread.id, &inside, "nested", &f.agent)).contains("cannot start another thread"));
        let (reference, _) = f.store.post_thread_message(&inside).unwrap();
        assert!(invalid(f.store.create_thread(&f.channel.id, &reference.id, "from a post", &f.agent)).contains("posted from a thread"));
        f.store.set_agent_archived(&f.agent, true).unwrap();
        let later = say(&f.store, &f.channel.id, Role::User, "later", None);
        assert!(invalid(f.store.create_thread(&f.channel.id, &later, "archived agent", &f.agent)).contains("Restore the agent"));
        assert_eq!(f.store.threads_for_channel(&f.channel.id).unwrap().len(), 1);
    }

    #[test]
    fn only_a_thread_without_messages_can_be_discarded() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "first", &f.agent).unwrap();
        assert!(!f.store.discard_empty_thread(&f.channel.id).unwrap(), "a normal channel is never discarded");
        assert!(f.store.load_channel(&f.channel.id).is_ok());
        say(&f.store, &thread.id, Role::User, "inside", None);
        assert!(!f.store.discard_empty_thread(&thread.id).unwrap(), "a thread with a message stays");
        assert!(f.store.thread_info(&thread.id).unwrap().is_some());
        let empty = say(&f.store, &f.channel.id, Role::Agent, "second", Some(&f.agent));
        let second = f.store.create_thread(&f.channel.id, &empty, "second", &f.agent).unwrap();
        assert!(f.store.discard_empty_thread(&second.id).unwrap());
        assert!(f.store.thread_info(&second.id).unwrap().is_none());
        assert!(f.store.load_channel(&second.id).is_err());
        assert!(f.store.thread_for_anchor(&empty).unwrap().is_none(), "the anchor can start a new thread");
        f.store.create_thread(&f.channel.id, &empty, "again", &f.agent).unwrap();
    }

    #[test]
    fn one_thread_per_anchor_is_enforced_by_the_database() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        f.store.create_thread(&f.channel.id, &anchor, "first", &f.agent).unwrap();
        let second = Channel { id: ChannelId::new(), project_id: None, name: "dup".into(), agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        f.store.insert_channel(&second).unwrap();
        assert!(f.store.conn.execute("UPDATE channels SET parent_channel_id=?2,anchor_message_id=?3,thread_kind='question' WHERE id=?1", params![second.id.0, f.channel.id.0, anchor.0]).is_err());
    }

    fn sub_thread(f: &Fixture, name: &str) -> Channel { f.store.create_sub_agent_thread(&f.channel.id, name, &f.agent).unwrap() }

    #[test]
    fn a_sub_agent_thread_starts_running_unanchored_and_not_followed() {
        let f = fixture();
        assert!(invalid(f.store.create_sub_agent_thread(&f.channel.id, "x", &f.agent)).contains("user message"), "the parent needs a user message");
        let user = say(&f.store, &f.channel.id, Role::User, "research this", None);
        let thread = sub_thread(&f, "Find callers");
        assert_eq!((thread.project_id.clone(), thread.agent_id.clone(), thread.name.as_str()), (f.channel.project_id.clone(), f.agent.clone(), "Find callers"));
        let info = f.store.thread_info(&thread.id).unwrap().unwrap();
        assert_eq!(info, ThreadInfo { channel_id: thread.id.clone(), parent_channel_id: f.channel.id.clone(), anchor_message_id: None, kind: ThreadKind::SubAgent, followed: false, name: "Find callers".into(), state: Some(ThreadState::Running), agent_id: f.agent.clone(), agent_name: "Scout".into() });
        assert!(f.store.latest_session(&thread.id).unwrap().is_some());
        assert_eq!(f.store.threads_for_channel(&f.channel.id).unwrap(), vec![info]);
        assert!(f.store.thread_for_anchor(&user).unwrap().is_none(), "no anchor yet");
        assert!(invalid(f.store.create_sub_agent_thread(&thread.id, "nested", &f.agent)).contains("cannot start another thread"));
        assert!(invalid(f.store.create_sub_agent_thread(&f.channel.id, "  ", &f.agent)).contains("name"));
        f.store.set_agent_archived(&f.agent, true).unwrap();
        assert!(invalid(f.store.create_sub_agent_thread(&f.channel.id, "late", &f.agent)).contains("Restore the agent"));
    }

    #[test]
    fn the_parent_response_anchors_the_threads_of_its_turn_in_creation_order() {
        let f = fixture();
        let user = say(&f.store, &f.channel.id, Role::User, "go", None);
        let (a, b) = (sub_thread(&f, "a"), sub_thread(&f, "b"));
        assert!(f.store.anchor_sub_agent_threads(&f.channel.id, None).unwrap().len() == 2, "no response: both fall back to the user message");
        let response = say(&f.store, &f.channel.id, Role::Agent, "done", Some(&f.agent));
        assert!(f.store.anchor_sub_agent_threads(&f.channel.id, Some(&response)).unwrap().is_empty(), "an anchored thread never moves");
        for thread in [&a, &b] { assert_eq!(f.store.thread_info(&thread.id).unwrap().unwrap().anchor_message_id, Some(user.clone())); }

        let user2 = say(&f.store, &f.channel.id, Role::User, "again", None);
        let (c, d) = (sub_thread(&f, "c"), sub_thread(&f, "d"));
        let response2 = say(&f.store, &f.channel.id, Role::Agent, "done again", Some(&f.agent));
        let changed = f.store.anchor_sub_agent_threads(&f.channel.id, Some(&response2)).unwrap();
        assert_eq!(changed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["c", "d"]);
        for thread in [&c, &d] { assert_eq!(f.store.thread_info(&thread.id).unwrap().unwrap().anchor_message_id, Some(response2.clone())); }
        let on_response = f.store.threads_at_anchor(&response2).unwrap();
        assert_eq!(on_response.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["c", "d"], "several threads on one anchor, in creation order");
        assert_ne!(user, user2);
    }

    #[test]
    fn a_stale_unanchored_thread_falls_back_to_its_own_user_message_not_the_new_response() {
        let f = fixture();
        let first = say(&f.store, &f.channel.id, Role::User, "one", None);
        let stale = sub_thread(&f, "stale");
        say(&f.store, &f.channel.id, Role::User, "two", None);
        let fresh = sub_thread(&f, "fresh");
        let response = say(&f.store, &f.channel.id, Role::Agent, "ok", Some(&f.agent));
        f.store.anchor_sub_agent_threads(&f.channel.id, Some(&response)).unwrap();
        assert_eq!(f.store.thread_info(&stale.id).unwrap().unwrap().anchor_message_id, Some(first));
        assert_eq!(f.store.thread_info(&fresh.id).unwrap().unwrap().anchor_message_id, Some(response));
    }

    #[test]
    fn a_question_thread_and_sub_agent_threads_share_one_anchor() {
        let f = fixture();
        say(&f.store, &f.channel.id, Role::User, "go", None);
        let sub = sub_thread(&f, "sub");
        let response = say(&f.store, &f.channel.id, Role::Agent, "done", Some(&f.agent));
        f.store.anchor_sub_agent_threads(&f.channel.id, Some(&response)).unwrap();
        let question = f.store.create_thread(&f.channel.id, &response, "why", &f.agent).unwrap();
        assert_eq!(f.store.thread_for_anchor(&response).unwrap().unwrap().channel_id, question.id, "the question lookup skips sub-agent threads");
        assert_eq!(f.store.threads_at_anchor(&response).unwrap().iter().map(|t| t.channel_id.clone()).collect::<Vec<_>>(), vec![sub.id.clone(), question.id.clone()]);
        assert!(invalid(f.store.create_thread(&f.channel.id, &response, "again", &f.agent)).contains("already has a thread"), "one question thread for each message");
        assert!(f.store.conn.execute("UPDATE channels SET anchor_message_id=?2 WHERE id=?1", params![sub_thread(&f, "x").id.0, response.0]).is_ok(), "the index lets sub-agent threads share an anchor");
    }

    #[test]
    fn the_first_final_state_of_a_sub_agent_thread_wins() {
        let f = fixture();
        say(&f.store, &f.channel.id, Role::User, "go", None);
        let thread = sub_thread(&f, "t");
        assert!(f.store.finish_sub_agent_thread(&thread.id, ThreadState::TimedOut).unwrap());
        assert!(!f.store.finish_sub_agent_thread(&thread.id, ThreadState::Cancelled).unwrap(), "a later state does not replace the first");
        assert_eq!(f.store.thread_info(&thread.id).unwrap().unwrap().state, Some(ThreadState::TimedOut));
        assert!(matches!(f.store.finish_sub_agent_thread(&thread.id, ThreadState::Running), Err(StoreError::Invalid(_))));
        let answer = say(&f.store, &f.channel.id, Role::Agent, "a", Some(&f.agent));
        let question = f.store.create_thread(&f.channel.id, &answer, "q", &f.agent).unwrap();
        assert!(matches!(f.store.finish_sub_agent_thread(&question.id, ThreadState::Done), Err(StoreError::NotFound(_))), "a question thread has no state");
        assert!(matches!(f.store.finish_sub_agent_thread(&f.channel.id, ThreadState::Done), Err(StoreError::NotFound(_))));
        for state in [ThreadState::Running, ThreadState::Done, ThreadState::Failed, ThreadState::Cancelled, ThreadState::TimedOut] {
            assert_eq!(ThreadState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ThreadState::parse("other"), None);
    }

    #[test]
    fn opening_the_database_cancels_sub_agent_runs_that_were_still_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.db");
        let f = fixture();
        let (running, done) = {
            let store = Store::open(&path).unwrap();
            store.insert_project(&f.project).unwrap();
            store.insert_agent(&f.agent, &store_agent_config()).unwrap();
            store.insert_channel(&f.channel).unwrap();
            say(&store, &f.channel.id, Role::User, "go", None);
            let running = store.create_sub_agent_thread(&f.channel.id, "running", &f.agent).unwrap();
            let done = store.create_sub_agent_thread(&f.channel.id, "done", &f.agent).unwrap();
            store.finish_sub_agent_thread(&done.id, ThreadState::Done).unwrap();
            (running, done)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.thread_info(&running.id).unwrap().unwrap().state, Some(ThreadState::Cancelled));
        assert_eq!(store.thread_info(&done.id).unwrap().unwrap().state, Some(ThreadState::Done));
        assert_eq!(store.cancel_running_sub_agents().unwrap(), 0);
    }

    #[test]
    fn a_sub_agent_thread_replays_only_its_own_messages_and_a_question_thread_keeps_the_parent_prefix() {
        let f = fixture();
        say(&f.store, &f.channel.id, Role::User, "parent one", None);
        let sub = sub_thread(&f, "sub");
        say(&f.store, &sub.id, Role::User, "the brief", Some(&f.agent));
        say(&f.store, &sub.id, Role::Agent, "the result", Some(&f.agent));
        let response = say(&f.store, &f.channel.id, Role::Agent, "parent two", Some(&f.agent));
        f.store.anchor_sub_agent_threads(&f.channel.id, Some(&response)).unwrap();
        assert_eq!(texts(&f.store.channel_history(&sub.id).unwrap().messages), ["the brief", "the result"], "no parent prefix, before or after anchoring");
        let question = f.store.create_thread(&f.channel.id, &response, "q", &f.agent).unwrap();
        say(&f.store, &question.id, Role::User, "side", None);
        assert_eq!(texts(&f.store.channel_history(&question.id).unwrap().messages), ["parent one", "parent two", "side"]);
        assert_eq!(f.store.last_agent_message(&sub.id).unwrap().map(|m| texts(&[m])), Some(vec!["the result".to_string()]));
        assert!(f.store.last_agent_message(&question.id).unwrap().is_none());
    }

    #[test]
    fn a_sub_agent_thread_name_uses_the_title_or_the_start_of_the_brief() {
        assert_eq!(sub_agent_thread_name("  Find   callers ", "ignored"), "Find callers");
        assert_eq!(sub_agent_thread_name("", "Search the repo\nfor every use of the cache"), "Search the repo for every use of the cac", "cut at 40 characters");
        assert_eq!(sub_agent_thread_name("   ", &"é".repeat(80)), "é".repeat(THREAD_NAME_CHARS));
        assert_eq!(sub_agent_thread_name(&"t".repeat(60), "brief"), "t".repeat(THREAD_NAME_CHARS));
        assert_eq!(sub_agent_thread_name("", " "), "Thread");
    }

    #[test]
    fn thread_names_cut_on_a_character_boundary() {
        assert_eq!(thread_name("  Why does\n the build\tfail? "), "Why does the build fail?");
        assert_eq!(thread_name(&"é".repeat(60)), "é".repeat(THREAD_NAME_CHARS));
        assert_eq!(thread_name(&format!("{} tail", "a".repeat(39))), "a".repeat(39), "no trailing space is kept");
        assert_eq!(thread_name(&"日本語".repeat(20)).chars().count(), THREAD_NAME_CHARS);
        assert_eq!(thread_name("short"), "short");
        assert_eq!(thread_name(" \n "), "Thread");
    }

    #[test]
    fn threads_are_not_listed_as_channels_or_conversations() {
        let f = fixture();
        let project = f.channel.project_id.clone().unwrap();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "general", &f.agent).unwrap();
        let direct = f.store.create_conversation(&f.agent, None, "chat").unwrap();
        let direct_anchor = say(&f.store, &direct.id, Role::User, "dm question", None);
        let direct_thread = f.store.create_thread(&direct.id, &direct_anchor, "dm thread", &f.agent).unwrap();
        let ids = |channels: Vec<Channel>| channels.into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(f.store.channels_for_project(&project).unwrap()), vec![f.channel.id.clone()]);
        assert_eq!(f.store.all_channels_for_project(&project).unwrap().into_iter().map(|(c, _)| c.id).collect::<Vec<_>>(), vec![f.channel.id.clone()]);
        assert_eq!(f.store.first_channel(&project).unwrap().unwrap().id, f.channel.id);
        assert_eq!(ids(f.store.all_conversations().unwrap()), vec![f.channel.id.clone(), direct.id.clone()]);
        assert_eq!(ids(f.store.conversations_for_agent(&f.agent).unwrap()), vec![f.channel.id.clone(), direct.id.clone()]);
        assert!(f.store.select_project_channel(&project, &thread.id).is_err(), "a thread cannot be the project's selected channel");
        f.store.conn.execute("INSERT INTO project_last_channels(project_id,channel_id) VALUES(?1,?2)", params![project.0, thread.id.0]).unwrap();
        assert_eq!(f.store.last_project_channel(&project).unwrap().map(|c| c.id), None);
        assert!(f.store.set_canonical_project_channel(&project, &thread.id).is_err(), "a thread named general is not a general channel");
        f.store.select_project_channel(&project, &f.channel.id).unwrap();
        assert_eq!(f.store.last_project_channel(&project).unwrap().unwrap().id, f.channel.id);
        // A direct thread stays out of the default conversation and the history search.
        say(&f.store, &direct_thread.id, Role::User, "needle in a thread", None);
        say(&f.store, &direct.id, Role::User, "needle in the chat", None);
        let found = f.store.search_direct_history(&f.agent, "needle").unwrap().results;
        assert_eq!(found.iter().map(|m| m.channel_id.clone()).collect::<Vec<_>>(), vec![direct.id.0.clone()]);
        let default = f.store.ensure_default_conversation(&f.agent).unwrap();
        assert!(f.store.thread_info(&default.id).unwrap().is_none() && f.store.default_conversation(&f.agent).unwrap().id == default.id);
    }

    #[test]
    fn a_thread_named_general_is_never_adopted_as_the_general_channel() {
        let f = fixture();
        let project = Project { id: ProjectId::new(), name: "second".into(), path: "/second".into() };
        f.store.insert_project(&project).unwrap();
        let parent = f.store.create_conversation(&f.agent, Some(&project.id), "branch-work").unwrap();
        let anchor = say(&f.store, &parent.id, Role::User, "question", None);
        let thread = f.store.create_thread(&parent.id, &anchor, "general", &f.agent).unwrap();
        let general = f.store.ensure_project_general_channel(&project.id, &f.agent).unwrap();
        assert_ne!(general.id, thread.id);
        assert_ne!(general.id, parent.id);
    }

    #[test]
    fn posting_stores_the_message_once_and_references_it_in_the_parent() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        let reply = say(&f.store, &thread.id, Role::Agent, "because", Some(&f.agent));
        say(&f.store, &thread.id, Role::User, "not posted", None);
        let later = say(&f.store, &f.channel.id, Role::User, "later parent message", None);
        let (reference, created) = f.store.post_thread_message(&reply).unwrap();
        assert!(created);
        assert_eq!((reference.channel_id.clone(), reference.role), (f.channel.id.clone(), Role::Agent));
        assert_ne!(reference.id, reply);
        let (again, created) = f.store.post_thread_message(&reply).unwrap();
        assert!(!created);
        assert_eq!(again, reference, "posting twice returns the existing reference");
        let count = |channel: &ChannelId| -> i64 { f.store.conn.query_row("SELECT COUNT(*) FROM messages WHERE channel_id=?1", params![channel.0], |r| r.get(0)).unwrap() };
        assert_eq!((count(&f.channel.id), count(&thread.id)), (3, 2), "the thread keeps its own rows and the parent gets one reference");
        let parent = f.store.messages_for_channel(&f.channel.id).unwrap();
        assert_eq!(parent.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![anchor.clone(), later, reference.id.clone()], "the reference sits at the time of posting");
        assert_eq!(parent[2].role, Role::Agent);
        assert_eq!(texts(&parent[2..]), ["because"]);
        assert_eq!(f.store.message_authors(&f.channel.id).unwrap().get(&reference.id).map(|a| a.agent_id.clone()), Some(f.agent.clone()), "the reference shows the source's author");
        let sources = f.store.posted_sources(&f.channel.id).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[&reference.id], PostedSource { source_message_id: reply.clone(), thread_channel_id: thread.id.clone(), thread_name: "Why?".into() });
        assert!(f.store.posted_sources(&thread.id).unwrap().is_empty());
        assert_eq!(f.store.messages_for_channel(&thread.id).unwrap().len(), 2);
        assert!(matches!(f.store.post_thread_message(&anchor), Err(StoreError::Invalid(_))), "a parent message is not a thread message");
        assert!(matches!(f.store.post_thread_message(&reference.id), Err(StoreError::Invalid(_))), "a reference cannot be posted again");
        assert!(matches!(f.store.post_thread_message(&MessageId::new()), Err(StoreError::NotFound(_))));
        let marker = say(&f.store, &thread.id, Role::System, "model changed", None);
        assert!(matches!(f.store.post_thread_message(&marker), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn a_thread_replays_its_parent_up_to_the_anchor_then_its_own_messages() {
        let f = fixture();
        let first = say(&f.store, &f.channel.id, Role::User, "one", None);
        let anchor = say(&f.store, &f.channel.id, Role::Agent, "two", Some(&f.agent));
        say(&f.store, &f.channel.id, Role::User, "three", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "t", &f.agent).unwrap();
        say(&f.store, &thread.id, Role::User, "side question", None);
        let side = say(&f.store, &thread.id, Role::Agent, "side answer", Some(&f.agent));
        say(&f.store, &f.channel.id, Role::User, "four", None);
        let history = f.store.channel_history(&thread.id).unwrap();
        assert_eq!(texts(&history.messages), ["one", "two", "side question", "side answer"]);
        assert!(history.authors.contains_key(&anchor) && history.authors.contains_key(&side) && !history.authors.contains_key(&first));
        assert!(history.posted.is_empty());
        let parent = f.store.channel_history(&f.channel.id).unwrap();
        assert_eq!(texts(&parent.messages), ["one", "two", "three", "four"], "thread messages that were not posted never reach the parent");
        let (reference, _) = f.store.post_thread_message(&side).unwrap();
        let parent = f.store.channel_history(&f.channel.id).unwrap();
        assert_eq!(texts(&parent.messages), ["one", "two", "three", "four", "side answer"]);
        assert_eq!(parent.posted[&reference.id].thread_name, "t");
        let thread_history = f.store.channel_history(&thread.id).unwrap();
        assert_eq!(texts(&thread_history.messages), ["one", "two", "side question", "side answer"], "later parent messages and posts stay out of the thread");
    }

    #[test]
    fn deleting_an_agents_conversations_removes_its_threads_and_their_references() {
        let f = fixture();
        let other = AgentId::new();
        f.store.insert_agent(&other, &store_agent_config()).unwrap();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "t", &other).unwrap();
        let reply = say(&f.store, &thread.id, Role::Agent, "answer", Some(&other));
        let (reference, _) = f.store.post_thread_message(&reply).unwrap();
        f.store.delete_agent(&other, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
        assert!(f.store.thread_info(&thread.id).unwrap().is_none());
        assert!(f.store.load_channel(&thread.id).is_err());
        assert!(f.store.messages_for_channel(&f.channel.id).unwrap().iter().all(|m| m.id != reference.id), "the reference goes with its source");
        assert_eq!(texts(&f.store.messages_for_channel(&f.channel.id).unwrap()), ["question"]);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "again", &f.agent).unwrap();
        say(&f.store, &thread.id, Role::User, "hi", None);
        f.store.delete_agent(&f.agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
        assert!(f.store.all_conversations().unwrap().is_empty());
        let rows: i64 = f.store.conn.query_row("SELECT COUNT(*) FROM channels", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "a thread never outlives its parent");
    }

    fn rows(store: &Store, table: &str) -> i64 {
        store.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    /// A thread with a turn, an event and an approval, so a delete has every kind of row to remove.
    fn busy_thread(f: &Fixture, anchor: &MessageId) -> (Channel, Turn, Approval) {
        let thread = f.store.create_thread(&f.channel.id, anchor, "Why?", &f.agent).unwrap();
        let session = f.store.latest_session(&thread.id).unwrap().unwrap();
        let turn = Turn::new(session.clone());
        f.store.insert_turn(&turn).unwrap();
        f.store.update_turn_state(&turn.id, TurnState::Running).unwrap();
        f.store.append_event(&Event { id: EventId::new(), session_id: session, seq: 1, kind: EventKind::TextDelta { text: "x".into() }, raw: None }).unwrap();
        let approval = Approval::new(turn.id.clone(), "Bash", json!({}));
        f.store.save_approval(&approval).unwrap();
        (thread, turn, approval)
    }

    #[test]
    fn deleting_a_thread_with_no_posted_message_removes_every_row_it_owns() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::Agent, "answer", Some(&f.agent));
        let (thread, turn, approval) = busy_thread(&f, &anchor);
        say(&f.store, &thread.id, Role::User, "why", None);
        say(&f.store, &thread.id, Role::Agent, "because", Some(&f.agent));
        let before = ["sessions", "channels", "messages"].map(|t| rows(&f.store, t));
        assert!(matches!(f.store.delete_thread(&thread.id), Err(StoreError::Invalid(message)) if message.contains("turn runs")), "a running turn blocks the delete");
        f.store.update_turn_state(&turn.id, TurnState::Completed).unwrap();
        assert!(matches!(f.store.delete_thread(&thread.id), Err(StoreError::Invalid(message)) if message.contains("approval waits")), "a pending approval blocks the delete");
        let mut decided = approval.clone();
        decided.decide(Decision::Deny, Decider::User, "ask").unwrap();
        f.store.save_approval(&decided).unwrap();
        assert_eq!(["sessions", "channels", "messages"].map(|t| rows(&f.store, t)), before, "a refused delete changes nothing");
        let deletion = f.store.delete_thread(&thread.id).unwrap();
        assert_eq!(deletion, ThreadDeletion { parent: f.channel.id.clone(), posted: vec![] });
        assert!(f.store.thread_info(&thread.id).unwrap().is_none());
        assert!(f.store.load_channel(&thread.id).is_err());
        for (table, column) in [("sessions", "channel_id"), ("messages", "channel_id")] {
            let left: i64 = f.store.conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1"), params![thread.id.0], |r| r.get(0)).unwrap();
            assert_eq!(left, 0, "{table}");
        }
        assert_eq!((rows(&f.store, "turns"), rows(&f.store, "events"), rows(&f.store, "approvals"), rows(&f.store, "approval_audit")), (0, 0, 0, 0));
        assert_eq!(texts(&f.store.messages_for_channel(&f.channel.id).unwrap()), ["answer"]);
        assert!(f.store.thread_for_anchor(&anchor).unwrap().is_none(), "the anchor can start a new thread");
        assert!(matches!(f.store.delete_thread(&thread.id), Err(StoreError::NotFound(_))));
        assert!(matches!(f.store.delete_thread(&f.channel.id), Err(StoreError::NotFound(_))), "a normal channel is not a thread");
    }

    #[test]
    fn deleting_a_thread_keeps_each_posted_message_in_the_parent_at_its_place() {
        let f = fixture();
        let other = AgentId::new();
        f.store.insert_agent(&other, &store_agent_config()).unwrap();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        let first = say(&f.store, &thread.id, Role::User, "side question", None);
        let second = say(&f.store, &thread.id, Role::Agent, "side answer", Some(&other));
        let unposted = say(&f.store, &thread.id, Role::Agent, "private note", Some(&f.agent));
        let (user_ref, _) = f.store.post_thread_message(&first).unwrap();
        say(&f.store, &f.channel.id, Role::Agent, "parent reply", Some(&f.agent));
        let (agent_ref, _) = f.store.post_thread_message(&second).unwrap();
        let author_before = f.store.message_authors(&f.channel.id).unwrap().get(&agent_ref.id).cloned();
        assert!(author_before.is_some());
        assert_eq!(f.store.posted_message_count(&thread.id).unwrap(), 2);
        let deletion = f.store.delete_thread(&thread.id).unwrap();
        assert_eq!(deletion.posted, vec![user_ref.id.clone(), agent_ref.id.clone()], "in channel order");
        let parent = f.store.messages_for_channel(&f.channel.id).unwrap();
        assert_eq!(parent.iter().map(|m| m.id.clone()).collect::<Vec<_>>().len(), 4);
        assert_eq!(texts(&parent), ["question", "side question", "parent reply", "side answer"], "each row keeps its position");
        assert_eq!(parent.iter().map(|m| m.role).collect::<Vec<_>>(), [Role::User, Role::User, Role::Agent, Role::Agent]);
        assert_eq!((parent[1].id.clone(), parent[3].id.clone()), (user_ref.id.clone(), agent_ref.id.clone()), "the row IDs stay, so read markers and summaries stay valid");
        assert!(f.store.posted_sources(&f.channel.id).unwrap().is_empty(), "no reference row is left");
        assert_eq!(f.store.message_authors(&f.channel.id).unwrap().get(&agent_ref.id).cloned(), author_before, "the author stays");
        assert!(!f.store.message_authors(&f.channel.id).unwrap().contains_key(&user_ref.id));
        let source_rows: i64 = f.store.conn.query_row("SELECT COUNT(*) FROM messages WHERE source_message_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(source_rows, 0);
        let history = f.store.channel_history(&f.channel.id).unwrap();
        let replay = crate::conversation_context::replay_with_posted(&history.messages, &history.authors, &history.posted);
        assert!(replay.iter().all(|(_, text)| !text.contains("[From thread")), "no thread label after the delete");
        assert!(replay.iter().any(|(_, text)| text.contains("side answer") && text.contains("role=\"agent\"")), "the parent context keeps the posted message");
        assert!(replay.iter().all(|(_, text)| !text.contains("private note")));
        assert!(f.store.messages_for_channel(&thread.id).unwrap().is_empty());
        let left: i64 = f.store.conn.query_row("SELECT COUNT(*) FROM messages WHERE id=?1", params![unposted.0], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "a message that was not posted is gone");
    }

    #[test]
    fn a_running_sub_agent_thread_cannot_be_deleted_until_its_task_ends() {
        let f = fixture();
        say(&f.store, &f.channel.id, Role::User, "go", None);
        let thread = f.store.create_sub_agent_thread(&f.channel.id, "Research", &f.agent).unwrap();
        assert!(matches!(f.store.delete_thread(&thread.id), Err(StoreError::Invalid(message)) if message.contains("task still runs")));
        assert!(f.store.thread_info(&thread.id).unwrap().is_some());
        f.store.finish_sub_agent_thread(&thread.id, ThreadState::Failed).unwrap();
        f.store.delete_thread(&thread.id).unwrap();
        assert!(f.store.thread_info(&thread.id).unwrap().is_none());
    }

    #[test]
    fn removing_a_reference_keeps_the_thread_message_and_allows_posting_again() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        let reply = say(&f.store, &thread.id, Role::Agent, "because", Some(&f.agent));
        let (reference, _) = f.store.post_thread_message(&reply).unwrap();
        say(&f.store, &f.channel.id, Role::User, "later", None);
        let removed = f.store.remove_posted_reference(&reference.id).unwrap();
        assert_eq!(removed, PostedSource { source_message_id: reply.clone(), thread_channel_id: thread.id.clone(), thread_name: "Why?".into() });
        assert_eq!(texts(&f.store.messages_for_channel(&f.channel.id).unwrap()), ["question", "later"]);
        assert_eq!(texts(&f.store.messages_for_channel(&thread.id).unwrap()), ["because"], "the message stays in the thread");
        assert!(f.store.posted_sources(&f.channel.id).unwrap().is_empty());
        let history = f.store.channel_history(&f.channel.id).unwrap();
        assert!(crate::conversation_context::replay_with_posted(&history.messages, &history.authors, &history.posted).iter().all(|(_, text)| !text.contains("because")), "later turns do not see it");
        let (again, created) = f.store.post_thread_message(&reply).unwrap();
        assert!(created, "Post to channel works again");
        assert_ne!(again.id, reference.id);
        assert!(matches!(f.store.remove_posted_reference(&reply), Err(StoreError::Invalid(_))), "a thread message is not a reference");
        assert!(matches!(f.store.remove_posted_reference(&anchor), Err(StoreError::Invalid(_))), "a normal message is not a reference");
        assert!(matches!(f.store.remove_posted_reference(&MessageId::new()), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn removing_the_reference_that_ends_a_summary_moves_the_summary_to_the_row_before() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        let reply = say(&f.store, &thread.id, Role::Agent, "because", Some(&f.agent));
        let (reference, _) = f.store.post_thread_message(&reply).unwrap();
        let history = f.store.channel_history(&f.channel.id).unwrap();
        let replay = crate::conversation_context::replay_with_posted(&history.messages, &history.authors, &history.posted);
        let ending = replay.last().unwrap().0.clone();
        assert!(ending.0.starts_with(&reference.id.0));
        let before = replay[replay.len() - 2].0.clone();
        f.store.set_conversation_context(&f.channel.id, &crate::conversation_context::ConversationContext { summary: Some("s".into()), through: Some(ending), ..Default::default() }).unwrap();
        f.store.remove_posted_reference(&reference.id).unwrap();
        assert_eq!(f.store.conversation_context(&f.channel.id).unwrap().through, Some(before));
    }

    #[test]
    fn thread_names_normalize_and_rename_both_kinds() {
        for (input, expected) in [("  Plan  ", "Plan"), ("a\nb\r\nc", "a b c"), ("é".repeat(80).as_str(), "é".repeat(80).as_str())] {
            assert_eq!(normalize_thread_name(input).unwrap(), expected, "{input:?}");
        }
        for input in ["", "  \n ", &"a".repeat(81)] {
            assert_eq!(normalize_thread_name(input).unwrap_err(), "Enter a thread name of 1 to 80 characters.", "{input:?}");
        }
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::Agent, "answer", Some(&f.agent));
        let question = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        say(&f.store, &f.channel.id, Role::User, "go", None);
        let sub = f.store.create_sub_agent_thread(&f.channel.id, "Research", &f.agent).unwrap();
        assert_eq!(f.store.rename_thread(&question.id, " Better\nname ").unwrap(), "Better name");
        assert_eq!(f.store.rename_thread(&sub.id, "Better name").unwrap(), "Better name", "duplicate names are allowed");
        assert_eq!(f.store.thread_info(&question.id).unwrap().unwrap().name, "Better name");
        assert!(matches!(f.store.rename_thread(&question.id, "  "), Err(StoreError::Invalid(_))));
        assert_eq!(f.store.thread_info(&question.id).unwrap().unwrap().name, "Better name", "a refused name changes nothing");
        assert!(matches!(f.store.rename_thread(&f.channel.id, "x"), Err(StoreError::NotFound(_))), "a channel is not a thread");
        assert!(matches!(f.store.rename_thread(&ChannelId::new(), "x"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn a_renamed_thread_changes_the_replay_label_of_its_posted_messages() {
        let f = fixture();
        let anchor = say(&f.store, &f.channel.id, Role::User, "question", None);
        let thread = f.store.create_thread(&f.channel.id, &anchor, "Why?", &f.agent).unwrap();
        let reply = say(&f.store, &thread.id, Role::Agent, "because", Some(&f.agent));
        let (reference, _) = f.store.post_thread_message(&reply).unwrap();
        let label = |store: &Store| {
            let history = store.channel_history(&f.channel.id).unwrap();
            crate::conversation_context::replay_with_posted(&history.messages, &history.authors, &history.posted).into_iter().find(|(id, _)| id.0.starts_with(&reference.id.0)).unwrap().1
        };
        assert!(label(&f.store).contains("[From thread \"Why?\"]"));
        f.store.rename_thread(&thread.id, "Latency").unwrap();
        assert!(label(&f.store).contains("[From thread \"Latency\"]"), "{}", label(&f.store));
        assert_eq!(f.store.posted_sources(&f.channel.id).unwrap()[&reference.id].thread_name, "Latency");
    }
}

#[cfg(test)]
mod project_folder_tests {
    use super::*;
    #[test]
    fn folder_import_reuses_identity_and_folder_changes_reset_sessions() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let next = root.path().join("next");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&next).unwrap();
        let db = root.path().join("store.db");
        let store = Store::open(&db).unwrap();
        let project = store.import_project_folder(&first).unwrap();
        assert_eq!(
            store.import_project_folder(&first.join(".")).unwrap().id,
            project.id
        );
        let ws = crate::bootstrap::ensure_workspace(
            &store,
            Path::new(&project.path),
            "claude",
            crate::bootstrap::Role::Default,
            crate::bootstrap::WorkspaceChoice::Main,
        )
        .unwrap();
        let old_session = store.latest_session(&ws.channel.id).unwrap();
        let message = Message {
            id: MessageId::new(),
            channel_id: ws.channel.id.clone(),
            role: Role::User,
            blocks: vec![],
        };
        store.insert_message(&message).unwrap();
        store.change_project_folder(&project.id, &next).unwrap();
        assert_ne!(store.latest_session(&ws.channel.id).unwrap(), old_session);
        assert_eq!(
            store.messages_for_channel(&ws.channel.id).unwrap(),
            vec![message]
        );
        assert_eq!(store.selected_project().unwrap().unwrap().id, project.id);
        drop(store);
        let store = Store::open(&db).unwrap();
        assert_eq!(
            store.selected_project().unwrap().unwrap().path,
            next.canonicalize().unwrap().to_string_lossy()
        );
        let other = store.import_project_folder(&first).unwrap();
        assert_ne!(other.id, project.id);
        let target_workspace = crate::bootstrap::ensure_workspace(
            &store,
            Path::new(&other.path),
            "claude",
            crate::bootstrap::Role::Default,
            crate::bootstrap::WorkspaceChoice::Main,
        )
        .unwrap();
        let target_session = store.latest_session(&target_workspace.channel.id).unwrap();
        assert!(matches!(
            store.change_project_folder(&project.id, &first),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(store.selected_project().unwrap().unwrap().id, other.id);
        let channels = store.channels_for_project(&other.id).unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.iter().all(|c| c.id != ws.channel.id));
        assert_eq!(store.messages_for_channel(&ws.channel.id).unwrap().len(), 1);
        assert_eq!(
            store.latest_session(&target_workspace.channel.id).unwrap(),
            target_session
        );
        assert!(store.import_project_folder(&db).is_err());
    }
}

#[cfg(test)]
mod agent_lifecycle_tests {
    use super::*;
    use crate::capability::ApprovalPolicy;
    struct ChannelFixture {
        store: Store,
        project: Project,
        agent: AgentId,
        channel: Channel,
    }
    fn channel_fixture() -> ChannelFixture {
        let store = Store::open_in_memory().unwrap();
        let agent = store.create_agent(&config()).unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "P".into(),
            path: "/p".into(),
        };
        store.insert_project(&project).unwrap();
        let channel = store
            .create_conversation(&agent, Some(&project.id), "general")
            .unwrap();
        ChannelFixture {
            store,
            project,
            agent,
            channel,
        }
    }
    #[test]
    fn restore_rejects_archived_legacy_project_channels_but_keeps_direct_restore() {
        let fixture = channel_fixture();
        let original = fixture.store.ensure_project_general_channel(&fixture.project.id, &fixture.agent).unwrap();
        let replacement = fixture.store.create_conversation(&fixture.agent, Some(&fixture.project.id), "general").unwrap();
        fixture.store.set_canonical_project_channel(&fixture.project.id, &original.id).unwrap();
        fixture.store.set_canonical_project_channel(&fixture.project.id, &replacement.id).unwrap();
        assert!(fixture.store.channel_archived(&original.id).unwrap());
        let error = fixture.store.restore_channel(&original.id).unwrap_err().to_string();
        assert!(error.contains("Choose it through canonical channel details"));
        assert!(fixture.store.channel_archived(&original.id).unwrap(), "restore must not unarchive a linked legacy channel");

        let direct = fixture.store.default_conversation(&fixture.agent).unwrap();
        fixture.store.archive_channel(&direct.id).unwrap();
        fixture.store.restore_channel(&direct.id).unwrap();
        assert!(!fixture.store.channel_archived(&direct.id).unwrap(), "direct chat restore remains available");
    }
    fn config() -> AgentConfig {
        AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
            reasoning_effort: None,
            name: crate::agent::generated_name(),
            avatar: None,
            provider: "claude".into(),
            model: Some("same-model".into()),
            system_prompt: "Be clear".into(),
            policy: ApprovalPolicy::Ask,
            plugin_write: false,
        }
    }
    #[test]
    fn independent_agents_have_editable_names_and_default_direct_chats() {
        let store = Store::open_in_memory().unwrap();
        let mut cfg = config();
        assert!(!cfg.name.trim().is_empty());
        let a = store.create_agent(&cfg).unwrap();
        let b = store.create_agent(&cfg).unwrap();
        assert_ne!(a, b);
        cfg.name = "Edited".into();
        store.update_agent(&a, &cfg).unwrap();
        assert_eq!(store.load_agent(&a).unwrap().name, "Edited");
        let direct = store.default_conversation(&a).unwrap();
        assert_eq!(direct.project_id, None);
        assert_eq!(store.default_conversation(&a).unwrap().id, direct.id);
        assert!(store.latest_session(&direct.id).unwrap().is_some());
        store.create_conversation(&a, None, "Other").unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "P".into(),
            path: "/p".into(),
        };
        store.insert_project(&project).unwrap();
        let one = store
            .create_conversation(&a, Some(&project.id), "One")
            .unwrap();
        store
            .create_conversation(&a, Some(&project.id), "Two")
            .unwrap();
        assert_eq!(store.conversations_for_agent(&a).unwrap().len(), 4);
        assert_eq!(
            store.conversations_for_agent(&a).unwrap()[2].project_id,
            one.project_id
        );
    }
    #[test]
    fn archive_restore_and_delete_keep_history_and_return_only_private_scopes() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_agent(&config()).unwrap();
        let direct = store.default_conversation(&a).unwrap();
        store
            .insert_message(&Message {
                id: MessageId::new(),
                channel_id: direct.id.clone(),
                role: Role::User,
                blocks: vec![],
            })
            .unwrap();
        store.set_agent_archived(&a, true).unwrap();
        assert!(store.agent_archived(&a).unwrap());
        assert!(store.list_agents().unwrap().is_empty());
        store.set_agent_archived(&a, false).unwrap();
        assert!(!store.agent_archived(&a).unwrap());
        let result = store
            .delete_agent(
                &a,
                DeleteAgentOptions {
                    delete_conversations: false,
                    delete_private_memory: true,
                },
            )
            .unwrap();
        assert_eq!(
            result.private_scopes,
            vec![PrivateMemoryScope::Agent(a.clone())]
        );
        assert_eq!(store.messages_for_channel(&direct.id).unwrap().len(), 1);
        assert_eq!(store.load_agent(&a).unwrap().name, "Deleted agent");
        assert!(store.set_agent_archived(&a, false).is_err());
        let b = store.create_agent(&config()).unwrap();
        let chat = store.default_conversation(&b).unwrap();
        store
            .delete_agent(
                &b,
                DeleteAgentOptions {
                    delete_conversations: true,
                    delete_private_memory: false,
                },
            )
            .unwrap();
        assert!(store.conversations_for_agent(&b).unwrap().is_empty());
        assert!(store.latest_session(&chat.id).unwrap().is_none());
    }
    #[test]
    fn agent_validation_rejects_blank_fields() {
        let store = Store::open_in_memory().unwrap();
        let mut cfg = config();
        cfg.name = " ".into();
        assert!(store.create_agent(&cfg).is_err());
        cfg = config();
        cfg.model = None;
        assert!(store.create_agent(&cfg).is_err());
        cfg = config();
        cfg.provider = " ".into();
        assert!(store.create_agent(&cfg).is_err());
        cfg = config();
        cfg.model = Some(" ".into());
        assert!(store.create_agent(&cfg).is_err());
    }
    #[test]
    fn deletion_choices_are_independent_and_do_not_remove_shared_projects_or_other_agents() {
        for delete_conversations in [false, true] {
            for delete_private_memory in [false, true] {
                let store = Store::open_in_memory().unwrap();
                let agent = store.create_agent(&config()).unwrap();
                let other = store.create_agent(&config()).unwrap();
                let project = Project {
                    id: ProjectId::new(),
                    name: "Shared".into(),
                    path: "/shared".into(),
                };
                store.insert_project(&project).unwrap();
                let direct = store.default_conversation(&agent).unwrap();
                let chat = store
                    .create_conversation(&agent, Some(&project.id), "Project")
                    .unwrap();
                let other_chat = store
                    .create_conversation(&other, Some(&project.id), "Other agent")
                    .unwrap();
                for channel in [&direct, &chat, &other_chat] {
                    store
                        .insert_message(&Message {
                            id: MessageId::new(),
                            channel_id: channel.id.clone(),
                            role: Role::User,
                            blocks: vec![Block::Markdown {
                                text: "History".into(),
                            }],
                        })
                        .unwrap();
                    let session = store.latest_session(&channel.id).unwrap().unwrap();
                    let turn = Turn::new(session.clone());
                    store.insert_turn(&turn).unwrap();
                    let approval = Approval::new(turn.id.clone(), "Read", serde_json::json!({}));
                    store.save_approval(&approval).unwrap();
                    store
                        .append_event(&Event {
                            id: EventId::new(),
                            session_id: session,
                            seq: 1,
                            kind: EventKind::TextDelta {
                                text: "History".into(),
                            },
                            raw: None,
                        })
                        .unwrap();
                }
                let result = store
                    .delete_agent(
                        &agent,
                        DeleteAgentOptions {
                            delete_conversations,
                            delete_private_memory,
                        },
                    )
                    .unwrap();
                if delete_private_memory {
                    assert_eq!(result.private_scopes.len(), 2);
                    assert!(
                        result
                            .private_scopes
                            .contains(&PrivateMemoryScope::Agent(agent.clone()))
                    );
                    assert!(
                        result
                            .private_scopes
                            .contains(&PrivateMemoryScope::AgentProject {
                                agent: agent.clone(),
                                project: project.id.clone()
                            })
                    );
                } else {
                    assert!(result.private_scopes.is_empty());
                }
                for channel in [&direct, &chat] {
                    assert_eq!(
                        store.messages_for_channel(&channel.id).unwrap().len(),
                        usize::from(!delete_conversations)
                    );
                }
                assert_eq!(store.messages_for_channel(&other_chat.id).unwrap().len(), 1);
                assert_eq!(store.list_projects().unwrap(), vec![project]);
                assert_eq!(store.list_agents().unwrap().len(), 1);
                if !delete_conversations {
                    assert_eq!(
                        store.load_agent(&chat.agent_id).unwrap().name,
                        "Deleted agent"
                    );
                }
            }
        }
    }

    #[test]
    fn fixed_context_and_failed_creation_leave_no_partial_conversations() {
        let store = Store::open_in_memory().unwrap();
        let agent = store.create_agent(&config()).unwrap();
        let chat = store.default_conversation(&agent).unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "P".into(),
            path: "/p".into(),
        };
        store.insert_project(&project).unwrap();
        assert!(
            store
                .conn
                .execute(
                    "UPDATE channels SET project_id=?2 WHERE id=?1",
                    params![chat.id.0, project.id.0]
                )
                .is_err()
        );
        assert!(
            store
                .create_conversation(&agent, Some(&ProjectId::new()), "Missing project")
                .is_err()
        );
        assert_eq!(
            store.conversations_for_agent(&agent).unwrap(),
            vec![chat.clone()]
        );
        assert_eq!(
            store.ensure_default_conversation(&agent).unwrap().id,
            chat.id
        );
        assert_eq!(store.conversations_for_agent(&agent).unwrap().len(), 1);
    }

    #[test]
    fn clearing_an_explicit_model_is_rejected_without_mutating_config() {
        let store = Store::open_in_memory().unwrap();
        let cfg = config();
        let id = store.create_agent(&cfg).unwrap();
        let mut cleared = cfg.clone();
        cleared.model = None;
        assert!(matches!(
            store.update_agent(&id, &cleared),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(store.load_agent(&id).unwrap(), cfg);
    }

    #[test]
    fn project_general_memberships_selection_and_main_agent_are_transactional() {
        let f = channel_fixture();
        let general = f
            .store
            .ensure_project_general_channel(&f.project.id, &f.agent)
            .unwrap();
        assert_eq!(general.id, f.channel.id);
        assert_eq!(
            f.store
                .ensure_project_general_channel(&f.project.id, &f.agent)
                .unwrap()
                .id,
            general.id
        );
        assert_eq!(
            f.store.project_main_agent(&f.project.id).unwrap(),
            Some(f.agent.clone())
        );
        let other = AgentId::new();
        f.store
            .insert_agent(&other, &f.store.load_agent(&f.agent).unwrap())
            .unwrap();
        f.store
            .invite_agent_to_channel(&f.project.id, &general.id, &other)
            .unwrap();
        f.store
            .invite_agent_to_channel(&f.project.id, &general.id, &other)
            .unwrap();
        assert_eq!(
            f.store
                .project_agents(&f.project.id)
                .unwrap()
                .iter()
                .filter(|a| **a == other)
                .count(),
            1
        );
        assert_eq!(
            f.store
                .channel_agents(&general.id)
                .unwrap()
                .iter()
                .filter(|a| **a == other)
                .count(),
            1
        );
        f.store
            .select_project_channel(&f.project.id, &general.id)
            .unwrap();
        let direct = Channel {
            id: ChannelId::new(),
            project_id: None,
            name: "DM".into(),
            agent_id: f.agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        f.store.insert_channel(&direct).unwrap();
        assert_eq!(
            f.store
                .last_project_channel(&f.project.id)
                .unwrap()
                .unwrap()
                .id,
            general.id,
            "direct channel access has no project selection write"
        );
        let old_session = f.store.latest_session(&general.id).unwrap();
        f.store
            .set_project_main_agent(&f.project.id, &other)
            .unwrap();
        assert_eq!(f.store.load_channel(&general.id).unwrap().agent_id, other);
        assert_ne!(
            f.store.latest_session(&general.id).unwrap(),
            old_session,
            "provider sessions cannot cross agent identities"
        );
        assert!(
            f.store
                .invite_agent_to_channel(&ProjectId::new(), &general.id, &other)
                .is_err()
        );
        f.store.set_agent_archived(&other, true).unwrap();
        assert!(
            f.store
                .invite_agent_to_channel(&f.project.id, &general.id, &other)
                .is_err()
        );
    }

    #[test]
    #[expect(clippy::redundant_pattern_matching, reason = "Preserve the existing behavioral assertions.")]
    fn topic_compare_and_set_history_and_branch_retirement_preserve_identity() {
        let f = channel_fixture();
        assert_eq!(
            f.store.channel_topic(&f.channel.id).unwrap(),
            ChannelTopic {
                channel_id: f.channel.id.clone(),
                text: String::new(),
                version: 0
            }
        );
        let first = f
            .store
            .save_channel_topic(&f.channel.id, 0, "First", TopicAuthorKind::User, None)
            .unwrap();
        assert_eq!(first.version, 1);
        assert!(
            f.store
                .save_channel_topic(&f.channel.id, 0, "Stale", TopicAuthorKind::User, None)
                .is_err()
        );
        f.store
            .save_channel_topic(
                &f.channel.id,
                1,
                "Second",
                TopicAuthorKind::Agent,
                Some(&f.agent),
            )
            .unwrap();
        assert_eq!(
            f.store.channel_topic_history(&f.channel.id).unwrap().len(),
            2
        );
        assert!(
            f.store
                .save_channel_topic(
                    &f.channel.id,
                    2,
                    &"x".repeat(4097),
                    TopicAuthorKind::User,
                    None
                )
                .is_err()
        );

        let branch_channel = Channel {
            id: ChannelId::new(),
            name: "Feature".into(),
            ..f.channel.clone()
        };
        f.store.insert_channel(&branch_channel).unwrap();
        f.store
            .bind_branch(&branch_channel.id, "feature/one")
            .unwrap();
        f.store
            .rename_project_branch(&f.project.id, &branch_channel.id, "feature/two")
            .unwrap();
        assert_eq!(
            f.store
                .channel_branch(&branch_channel.id)
                .unwrap()
                .as_deref(),
            Some("feature/two")
        );
        f.store
            .retire_project_branch(&f.project.id, &branch_channel.id)
            .unwrap();
        assert!(f.store.project_branch_channel(&f.project.id, "feature/two").unwrap().is_none());
        assert_eq!(
            f.store.project_channel_metadata(&branch_channel.id).unwrap().unwrap().branch.as_deref(),
            Some("feature/two"),
            "retired metadata retains the branch lineage"
        );
        let replacement = Channel {
            id: ChannelId::new(),
            name: "Feature again".into(),
            ..f.channel.clone()
        };
        f.store.insert_channel(&replacement).unwrap();
        f.store.bind_branch(&replacement.id, "feature/two").unwrap();
        assert_eq!(f.store.project_branch_channel(&f.project.id, "feature/two").unwrap().unwrap().id, replacement.id);
        assert_eq!(
            f.store
                .project_channel_metadata(&replacement.id)
                .unwrap()
                .unwrap()
                .kind,
            ProjectChannelKind::Branch
        );
        assert!(matches!(
            f.store.messages_for_channel(&branch_channel.id),
            Ok(_)
        ));
    }

    #[test]
    #[expect(clippy::bool_assert_comparison, reason = "Preserve the existing behavioral assertions.")]
    fn v3_migration_collapses_duplicate_channels_without_losing_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("v3.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(SCHEMA_V2).unwrap();
        conn.execute_batch("ALTER TABLE worktrees ADD COLUMN owned INTEGER NOT NULL DEFAULT 1; CREATE TABLE channel_branches(channel_id TEXT PRIMARY KEY REFERENCES channels(id),branch TEXT NOT NULL);").unwrap();
        let cfg = config();
        conn.execute("INSERT INTO projects VALUES('p','P','/p')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO agents(id,config_toml) VALUES('a',?1)",
            params![cfg.to_toml().unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agents(id,config_toml) VALUES('b',?1)",
            params![cfg.to_toml().unwrap()],
        )
        .unwrap();
        for id in ["old", "new", "general-old", "general-new", "planning"] {
            let name = if id.starts_with("general") { "General" } else if id == "planning" { "Planning" } else { "Branch" };
            let agent = if id == "general-new" { "b" } else { "a" };
            conn.execute("INSERT INTO channels(id,project_id,name,agent_id,workspace_mode) VALUES(?1,'p',?2,?3,'main_checkout')",params![id,name,agent]).unwrap();
            conn.execute(
                "INSERT INTO sessions(id,channel_id) VALUES(?1,?2)",
                params![format!("s-{id}"), id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages(id,channel_id,role,blocks_json) VALUES(?1,?2,'user','[]')",
                params![format!("m-{id}"), id],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO channel_branches VALUES('old','topic')", [])
            .unwrap();
        conn.execute("INSERT INTO worktrees(channel_id,path,branch,base_commit,repo,owned) VALUES('new','/w/new','topic','abc','/p',1)", []).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        let store = Store::init(conn).unwrap();
        assert_eq!(
            store
                .project_channel_metadata(&ChannelId("new".into()))
                .unwrap()
                .unwrap()
                .kind,
            ProjectChannelKind::Branch
        );
        let duplicate = store
            .project_channel_metadata(&ChannelId("old".into()))
            .unwrap()
            .unwrap();
        assert_eq!(duplicate.kind, ProjectChannelKind::ArchivedLegacy);
        assert_eq!(duplicate.archived_into, Some(ChannelId("new".into())));
        assert_eq!(
            store.channel_archived(&ChannelId("old".into())).unwrap(),
            true
        );
        assert_eq!(
            store
                .project_channel_metadata(&ChannelId("general-new".into()))
                .unwrap()
                .unwrap()
                .kind,
            ProjectChannelKind::General
        );
        assert_eq!(
            store
                .channel_archived(&ChannelId("general-old".into()))
                .unwrap(),
            true
        );
        assert_eq!(store.project_main_agent(&ProjectId("p".into())).unwrap(), Some(AgentId("b".into())));
        assert_eq!(store.load_channel(&ChannelId("general-new".into())).unwrap().agent_id, AgentId("b".into()));
        assert_eq!(store.channel_archived(&ChannelId("planning".into())).unwrap(), true);
        assert!(!store.channels_for_project(&ProjectId("p".into())).unwrap().iter().any(|c| c.id.0 == "planning"));
        assert_eq!(store.messages_for_channel(&ChannelId("planning".into())).unwrap().len(), 1);
        for id in ["old", "new", "general-old", "general-new", "planning"] {
            assert_eq!(
                store
                    .messages_for_channel(&ChannelId(id.into()))
                    .unwrap()
                    .len(),
                1
            );
        }

        store.select_project_channel(&ProjectId("p".into()), &ChannelId("new".into())).unwrap();
        store.set_canonical_project_branch(&ProjectId("p".into()), "topic", &ChannelId("old".into())).unwrap();
        assert!(store.channel_archived(&ChannelId("new".into())).unwrap());
        assert!(!store.channel_archived(&ChannelId("old".into())).unwrap());
        assert!(store.last_project_channel(&ProjectId("p".into())).unwrap().is_none());
        assert_eq!(store.project_branch_channel(&ProjectId("p".into()), "topic").unwrap().unwrap().id, ChannelId("old".into()));

        store.select_project_channel(&ProjectId("p".into()), &ChannelId("general-new".into())).unwrap();
        let old_general_session = store.latest_session(&ChannelId("general-old".into())).unwrap();
        store.set_canonical_project_channel(&ProjectId("p".into()), &ChannelId("general-old".into())).unwrap();
        assert!(store.channel_archived(&ChannelId("general-new".into())).unwrap());
        assert!(!store.channel_archived(&ChannelId("general-old".into())).unwrap());
        assert_eq!(store.load_channel(&ChannelId("general-old".into())).unwrap().agent_id, AgentId("b".into()));
        assert_ne!(store.latest_session(&ChannelId("general-old".into())).unwrap(), old_general_session);
        assert!(store.last_project_channel(&ProjectId("p".into())).unwrap().is_none());
        assert_eq!(store.project_channel_or_fallback(&ProjectId("p".into()), &AgentId("b".into())).unwrap().id, ChannelId("general-old".into()));
        drop(store);
        let store = Store::open(&db).unwrap();
        assert!(store.channel_archived(&ChannelId("planning".into())).unwrap());
        assert_eq!(store.messages_for_channel(&ChannelId("planning".into())).unwrap().len(), 1);
    }

    #[test]
    fn deleting_channels_cleans_channel_metadata_topics_and_memberships() {
        let f = channel_fixture();
        let general = f
            .store
            .ensure_project_general_channel(&f.project.id, &f.agent)
            .unwrap();
        f.store.select_project_channel(&f.project.id, &general.id).unwrap();
        f.store
            .save_channel_topic(&general.id, 0, "Topic", TopicAuthorKind::User, None)
            .unwrap();
        f.store
            .delete_agent(
                &f.agent,
                DeleteAgentOptions {
                    delete_conversations: true,
                    delete_private_memory: false,
                },
            )
            .unwrap();
        assert!(f.store.project_channel_metadata(&general.id).unwrap().is_none());
        assert!(f.store.channel_topic_history(&general.id).unwrap().is_empty());
        assert!(f.store.channel_agents(&general.id).unwrap().is_empty());
        assert!(f.store.last_project_channel(&f.project.id).unwrap().is_none());
    }

    #[test]
    fn named_branch_creation_uses_a_savepoint_and_rolls_back_later_failures() {
        let f = channel_fixture();
        let branch = Channel {
            id: ChannelId::new(),
            project_id: Some(f.project.id.clone()),
            name: "Feature".into(),
            agent_id: f.agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        f.store.create_named_channel(&branch, Some("feature/one"), None).unwrap();
        assert_eq!(f.store.project_branch_channel(&f.project.id, "feature/one").unwrap().unwrap().id, branch.id);
        assert!(f.store.latest_session(&branch.id).unwrap().is_some());

        let failing = Channel { id: ChannelId::new(), name: "Will roll back".into(), ..branch.clone() };
        let bad_worktree = WorktreeRecord {
            channel_id: ChannelId::new(),
            path: "/w/fail".into(),
            branch: "feature/two".into(),
            base_commit: "abc".into(),
            repo: f.project.path.clone(),
            owned: true,
        };
        assert!(f.store.create_named_channel(&failing, Some("feature/two"), Some(&bad_worktree)).is_err());
        assert!(matches!(f.store.load_channel(&failing.id), Err(StoreError::Sqlite(_))));
        assert_eq!(f.store.latest_session(&failing.id).unwrap(), None);
        assert_eq!(f.store.channel_branch(&failing.id).unwrap(), None);
        assert!(f.store.project_branch_channel(&f.project.id, "feature/two").unwrap().is_none());
    }

    #[test]
    fn deleting_either_owner_of_a_linked_branch_keeps_the_other_history() {
        for delete_archive_first in [false, true] {
            let store = Store::open_in_memory().unwrap();
            let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
            store.insert_project(&project).unwrap();
            let archive_agent = store.create_agent(&config()).unwrap();
            let canonical_agent = store.create_agent(&config()).unwrap();
            let archived = store.create_conversation(&archive_agent, Some(&project.id), "Old branch").unwrap();
            let canonical = store.create_conversation(&canonical_agent, Some(&project.id), "Current branch").unwrap();
            store.bind_branch(&archived.id, "feature/shared").unwrap();
            store.set_canonical_project_branch(&project.id, "feature/shared", &canonical.id).unwrap();
            for channel in [&archived, &canonical] {
                store.insert_message(&Message { id: MessageId::new(), channel_id: channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: channel.name.clone() }] }).unwrap();
            }

            if delete_archive_first {
                store.delete_agent(&archive_agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
                assert_eq!(store.messages_for_channel(&canonical.id).unwrap().len(), 1);
                store.delete_agent(&canonical_agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
            } else {
                store.delete_agent(&canonical_agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
                assert_eq!(store.messages_for_channel(&archived.id).unwrap().len(), 1);
                assert_eq!(store.project_channel_metadata(&archived.id).unwrap().unwrap().archived_into, None);
                store.delete_agent(&archive_agent, DeleteAgentOptions { delete_conversations: true, delete_private_memory: false }).unwrap();
            }
        }
    }

    #[test]
    fn general_agent_reconciles_on_reuse_same_setting_and_main_agent_deletion() {
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let main = store.create_agent(&config()).unwrap();
        let legacy_agent = store.create_agent(&config()).unwrap();
        let legacy_general = store.create_conversation(&legacy_agent, Some(&project.id), "general").unwrap();
        let message = Message { id: MessageId::new(), channel_id: legacy_general.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "history".into() }] };
        store.insert_message(&message).unwrap();
        let old_session = store.latest_session(&legacy_general.id).unwrap();

        let general = store.ensure_project_general_channel(&project.id, &main).unwrap();
        assert_eq!(general.id, legacy_general.id);
        assert_eq!(general.agent_id, main);
        assert_ne!(store.latest_session(&general.id).unwrap(), old_session);
        assert_eq!(store.messages_for_channel(&general.id).unwrap(), vec![message.clone()]);
        assert_eq!(store.channel_agents(&general.id).unwrap(), vec![legacy_agent.clone(), main.clone()]);

        let wrong_session = SessionId::new();
        store.conn.execute("UPDATE channels SET agent_id=?2 WHERE id=?1", params![general.id.0, legacy_agent.0]).unwrap();
        store.create_session(&wrong_session, &general.id).unwrap();
        let before_repair = store.latest_session(&general.id).unwrap();
        store.set_project_main_agent(&project.id, &main).unwrap();
        assert_eq!(store.load_channel(&general.id).unwrap().agent_id, main);
        assert_ne!(store.latest_session(&general.id).unwrap(), before_repair);

        store.invite_agent_to_channel(&project.id, &general.id, &legacy_agent).unwrap();
        let before_delete = store.latest_session(&general.id).unwrap();
        store.delete_agent(&main, DeleteAgentOptions { delete_conversations: false, delete_private_memory: false }).unwrap();
        assert_eq!(store.project_main_agent(&project.id).unwrap(), Some(legacy_agent.clone()));
        assert_eq!(store.load_channel(&general.id).unwrap().agent_id, legacy_agent);
        assert_ne!(store.latest_session(&general.id).unwrap(), before_delete);
        assert_eq!(store.messages_for_channel(&general.id).unwrap(), vec![message]);
    }

    #[test]
    fn archived_general_is_retired_and_replaced_on_next_ensure() {
        let f = channel_fixture();
        let old = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        f.store.archive_channel(&old.id).unwrap();
        let current = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        assert_ne!(old.id, current.id);
        assert_eq!(f.store.project_channel_metadata(&old.id).unwrap().unwrap().archived_into, Some(current.id.clone()));
        assert_eq!(f.store.project_channel_metadata(&current.id).unwrap().unwrap().kind, ProjectChannelKind::General);
    }
}

#[cfg(test)]
mod text_channel_tests {
    use super::*;
    use crate::capability::ApprovalPolicy;

    struct Fixture {
        store: Store,
        project: Project,
        agent: AgentId,
    }

    fn config(name: &str) -> AgentConfig {
        AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
            reasoning_effort: None,
            name: name.into(),
            avatar: None,
            provider: "claude".into(),
            model: Some("model".into()),
            system_prompt: String::new(),
            policy: ApprovalPolicy::Ask,
            plugin_write: false,
        }
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let agent = store.create_agent(&config("Main")).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        store.ensure_project_general_channel(&project.id, &agent).unwrap();
        Fixture { store, project, agent }
    }

    fn message(store: &Store, channel: &ChannelId, text: &str) -> MessageId {
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: text.into() }] };
        store.insert_message(&message).unwrap();
        message.id
    }

    #[test]
    fn names_normalize_to_lowercase_words_joined_by_hyphens() {
        for (input, expected) in [("  Release Notes  ", "release-notes"), ("a   b\t c", "a-b-c"), ("a -- b", "a-b"), ("--Plan--", "plan"), ("keep_this-1", "keep_this-1")] {
            assert_eq!(normalize_text_channel_name(input).unwrap(), expected, "{input:?}");
        }
        for input in ["café", "a/b", "#plan", "a.b"] {
            assert_eq!(normalize_text_channel_name(input).unwrap_err(), "Use letters, numbers, hyphens, and underscores.", "{input:?}");
        }
        for input in ["", "   ", "---", &"a".repeat(51)] {
            assert_eq!(normalize_text_channel_name(input).unwrap_err(), "Enter a channel name of 1 to 50 characters.", "{input:?}");
        }
        assert_eq!(normalize_text_channel_name(&"a".repeat(50)).unwrap().len(), 50);
    }

    #[test]
    fn a_text_channel_uses_the_main_agent_and_the_main_checkout() {
        let f = fixture();
        let channel = f.store.create_project_text_channel(&f.project.id, "Release Notes").unwrap();
        assert_eq!(channel.name, "release-notes");
        assert_eq!(channel.agent_id, f.agent);
        assert_eq!(channel.workspace_mode, WorkspaceMode::MainCheckout);
        assert_eq!(f.store.load_channel(&channel.id).unwrap(), channel);
        assert!(f.store.latest_session(&channel.id).unwrap().is_some());
        assert_eq!(f.store.channel_agents(&channel.id).unwrap(), vec![f.agent.clone()]);
        let metadata = f.store.project_channel_metadata(&channel.id).unwrap().unwrap();
        assert_eq!((metadata.kind, metadata.branch, metadata.archived_into), (ProjectChannelKind::Text, None, None));
        assert!(f.store.channels_for_project(&f.project.id).unwrap().iter().any(|c| c.id == channel.id));
    }

    #[test]
    fn a_text_channel_name_is_unique_among_active_general_and_text_channels() {
        let f = fixture();
        let first = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        for name in ["plans", "PLANS", " Plans ", "general", "General"] {
            let error = f.store.create_project_text_channel(&f.project.id, name).unwrap_err();
            assert!(matches!(&error, StoreError::Invalid(message) if message == "A channel with this name already exists."), "{name}: {error:?}");
        }
        assert!(matches!(f.store.create_project_text_channel(&f.project.id, "bad/name"), Err(StoreError::Invalid(message)) if message.starts_with("Use letters")));
        f.store.archive_channel(&first.id).unwrap();
        let again = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        assert_ne!(again.id, first.id, "an archived text channel frees its name");
        let other = Project { id: ProjectId::new(), name: "Q".into(), path: "/q".into() };
        f.store.insert_project(&other).unwrap();
        f.store.ensure_project_general_channel(&other.id, &f.agent).unwrap();
        f.store.create_project_text_channel(&other.id, "plans").unwrap();
    }

    #[test]
    fn a_text_channel_needs_a_project_and_a_main_agent() {
        let f = fixture();
        assert!(matches!(f.store.create_project_text_channel(&ProjectId::new(), "plans"), Err(StoreError::NotFound(_))));
        let bare = Project { id: ProjectId::new(), name: "Bare".into(), path: "/bare".into() };
        f.store.insert_project(&bare).unwrap();
        assert!(matches!(f.store.create_project_text_channel(&bare.id, "plans"), Err(StoreError::Invalid(message)) if message.contains("no main agent")));
        assert!(f.store.all_channels_for_project(&bare.id).unwrap().is_empty(), "a failed create leaves nothing behind");
    }

    #[test]
    fn a_text_channel_is_never_the_canonical_general_channel() {
        let f = fixture();
        let general = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let text = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        assert_eq!(f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap().id, general.id);
        assert!(f.store.set_canonical_project_channel(&f.project.id, &text.id).is_err());
        let metadata = f.store.project_channel_metadata(&text.id).unwrap().unwrap();
        assert_eq!(metadata.kind, ProjectChannelKind::Text);
        assert_eq!(f.store.project_channel_metadata(&general.id).unwrap().unwrap().kind, ProjectChannelKind::General);
        // An archived text channel stays text and does not give way to a new general channel.
        f.store.archive_channel(&text.id).unwrap();
        assert_eq!(f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap().id, general.id);
        assert_eq!(f.store.project_channel_metadata(&text.id).unwrap().unwrap().kind, ProjectChannelKind::Text);
    }

    #[test]
    fn active_text_channels_follow_the_project_main_agent() {
        let f = fixture();
        let general = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let active = f.store.create_project_text_channel(&f.project.id, "active").unwrap();
        let archived = f.store.create_project_text_channel(&f.project.id, "old").unwrap();
        f.store.archive_channel(&archived.id).unwrap();
        let next = f.store.create_agent(&config("Next")).unwrap();
        let (active_session, archived_session) = (f.store.latest_session(&active.id).unwrap(), f.store.latest_session(&archived.id).unwrap());
        f.store.set_project_main_agent(&f.project.id, &next).unwrap();
        assert_eq!(f.store.load_channel(&general.id).unwrap().agent_id, next);
        assert_eq!(f.store.load_channel(&active.id).unwrap().agent_id, next);
        assert_ne!(f.store.latest_session(&active.id).unwrap(), active_session, "the channel starts a new session");
        assert_eq!(f.store.channel_agents(&active.id).unwrap(), vec![f.agent.clone(), next.clone()]);
        assert_eq!(f.store.load_channel(&archived.id).unwrap().agent_id, f.agent, "an archived text channel keeps its agent");
        assert_eq!(f.store.latest_session(&archived.id).unwrap(), archived_session);
        f.store.delete_agent(&next, DeleteAgentOptions { delete_conversations: false, delete_private_memory: false }).unwrap();
        assert_eq!(f.store.project_main_agent(&f.project.id).unwrap(), Some(f.agent.clone()));
        assert_eq!(f.store.load_channel(&active.id).unwrap().agent_id, f.agent);
    }

    #[test]
    fn show_archived_is_off_by_default_and_saved_for_each_project() {
        let f = fixture();
        let other = Project { id: ProjectId::new(), name: "Q".into(), path: "/q".into() };
        f.store.insert_project(&other).unwrap();
        assert!(!f.store.project_show_archived(&f.project.id).unwrap());
        f.store.set_project_show_archived(&f.project.id, true).unwrap();
        assert!(f.store.project_show_archived(&f.project.id).unwrap());
        assert!(!f.store.project_show_archived(&other.id).unwrap());
        f.store.set_project_show_archived(&f.project.id, false).unwrap();
        assert!(!f.store.project_show_archived(&f.project.id).unwrap());
        assert!(matches!(f.store.set_project_show_archived(&ProjectId::new(), true), Err(StoreError::NotFound(_))));
        assert!(matches!(f.store.project_show_archived(&ProjectId::new()), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn a_channel_has_messages_when_it_or_one_of_its_threads_has_one() {
        let f = fixture();
        let channel = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        assert!(!f.store.channel_has_messages(&channel.id).unwrap());
        let anchor = message(&f.store, &channel.id, "question");
        let thread = f.store.create_thread(&channel.id, &anchor, "question", &f.agent).unwrap();
        assert!(f.store.channel_has_messages(&channel.id).unwrap(), "the channel holds the anchor message");
        let bare = f.store.create_project_text_channel(&f.project.id, "bare").unwrap();
        assert!(!f.store.channel_has_messages(&bare.id).unwrap());
        // A thread message counts for the parent even when the parent has no message of its own.
        f.store.conn.execute("DELETE FROM messages WHERE id=?1", params![anchor.0]).unwrap();
        assert!(!f.store.channel_has_messages(&channel.id).unwrap());
        message(&f.store, &thread.id, "reply in thread");
        assert!(f.store.channel_has_messages(&channel.id).unwrap());
        assert!(!f.store.channel_has_messages(&bare.id).unwrap());
    }

    #[test]
    fn migration_13_adds_text_channels_and_show_archived_to_a_version_12_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v12.db");
        let (project, general, branch) = {
            let store = Store::open(&path).unwrap();
            let agent = store.create_agent(&config("Main")).unwrap();
            let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
            store.insert_project(&project).unwrap();
            let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
            let branch = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "feature".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
            store.create_named_repository_channel(&branch, "/p/sub", Some("feature"), None).unwrap();
            store.set_project_show_archived(&project.id, true).unwrap();
            // Put the database back in its version 12 shape.
            store.conn.execute_batch("ALTER TABLE project_channels RENAME TO project_channels_v13;
                CREATE TABLE project_channels (
                    channel_id TEXT PRIMARY KEY REFERENCES channels(id),
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    kind TEXT NOT NULL CHECK(kind IN ('general','branch','archived_legacy','legacy')),
                    branch_name TEXT,
                    archived_into TEXT REFERENCES channels(id),
                    repository TEXT NOT NULL DEFAULT '');
                INSERT INTO project_channels SELECT * FROM project_channels_v13;
                DROP TABLE project_channels_v13;
                CREATE UNIQUE INDEX one_project_general_channel ON project_channels(project_id) WHERE kind='general';
                CREATE UNIQUE INDEX one_project_active_branch ON project_channels(project_id,repository,branch_name) WHERE kind='branch' AND branch_name IS NOT NULL;
                ALTER TABLE projects DROP COLUMN show_archived;
                PRAGMA user_version=12;").unwrap();
            let text = store.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES('x',?1,'text')", params![project.id.0]);
            assert!(text.is_err(), "the version 12 table refuses the text kind");
            (project, general, branch)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        assert!(!store.project_show_archived(&project.id).unwrap(), "the new column defaults to off");
        let general_metadata = store.project_channel_metadata(&general.id).unwrap().unwrap();
        assert_eq!((general_metadata.kind, general_metadata.project_id.clone()), (ProjectChannelKind::General, project.id.clone()));
        let branch_metadata = store.project_channel_metadata(&branch.id).unwrap().unwrap();
        assert_eq!((branch_metadata.kind, branch_metadata.branch.as_deref(), branch_metadata.repository.as_str()), (ProjectChannelKind::Branch, Some("feature"), "/p/sub"));
        let indexes: Vec<String> = store.conn.prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='project_channels' AND name LIKE 'one_project_%' ORDER BY name").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(indexes, ["one_project_active_branch", "one_project_general_channel"]);
        let text = store.create_project_text_channel(&project.id, "plans").unwrap();
        assert_eq!(store.project_channel_metadata(&text.id).unwrap().unwrap().kind, ProjectChannelKind::Text);
        let other = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "feature-2".into(), agent_id: text.agent_id.clone(), workspace_mode: WorkspaceMode::Worktree };
        store.insert_channel(&other).unwrap();
        assert!(store.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name,repository) VALUES(?1,?2,'branch','feature','/p/sub')", params![other.id.0, project.id.0]).is_err(), "the branch index still holds");
        assert!(store.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind) VALUES(?1,?2,'general')", params![other.id.0, project.id.0]).is_err(), "the general index still holds");
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
    }

    #[test]
    fn a_text_channel_renames_with_the_creation_rules() {
        let f = fixture();
        let general = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let plans = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        let notes = f.store.create_project_text_channel(&f.project.id, "notes").unwrap();
        let renamed = f.store.rename_text_channel(&f.project.id, &plans.id, "  Release Notes ").unwrap();
        assert_eq!((renamed.name.as_str(), renamed.id.clone()), ("release-notes", plans.id.clone()));
        assert_eq!(f.store.load_channel(&plans.id).unwrap().name, "release-notes");
        for name in ["NOTES", "general", "General"] {
            assert!(matches!(f.store.rename_text_channel(&f.project.id, &plans.id, name), Err(StoreError::Invalid(message)) if message == "A channel with this name already exists."), "{name}");
        }
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &plans.id, "bad/name"), Err(StoreError::Invalid(message)) if message.starts_with("Use letters")));
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &plans.id, "---"), Err(StoreError::Invalid(message)) if message.contains("1 to 50")));
        assert_eq!(f.store.load_channel(&plans.id).unwrap().name, "release-notes", "a refused name changes nothing");
        // The same name, in another spelling, is the unchanged name: it is not a clash with the channel itself.
        assert_eq!(f.store.rename_text_channel(&f.project.id, &plans.id, "Release  NOTES").unwrap().name, "release-notes");
        // A name that an archived channel holds is free.
        f.store.archive_channel(&notes.id).unwrap();
        assert_eq!(f.store.rename_text_channel(&f.project.id, &plans.id, "notes").unwrap().name, "notes");
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &notes.id, "again"), Err(StoreError::Invalid(message)) if message.contains("Restore")), "an archived channel stays as it is");
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &general.id, "main"), Err(StoreError::Invalid(message)) if message.contains("text channel")), "#general cannot be renamed");
        assert_eq!(f.store.load_channel(&general.id).unwrap().name, general.name);
        assert!(matches!(f.store.rename_text_channel(&ProjectId::new(), &plans.id, "x"), Err(StoreError::NotFound(_))), "the channel must belong to the project");
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &ChannelId::new(), "x"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn a_branch_channel_cannot_be_renamed_as_a_text_channel() {
        let f = fixture();
        f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let branch = Channel { id: ChannelId::new(), project_id: Some(f.project.id.clone()), name: "feature".into(), agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        f.store.insert_channel(&branch).unwrap();
        f.store.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name) VALUES(?1,?2,'branch','feature')", params![branch.id.0, f.project.id.0]).unwrap();
        assert!(matches!(f.store.rename_text_channel(&f.project.id, &branch.id, "other"), Err(StoreError::Invalid(message)) if message.contains("text channel")));
        assert_eq!(f.store.load_channel(&branch.id).unwrap().name, "feature");
    }
}

#[cfg(test)]
mod navigation_action_tests {
    use super::*;
    use crate::capability::ApprovalPolicy;

    struct Fixture {
        store: Store,
        project: Project,
        agent: AgentId,
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let config = AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false, reasoning_effort: None,
            name: "Main".into(), avatar: None, provider: "claude".into(), model: Some("model".into()),
            system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
        };
        let agent = store.create_agent(&config).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        Fixture { store, project, agent }
    }

    fn note(store: &Store, channel: &ChannelId, text: &str) -> MessageId {
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: text.into() }] };
        store.insert_message(&message).unwrap();
        message.id
    }

    #[test]
    fn the_mute_flag_is_off_by_default_and_round_trips_without_other_changes() {
        let f = fixture();
        let general = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let text = f.store.create_project_text_channel(&f.project.id, "plans").unwrap();
        let message = note(&f.store, &text.id, "hello");
        assert!(!f.store.channel_muted(&general.id).unwrap());
        assert!(!f.store.channel_muted(&text.id).unwrap());
        f.store.set_channel_muted(&text.id, true).unwrap();
        assert!(f.store.channel_muted(&text.id).unwrap());
        assert!(!f.store.channel_muted(&general.id).unwrap(), "another channel is not muted");
        assert_eq!(f.store.load_channel(&text.id).unwrap(), text, "mute does not change the channel");
        assert_eq!(f.store.messages_for_channel(&text.id).unwrap().iter().map(|m| m.id.clone()).collect::<Vec<_>>(), [message]);
        f.store.set_channel_muted(&text.id, false).unwrap();
        assert!(!f.store.channel_muted(&text.id).unwrap());
        // #general can be muted. An archived channel keeps its flag.
        f.store.set_channel_muted(&general.id, true).unwrap();
        assert!(f.store.channel_muted(&general.id).unwrap());
        f.store.set_channel_muted(&text.id, true).unwrap();
        f.store.archive_channel(&text.id).unwrap();
        assert!(f.store.channel_muted(&text.id).unwrap());
    }

    #[test]
    fn migration_15_adds_the_rail_layout_and_keeps_the_project_order_of_a_version_14_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v14.db");
        let ids = ["first", "second", "third"].map(|id| ProjectId(id.into()));
        {
            let store = Store::open(&path).unwrap();
            for id in &ids {
                store.insert_project(&Project { id: id.clone(), name: id.0.clone(), path: format!("/{}", id.0) }).unwrap();
            }
            store.rail_group_projects(&ids[2], &ids[0]).unwrap();
            store.conn.execute_batch("DROP TABLE rail_projects; DROP TABLE rail_groups; PRAGMA user_version=14;").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        let layout = store.rail_layout().unwrap();
        assert_eq!(layout.items, ids.iter().cloned().map(RailItem::Project).collect::<Vec<_>>(), "no saved order keeps the visible_projects order");
        let group = store.rail_group_projects(&ids[1], &ids[0]).unwrap();
        assert!(matches!(&store.rail_layout().unwrap().items[0], RailItem::Group(g) if g.id == group && g.project_ids == [ids[0].clone(), ids[1].clone()]));
        assert!(Store::open(&path).unwrap().rail_layout().unwrap().items.len() == 2, "the layout survives a reopen");
    }

    #[test]
    fn a_thread_and_an_unknown_channel_cannot_be_muted() {
        let f = fixture();
        let general = f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let anchor = note(&f.store, &general.id, "why?");
        let thread = f.store.create_thread(&general.id, &anchor, "Why", &f.agent).unwrap();
        assert!(matches!(f.store.set_channel_muted(&thread.id, true), Err(StoreError::Invalid(message)) if message.contains("thread")));
        assert!(!f.store.channel_muted(&thread.id).unwrap());
        assert!(matches!(f.store.set_channel_muted(&ChannelId::new(), true), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn migration_14_adds_the_mute_flag_to_a_version_13_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v13.db");
        let (project, general) = {
            let f = fixture();
            drop(f);
            let store = Store::open(&path).unwrap();
            let config = AgentConfig {
                workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false, reasoning_effort: None,
                name: "Main".into(), avatar: None, provider: "claude".into(), model: Some("model".into()),
                system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
            };
            let agent = store.create_agent(&config).unwrap();
            let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
            store.insert_project(&project).unwrap();
            let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
            store.set_channel_muted(&general.id, true).unwrap();
            store.conn.execute_batch("ALTER TABLE channels DROP COLUMN muted; PRAGMA user_version=13;").unwrap();
            (project, general)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 16);
        assert!(!store.channel_muted(&general.id).unwrap(), "the new column defaults to off");
        assert_eq!(store.load_channel(&general.id).unwrap().project_id, Some(project.id));
        store.set_channel_muted(&general.id, true).unwrap();
        assert!(Store::open(&path).unwrap().channel_muted(&general.id).unwrap(), "the flag survives a reopen");
    }

    #[test]
    fn a_branch_rename_moves_the_binding_and_keeps_the_channel_history_and_worktree() {
        let f = fixture();
        f.store.ensure_project_general_channel(&f.project.id, &f.agent).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: Some(f.project.id.clone()), name: "feature/one".into(), agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let record = WorktreeRecord { channel_id: channel.id.clone(), path: "/w/feature".into(), branch: "feature/one".into(), base_commit: "abc".into(), repo: "/p".into(), owned: true };
        f.store.create_named_repository_channel(&channel, "", Some("feature/one"), Some(&record)).unwrap();
        f.store.register_bootstrap_project_channel(&channel.id).unwrap();
        let message = note(&f.store, &channel.id, "keep me");
        let session = f.store.latest_session(&channel.id).unwrap();
        f.store.rename_project_branch(&f.project.id, &channel.id, "feature/two").unwrap();
        assert!(f.store.project_branch_channel(&f.project.id, "feature/one").unwrap().is_none());
        assert_eq!(f.store.project_branch_channel(&f.project.id, "feature/two").unwrap().unwrap().id, channel.id);
        assert_eq!(f.store.channel_branch(&channel.id).unwrap().as_deref(), Some("feature/two"));
        assert_eq!(f.store.project_channel_metadata(&channel.id).unwrap().unwrap().branch.as_deref(), Some("feature/two"));
        assert_eq!(f.store.worktree_for(&channel.id).unwrap().unwrap(), WorktreeRecord { branch: "feature/two".into(), ..record });
        assert_eq!(f.store.messages_for_channel(&channel.id).unwrap().iter().map(|m| m.id.clone()).collect::<Vec<_>>(), [message]);
        assert_eq!(f.store.latest_session(&channel.id).unwrap(), session);
        assert_eq!(f.store.load_channel(&channel.id).unwrap(), channel);
    }

    #[test]
    fn a_branch_rename_that_clashes_with_another_branch_channel_changes_nothing() {
        let f = fixture();
        let make = |name: &str| {
            let channel = Channel { id: ChannelId::new(), project_id: Some(f.project.id.clone()), name: name.into(), agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
            f.store.create_named_repository_channel(&channel, "", Some(name), None).unwrap();
            channel
        };
        let (one, two) = (make("one"), make("two"));
        assert!(f.store.rename_project_branch(&f.project.id, &one.id, "two").is_err());
        assert_eq!(f.store.channel_branch(&one.id).unwrap().as_deref(), Some("one"), "the failed rename rolls back");
        assert_eq!(f.store.channel_branch(&two.id).unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn worktree_records_are_deleted_together_and_the_channels_stay() {
        let f = fixture();
        let make = |name: &str| {
            let channel = Channel { id: ChannelId::new(), project_id: Some(f.project.id.clone()), name: name.into(), agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
            let record = WorktreeRecord { channel_id: channel.id.clone(), path: "/w/shared".into(), branch: name.into(), base_commit: "abc".into(), repo: "/p".into(), owned: false };
            f.store.create_named_repository_channel(&channel, "", Some(name), Some(&record)).unwrap();
            channel
        };
        let (one, two, other) = (make("one"), make("two"), make("other"));
        f.store.delete_worktrees(&[one.id.clone(), two.id.clone()]).unwrap();
        assert!(f.store.worktree_for(&one.id).unwrap().is_none());
        assert!(f.store.worktree_for(&two.id).unwrap().is_none());
        assert!(f.store.worktree_for(&other.id).unwrap().is_some());
        assert_eq!(f.store.load_channel(&one.id).unwrap(), one, "the channel stays");
    }
}

#[cfg(test)]
mod agent_session_tests {
    use super::*;
    use crate::capability::ApprovalPolicy;
    use crate::model::{Block, Message, Role};
    use std::collections::BTreeMap;

    struct Fixture {
        store: Store,
        agent: AgentId,
        channel: Channel,
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let config = AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false, reasoning_effort: None,
            name: "Main".into(), avatar: None, provider: "claude".into(), model: Some("model".into()),
            system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
        };
        let agent = store.create_agent(&config).unwrap();
        let project = Project { id: ProjectId::new(), name: "P".into(), path: "/p".into() };
        store.insert_project(&project).unwrap();
        let channel = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        Fixture { store, agent, channel }
    }

    fn row(channel: &ChannelId, agent: &AgentId) -> AgentSession {
        AgentSession {
            channel_id: channel.clone(), agent_id: agent.clone(), handle: "handle-1".into(),
            fingerprint: r#"{"model":"m"}"#.into(), delivered_through: Some(MessageId::new()),
            delivered_memory: BTreeMap::from([("a".to_string(), 3), ("b".to_string(), 7)]),
            delivered_topic_version: Some(4), delivered_capture_paused: true, bytes_in_session: 1234,
            cwd: "/p".into(), dropped_reason: None,
        }
    }

    #[test]
    fn migration_16_adds_agent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v15.db");
        {
            let store = Store::open(&path).unwrap();
            store.conn.execute_batch("DROP TABLE agent_sessions; PRAGMA user_version=15;").unwrap();
        }
        let f = Store::open(&path).unwrap();
        assert_eq!(f.schema_version().unwrap(), 16);
        let count: i64 = f.conn.query_row("SELECT COUNT(*) FROM agent_sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn agent_session_round_trips_with_memory_map() {
        let f = fixture();
        assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_none());
        let saved = row(&f.channel.id, &f.agent);
        f.store.put_agent_session(&saved).unwrap();
        assert_eq!(f.store.agent_session(&f.channel.id, &f.agent).unwrap(), Some(saved.clone()));
        let changed = AgentSession { handle: "handle-2".into(), delivered_through: None, delivered_topic_version: None, delivered_capture_paused: false, ..saved };
        f.store.put_agent_session(&changed).unwrap();
        assert_eq!(f.store.agent_session(&f.channel.id, &f.agent).unwrap(), Some(changed), "put upserts");
        f.store.delete_agent_session(&f.channel.id, &f.agent).unwrap();
        assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_none());
    }

    #[test]
    fn drop_then_put_clears_the_reason() {
        let f = fixture();
        let saved = row(&f.channel.id, &f.agent);
        f.store.put_agent_session(&saved).unwrap();
        f.store.drop_agent_session(&f.channel.id, &f.agent, "model changed").unwrap();
        let dropped = f.store.agent_session(&f.channel.id, &f.agent).unwrap().unwrap();
        assert_eq!(dropped.dropped_reason.as_deref(), Some("model changed"));
        assert_eq!(dropped.handle, saved.handle, "the row keeps its handle");
        f.store.put_agent_session(&saved).unwrap();
        assert_eq!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().unwrap().dropped_reason, None);
    }

    #[test]
    fn deleting_a_channel_removes_its_rows() {
        let f = fixture();
        let anchor = Message { id: MessageId::new(), channel_id: f.channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "why?".into() }] };
        f.store.insert_message(&anchor).unwrap();
        let thread = f.store.create_thread(&f.channel.id, &anchor.id, "Why", &f.agent).unwrap();
        f.store.put_agent_session(&row(&thread.id, &f.agent)).unwrap();
        f.store.put_agent_session(&row(&f.channel.id, &f.agent)).unwrap();
        f.store.delete_thread(&thread.id).unwrap();
        assert!(f.store.agent_session(&thread.id, &f.agent).unwrap().is_none());
        assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_some(), "other channels keep theirs");
    }

    #[test]
    fn archiving_a_channel_removes_its_rows() {
        let f = fixture();
        f.store.put_agent_session(&row(&f.channel.id, &f.agent)).unwrap();
        f.store.archive_channel(&f.channel.id).unwrap();
        assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_none());
    }

    #[test]
    fn deleting_an_agent_removes_its_rows() {
        for delete_conversations in [true, false] {
            let f = fixture();
            f.store.put_agent_session(&row(&f.channel.id, &f.agent)).unwrap();
            f.store.delete_agent(&f.agent, DeleteAgentOptions { delete_conversations, delete_private_memory: false }).unwrap();
            assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_none(), "delete_conversations={delete_conversations}");
        }
    }

    fn said(f: &Fixture, role: Role, text: &str) -> Message {
        let message = Message { id: MessageId::new(), channel_id: f.channel.id.clone(), role, blocks: vec![Block::Markdown { text: text.into() }] };
        f.store.insert_message(&message).unwrap();
        message
    }

    #[test]
    fn clearing_context_ends_sessions_marks_the_transcript_and_cuts_replay() {
        let f = fixture();
        let other_agent = f.store.create_agent(&f.store.load_agent(&f.agent).unwrap()).unwrap();
        let other_channel = f.store.create_conversation(&f.agent, None, "dm").unwrap();
        for (channel, agent) in [(&f.channel.id, &f.agent), (&f.channel.id, &other_agent), (&other_channel.id, &f.agent)] {
            f.store.put_agent_session(&row(channel, agent)).unwrap();
        }
        said(&f, Role::User, "old question");
        let old = said(&f, Role::Agent, "old answer");
        f.store.set_conversation_context(&f.channel.id, &crate::conversation_context::ConversationContext {
            provider: Some("claude".into()), model: Some("model".into()), summary: Some("s".into()), through: Some(old.id.clone()), reset_from: None,
        }).unwrap();

        let marker = f.store.reset_channel_context(&f.channel.id).unwrap();
        let fresh = said(&f, Role::User, "new question");

        assert_eq!(marker.role, Role::System);
        assert!(matches!(&marker.blocks[..], [Block::Markdown { text }] if text == crate::conversation_context::CONTEXT_CLEARED));
        assert!(f.store.agent_session(&f.channel.id, &f.agent).unwrap().is_none());
        assert!(f.store.agent_session(&f.channel.id, &other_agent).unwrap().is_none(), "every agent loses its session");
        assert!(f.store.agent_session(&other_channel.id, &f.agent).unwrap().is_some(), "other channels keep theirs");
        let ids: Vec<_> = f.store.messages_for_channel(&f.channel.id).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 4, "the transcript keeps every message");
        assert_eq!(ids[2], marker.id);
        let history = f.store.channel_history(&f.channel.id).unwrap();
        assert_eq!(history.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![fresh.id.clone()], "replay starts after the marker");
        let state = f.store.conversation_context(&f.channel.id).unwrap();
        assert_eq!((state.summary, state.through, state.reset_from), (None, None, Some(marker.id.clone())));
        assert_eq!((state.provider.as_deref(), state.model.as_deref()), (Some("claude"), Some("model")));
        let second = f.store.reset_channel_context(&f.channel.id).unwrap();
        assert_eq!(f.store.conversation_context(&f.channel.id).unwrap().reset_from, Some(second.id));
        assert!(f.store.channel_history(&f.channel.id).unwrap().messages.is_empty());
    }

    #[test]
    fn a_thread_after_a_cleared_parent_does_not_replay_the_cleared_messages() {
        let f = fixture();
        said(&f, Role::User, "before clear");
        f.store.reset_channel_context(&f.channel.id).unwrap();
        let anchor = said(&f, Role::Agent, "after clear");
        let thread = f.store.create_thread(&f.channel.id, &anchor.id, "T", &f.agent).unwrap();
        let history = f.store.channel_history(&thread.id).unwrap();
        assert_eq!(history.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![anchor.id]);
    }
}
