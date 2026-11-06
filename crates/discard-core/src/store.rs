//! SQLite store. Discard's own history is the source of truth for the UI.
//! The provider's native session ID is kept only as a resume handle.

use crate::agent::AgentConfig;
use crate::approval::{Approval, ApprovalState, AuditRecord, Decider, Decision};
use crate::event::{ApprovalId, Event, EventId, EventKind, SessionId, TurnId};
use crate::model::{
    AgentId, Block, Channel, ChannelId, ChannelTopic, ChannelTopicRevision, Message, MessageId,
    Project, ProjectChannelKind, ProjectChannelMetadata, ProjectId, Role, TopicAuthorKind,
    WorkspaceMode,
};
use crate::turn::{Turn, TurnState};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::path::Path;

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
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
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
            let mut groups: std::collections::HashMap<(String, String), Vec<(String, bool, i64)>> =
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
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS direct_message_list (agent_id TEXT PRIMARY KEY REFERENCES agents(id));")?;
        Ok(())
    }

    fn require_active_agent(&self, agent: &AgentId) -> Result<()> {
        if self.agent_archived(agent)? {
            return Err(StoreError::Invalid("Restore the agent first.".into()));
        }
        Ok(())
    }

    fn reconcile_general_agent(
        conn: &Connection,
        project: &ProjectId,
        agent: &AgentId,
    ) -> Result<()> {
        let channel: Option<(String, String, bool)> = conn
            .query_row(
                "SELECT pc.channel_id,c.agent_id,a.deleted FROM project_channels pc
                 JOIN channels c ON c.id=pc.channel_id JOIN agents a ON a.id=c.agent_id
                 WHERE pc.project_id=?1 AND pc.kind='general' AND c.archived=0",
                params![project.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((channel, current_agent, current_agent_deleted)) = channel else {
            return Ok(());
        };
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
        Ok(())
    }

    pub fn project_channel_metadata(
        &self,
        channel: &ChannelId,
    ) -> Result<Option<ProjectChannelMetadata>> {
        let row = self.conn.query_row("SELECT project_id,kind,branch_name,archived_into FROM project_channels WHERE channel_id=?1", params![channel.0], |r| {
            Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?))
        }).optional()?;
        let Some((project, kind, branch, archived_into)) = row else {
            return Ok(None);
        };
        let kind = ProjectChannelKind::parse(&kind)
            .ok_or_else(|| StoreError::Corrupt(format!("project channel kind {kind}")))?;
        Ok(Some(ProjectChannelMetadata {
            project_id: ProjectId(project),
            channel_id: channel.clone(),
            kind,
            branch,
            archived_into: archived_into.map(ChannelId),
        }))
    }

    /// Find the active canonical channel for a project branch.
    pub fn project_branch_channel(&self, project: &ProjectId, branch: &str) -> Result<Option<Channel>> {
        let id: Option<String> = self.conn.query_row(
            "SELECT pc.channel_id FROM project_channels pc JOIN channels c ON c.id=pc.channel_id
             WHERE pc.project_id=?1 AND pc.kind='branch' AND pc.branch_name=?2 AND c.archived=0",
            params![project.0, branch],
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
                 WHERE c.project_id=?1 AND c.archived=0 AND lower(trim(c.name))='general' AND pc.channel_id IS NULL
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
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND project_id=?2 AND archived=0)",
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
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND project_id=?2 AND archived=0)",
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
             WHERE c.id=?1 AND c.project_id=?2 AND lower(trim(c.name))='general'
             AND (c.archived=0 OR pc.kind='archived_legacy') AND (pc.channel_id IS NULL OR pc.kind!='branch')",
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
        let valid: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM channels c LEFT JOIN project_channels pc ON pc.channel_id=c.id
            WHERE c.id=?1 AND c.project_id=?2 AND (pc.channel_id IS NULL OR (pc.kind='archived_legacy' AND pc.branch_name=?3) OR (pc.kind='branch' AND pc.branch_name=?3)))",params![channel.0,project.0,branch],|r|r.get(0))?;
        if !valid {
            return Err(StoreError::Invalid(
                "Select a channel from this project that belongs to this branch.".into(),
            ));
        }
        tx.execute("UPDATE project_channels SET kind='archived_legacy',archived_into=?3 WHERE project_id=?1 AND kind='branch' AND branch_name=?2 AND channel_id!=?3",params![project.0,branch,channel.0])?;
        tx.execute("UPDATE channels SET archived=1 WHERE id IN (SELECT channel_id FROM project_channels WHERE project_id=?1 AND kind='archived_legacy' AND branch_name=?2 AND archived_into=?3)",params![project.0,branch,channel.0])?;
        tx.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name) VALUES(?1,?2,'branch',?3)
            ON CONFLICT(channel_id) DO UPDATE SET project_id=excluded.project_id,kind='branch',branch_name=excluded.branch_name,archived_into=NULL",params![channel.0,project.0,branch])?;
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
                     WHERE project_id=?1 AND kind='branch' AND branch_name=?2 AND channel_id!=?3",
                    params![project, branch, channel.0],
                )?;
                self.conn.execute(
                    "UPDATE channels SET archived=1 WHERE id IN (
                        SELECT channel_id FROM project_channels
                        WHERE project_id=?1 AND kind='archived_legacy' AND branch_name=?2 AND archived_into=?3
                    )",
                    params![project, branch, channel.0],
                )?;
                self.conn.execute("INSERT INTO project_channels(channel_id,project_id,kind,branch_name) VALUES(?1,?2,'branch',?3)
                    ON CONFLICT(channel_id) DO UPDATE SET project_id=excluded.project_id,kind='branch',branch_name=excluded.branch_name,archived_into=NULL",params![channel.0,project,branch])?;
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
        self.conn.execute(
            "INSERT OR IGNORE INTO hidden_projects VALUES (?1)",
            params![id.0],
        )?;
        self.conn.execute(
            "DELETE FROM project_selection WHERE project_id=?1",
            params![id.0],
        )?;
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
            for channel in channels {
                for sql in [
                    "DELETE FROM project_last_channels WHERE channel_id=?1",
                    "DELETE FROM project_channel_views WHERE channel_id=?1",
                    "DELETE FROM project_channels WHERE channel_id=?1",
                    "DELETE FROM channel_topic_revisions WHERE channel_id=?1",
                    "DELETE FROM channel_agents WHERE channel_id=?1",
                ] {
                    tx.execute(sql, params![channel.id.0])?;
                }
                for sql in [
                    "DELETE FROM approval_audit WHERE approval_id IN (SELECT a.id FROM approvals a JOIN turns t ON a.turn_id=t.id JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
                    "DELETE FROM approvals WHERE turn_id IN (SELECT t.id FROM turns t JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
                    "DELETE FROM turns WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
                    "DELETE FROM events WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
                    "DELETE FROM conversation_context WHERE channel_id=?1",
                    "DELETE FROM response_memory WHERE channel_id=?1",
                    "DELETE FROM sessions WHERE channel_id=?1",
                    "DELETE FROM messages WHERE channel_id=?1",
                    "DELETE FROM worktrees WHERE channel_id=?1",
                    "DELETE FROM channel_branches WHERE channel_id=?1",
                    "DELETE FROM channels WHERE id=?1",
                ] {
                    self.conn.execute(sql, params![channel.id.0])?;
                }
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
        // Keep a minimal tombstone so retained history still has a readable author.
        let tombstone = AgentConfig {
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

    /// All saved conversations, including retained histories of deleted agents.
    pub fn all_conversations(&self) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare("SELECT id FROM channels ORDER BY ord")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| self.load_channel(&ChannelId(id)))
            .collect()
    }

    pub fn conversations_for_agent(&self, agent: &AgentId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,project_id,name,workspace_mode FROM channels WHERE agent_id=?1 ORDER BY ord",
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
            "SELECT a.id, a.config_toml FROM channels c JOIN agents a ON a.id = c.agent_id WHERE c.project_id = ?1 AND a.archived=0 AND a.deleted=0 ORDER BY c.ord",
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
        let tx = self.conn.unchecked_transaction()?;
        self.insert_channel(channel)?;
        self.create_session(&SessionId::new(), &channel.id)?;
        if let Some(branch) = branch {
            self.bind_branch(&channel.id, branch)?;
        }
        if let Some(worktree) = worktree {
            self.save_worktree(worktree)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn channels_for_project(&self, p: &ProjectId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, agent_id, workspace_mode FROM channels WHERE project_id = ?1 AND archived = 0 ORDER BY ord",
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
        let mut stmt = self.conn.prepare("SELECT id, project_id, name, agent_id, workspace_mode, archived FROM channels WHERE project_id = ?1 ORDER BY ord")?;
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
    pub fn archive_channel(&self, channel: &ChannelId) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE channels SET archived = 1 WHERE id = ?1",
            params![channel.0],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
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

    pub fn save_approval(&self, a: &Approval) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO approvals (id, turn_id, tool, input_json, state) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET state = excluded.state",
            params![a.id.0, a.turn_id.0, a.tool, serde_json::to_string(&a.input)?, approval_state_str(a.state)],
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
        let row: Option<(String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT turn_id, tool, input_json, state FROM approvals WHERE id = ?1",
                params![id.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (turn_id, tool, input, state) =
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
        {
            if let Some((id, _)) = self
                .response_memory(channel)?
                .into_iter()
                .find(|(_, c)| matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id))
            {
                target = id;
            }
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

    pub fn messages_for_channel(&self, c: &ChannelId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, blocks_json FROM messages WHERE channel_id = ?1 ORDER BY ord",
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

    /// Literal, case-insensitive search of saved direct message text, newest first.
    /// Includes archived and retained chats; never reads project conversations.
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
            "SELECT m.id, m.channel_id, c.name, m.role, m.blocks_json
             FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE c.agent_id = ?1 AND c.project_id IS NULL ORDER BY m.ord DESC",
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
        assert_eq!(v1, 5);
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
        assert_eq!(store.schema_version().unwrap(), 5);
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
                    approval_id: "a1".into(),
                    tool: "Bash".into(),
                    summary: "touch x".into(),
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
