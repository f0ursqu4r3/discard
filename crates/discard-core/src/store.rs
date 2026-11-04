//! SQLite store. Discard's own history is the source of truth for the UI.
//! The provider's native session ID is kept only as a resume handle.

use crate::agent::AgentConfig;
use crate::approval::{Approval, ApprovalState, AuditRecord, Decider, Decision};
use crate::event::{ApprovalId, Event, EventId, EventKind, SessionId, TurnId};
use crate::model::{
    AgentId, Block, Channel, ChannelId, Message, MessageId, Project, ProjectId, Role, WorkspaceMode,
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
CREATE TABLE agents (id TEXT PRIMARY KEY, config_toml TEXT NOT NULL);
CREATE TABLE channels (
    ord INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    workspace_mode TEXT NOT NULL
);
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

/// A channel's git worktree, as recorded when it was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRecord {
    pub channel_id: ChannelId,
    pub path: String,
    pub branch: String,
    pub base_commit: String,
    pub repo: String,
}

pub struct Store {
    conn: Connection,
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
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, name, path) VALUES (?1, ?2, ?3)",
            params![p.id.0, p.name, p.path],
        )?;
        Ok(())
    }

    pub fn insert_agent(&self, id: &AgentId, cfg: &AgentConfig) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agents (id, config_toml) VALUES (?1, ?2)",
            params![id.0, cfg.to_toml()?],
        )?;
        Ok(())
    }

    pub fn load_agent(&self, id: &AgentId) -> Result<AgentConfig> {
        let text: Option<String> = self
            .conn
            .query_row("SELECT config_toml FROM agents WHERE id = ?1", params![id.0], |r| r.get(0))
            .optional()?;
        let text = text.ok_or_else(|| StoreError::NotFound(format!("agent {}", id.0)))?;
        Ok(AgentConfig::from_toml(&text)?)
    }

    pub fn list_agents(&self) -> Result<Vec<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare("SELECT id, config_toml FROM agents ORDER BY rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut agents = Vec::new();
        for row in rows {
            let (id, config) = row?;
            agents.push((AgentId(id), AgentConfig::from_toml(&config)?));
        }
        Ok(agents)
    }

    /// The first agent that a channel of `project` uses (archived channels count) with this provider and
    /// `plugin_write` value. Agents are never shared between projects: agent-private memory is keyed by agent.
    pub fn find_project_agent(&self, project: &ProjectId, provider: &str, plugin_write: bool) -> Result<Option<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.config_toml FROM channels c JOIN agents a ON a.id = c.agent_id WHERE c.project_id = ?1 ORDER BY c.ord",
        )?;
        let rows = statement.query_map(params![project.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
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
        let updated = self.conn.execute("UPDATE agents SET config_toml = ?1 WHERE id = ?2", params![cfg.to_toml()?, id.0])?;
        if updated == 0 { return Err(StoreError::NotFound(format!("agent {}", id.0))); }
        Ok(())
    }

    pub fn insert_channel(&self, c: &Channel) -> Result<()> {
        self.conn.execute(
            "INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.id.0, c.project_id.0, c.name, c.agent_id.0, c.workspace_mode.as_str()],
        )?;
        Ok(())
    }

    pub fn channels_for_project(&self, p: &ProjectId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, agent_id, workspace_mode FROM channels WHERE project_id = ?1 AND archived = 0 ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![p.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, project_id, name, agent_id, mode) = row?;
            let workspace_mode = WorkspaceMode::parse(&mode)
                .ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
            out.push(Channel { id: ChannelId(id), project_id: ProjectId(project_id), name, agent_id: AgentId(agent_id), workspace_mode });
        }
        Ok(out)
    }

    pub fn save_worktree(&self, w: &WorktreeRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO worktrees (channel_id, path, branch, base_commit, repo) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(channel_id) DO UPDATE SET path = excluded.path, branch = excluded.branch,
               base_commit = excluded.base_commit, repo = excluded.repo",
            params![w.channel_id.0, w.path, w.branch, w.base_commit, w.repo],
        )?;
        Ok(())
    }

    pub fn worktree_for(&self, channel: &ChannelId) -> Result<Option<WorktreeRecord>> {
        let row = self
            .conn
            .query_row(
                "SELECT path, branch, base_commit, repo FROM worktrees WHERE channel_id = ?1",
                params![channel.0],
                |r| Ok(WorktreeRecord { channel_id: channel.clone(), path: r.get(0)?, branch: r.get(1)?, base_commit: r.get(2)?, repo: r.get(3)? }),
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_worktree(&self, channel: &ChannelId) -> Result<()> {
        self.conn.execute("DELETE FROM worktrees WHERE channel_id = ?1", params![channel.0])?;
        Ok(())
    }

    pub fn set_workspace_mode(&self, channel: &ChannelId, mode: WorkspaceMode) -> Result<()> {
        let n = self.conn.execute("UPDATE channels SET workspace_mode = ?2 WHERE id = ?1", params![channel.0, mode.as_str()])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
        Ok(())
    }

    /// Hide a channel from its project. Its history stays in the database.
    pub fn archive_channel(&self, channel: &ChannelId) -> Result<()> {
        let n = self.conn.execute("UPDATE channels SET archived = 1 WHERE id = ?1", params![channel.0])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("channel {}", channel.0)));
        }
        Ok(())
    }

    pub fn project_by_path(&self, path: &str) -> Result<Option<Project>> {
        let row = self
            .conn
            .query_row("SELECT id, name, path FROM projects WHERE path = ?1", params![path], |r| {
                Ok(Project { id: ProjectId(r.get(0)?), name: r.get(1)?, path: r.get(2)? })
            })
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
        let n = self.conn.execute("UPDATE sessions SET resume_handle = ?2 WHERE id = ?1", params![id.0, handle])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("session {}", id.0)));
        }
        Ok(())
    }

    pub fn resume_handle(&self, id: &SessionId) -> Result<Option<String>> {
        let row: Option<Option<String>> = self
            .conn
            .query_row("SELECT resume_handle FROM sessions WHERE id = ?1", params![id.0], |r| r.get(0))
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
            .query_row("SELECT state FROM turns WHERE id = ?1", params![id.0], |r| r.get(0))
            .optional()?;
        let s = s.ok_or_else(|| StoreError::NotFound(format!("turn {}", id.0)))?;
        TurnState::parse(&s).ok_or_else(|| StoreError::Corrupt(format!("turn state {s}")))
    }

    pub fn update_turn_state(&self, id: &TurnId, to: TurnState) -> Result<()> {
        let from = self.turn_state(id)?;
        if !from.can_transition_to(to) {
            return Err(StoreError::InvalidTransition { from, to });
        }
        self.conn.execute("UPDATE turns SET state = ?2 WHERE id = ?1", params![id.0, to.as_str()])?;
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
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
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
        let (turn_id, tool, input, state) = row.ok_or_else(|| StoreError::NotFound(format!("approval {}", id.0)))?;
        let state = parse_approval_state(&state).ok_or_else(|| StoreError::Corrupt(format!("approval state {state}")))?;
        let mut stmt = self.conn.prepare(
            "SELECT decision, decider_json, policy FROM approval_audit WHERE approval_id = ?1 ORDER BY n",
        )?;
        let rows = stmt.query_map(params![id.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut audit = Vec::new();
        for row in rows {
            let (decision, decider, policy) = row?;
            audit.push(AuditRecord {
                approval_id: id.clone(),
                decision: parse_decision(&decision).ok_or_else(|| StoreError::Corrupt(format!("decision {decision}")))?,
                decider: serde_json::from_str::<Decider>(&decider)?,
                policy,
            });
        }
        Ok(Approval { id: id.clone(), turn_id: TurnId(turn_id), tool, input: serde_json::from_str(&input)?, state, audit })
    }

    pub fn insert_message(&self, m: &Message) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, channel_id, role, blocks_json) VALUES (?1, ?2, ?3, ?4)",
            params![m.id.0, m.channel_id.0, m.role.as_str(), serde_json::to_string(&m.blocks)?],
        )?;
        Ok(())
    }

    pub fn messages_for_channel(&self, c: &ChannelId) -> Result<Vec<Message>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, role, blocks_json FROM messages WHERE channel_id = ?1 ORDER BY ord")?;
        let rows = stmt.query_map(params![c.0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, role, blocks) = row?;
            out.push(Message {
                id: MessageId(id),
                channel_id: c.clone(),
                role: Role::parse(&role).ok_or_else(|| StoreError::Corrupt(format!("role {role}")))?,
                blocks: serde_json::from_str::<Vec<Block>>(&blocks)?,
            });
        }
        Ok(out)
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
            tx.execute("UPDATE turns SET state = 'failed' WHERE id = ?1", params![turn_id])?;
            let pending: Vec<String> = {
                let mut stmt = tx.prepare("SELECT id FROM approvals WHERE turn_id = ?1 AND state = 'pending' ORDER BY id")?;
                let rows = stmt.query_map(params![turn_id], |r| r.get(0))?;
                rows.collect::<std::result::Result<_, _>>()?
            };
            for approval_id in pending {
                tx.execute("UPDATE approvals SET state = 'stale' WHERE id = ?1", params![approval_id])?;
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
        let project = Project { id: ProjectId::new(), name: "demo".into(), path: "/tmp/demo".into() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store
            .insert_agent(
                &agent,
                &AgentConfig {
                    name: "Scout".into(),
                    avatar: None,
                    provider: "claude".into(),
                    model: None,
                    system_prompt: String::new(),
                    policy: ApprovalPolicy::Ask, plugin_write: false,
                },
            )
            .unwrap();
        let channel = Channel {
            id: ChannelId::new(),
            project_id: project.id.clone(),
            name: "general".into(),
            agent_id: agent.clone(),
            workspace_mode: WorkspaceMode::MainCheckout,
        };
        store.insert_channel(&channel).unwrap();
        let session = SessionId::new();
        store.create_session(&session, &channel.id).unwrap();
        Fixture { store, project, agent, channel, session }
    }

    #[test]
    fn list_agents_returns_agents_in_insert_order() {
        let f = fixture();
        let mut config = f.store.load_agent(&f.agent).unwrap();
        let second = AgentId::new();
        config.plugin_write = true;
        f.store.insert_agent(&second, &config).unwrap();
        let agents = f.store.list_agents().unwrap();
        assert_eq!(agents.iter().map(|(id, _)| id).collect::<Vec<_>>(), vec![&f.agent, &second]);
    }

    #[test]
    fn find_project_agent_matches_provider_and_plugin_write_within_the_project_only() {
        let f = fixture();
        let mut builder = f.store.load_agent(&f.agent).unwrap();
        builder.plugin_write = true;
        let builder_id = AgentId::new();
        f.store.insert_agent(&builder_id, &builder).unwrap();
        f.store
            .insert_channel(&Channel { id: ChannelId::new(), project_id: f.project.id.clone(), name: "b".into(), agent_id: builder_id.clone(), workspace_mode: WorkspaceMode::MainCheckout })
            .unwrap();
        let found = |provider, plugin_write| f.store.find_project_agent(&f.project.id, provider, plugin_write).unwrap().map(|(id, _)| id);
        assert_eq!(found("claude", false), Some(f.agent.clone()));
        assert_eq!(found("claude", true), Some(builder_id));
        assert_eq!(found("unknown", false), None);
        // An agent that no channel of the project uses is never found, and neither is another project's agent.
        assert!(f.store.find_project_agent(&ProjectId::new(), "claude", false).unwrap().is_none());
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
        assert!(matches!(f.store.update_agent(&AgentId::new(), &config), Err(StoreError::NotFound(_))));
    }

    fn stamp(seq: &mut Sequencer, s: &SessionId, text: &str) -> Event {
        seq.stamp(s, RawEvent { kind: EventKind::TextDelta { text: text.into() }, raw: Some(json!({"v": 1})) })
    }

    #[test]
    fn migration_sets_schema_version_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        let v1 = Store::open(&path).unwrap().schema_version().unwrap();
        let v2 = Store::open(&path).unwrap().schema_version().unwrap();
        assert!(v1 >= 2);
        assert_eq!(v1, v2);
    }

    #[test]
    fn lookups_find_the_project_channel_and_session() {
        let f = fixture();
        assert_eq!(f.store.project_by_path("/tmp/demo").unwrap(), Some(f.project.clone()));
        assert_eq!(f.store.project_by_path("/nope").unwrap(), None);
        assert_eq!(f.store.first_channel(&f.project.id).unwrap(), Some(f.channel.clone()));
        assert_eq!(f.store.latest_session(&f.channel.id).unwrap(), Some(f.session.clone()));
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
            conn.execute("INSERT INTO projects (id, name, path) VALUES ('p', 'old', '/old')", []).unwrap();
            conn.execute("INSERT INTO agents (id, config_toml) VALUES ('a', 'name=\"A\"\nprovider=\"claude\"\npolicy=\"ask\"')", []).unwrap();
            conn.execute("INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES ('c', 'p', 'general', 'a', 'main_checkout')", []).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
        let channels = store.channels_for_project(&ProjectId("p".into())).unwrap();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].workspace_mode, WorkspaceMode::MainCheckout, "old channels stay in the main checkout");
        assert_eq!(store.worktree_for(&ChannelId("c".into())).unwrap(), None);
    }

    #[test]
    fn worktree_records_round_trip_update_and_delete() {
        let f = fixture();
        let rec = WorktreeRecord { channel_id: f.channel.id.clone(), path: "/w/x".into(), branch: "discard/x".into(), base_commit: "abc".into(), repo: "/r".into() };
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), None);
        f.store.save_worktree(&rec).unwrap();
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), Some(rec.clone()));
        let moved = WorktreeRecord { path: "/w/y".into(), ..rec };
        f.store.save_worktree(&moved).unwrap();
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), Some(moved));
        f.store.delete_worktree(&f.channel.id).unwrap();
        assert_eq!(f.store.worktree_for(&f.channel.id).unwrap(), None);
    }

    #[test]
    fn the_workspace_mode_can_be_changed_and_the_change_persists() {
        let f = fixture();
        f.store.set_workspace_mode(&f.channel.id, WorkspaceMode::Worktree).unwrap();
        assert_eq!(f.store.channels_for_project(&f.project.id).unwrap()[0].workspace_mode, WorkspaceMode::Worktree);
        assert!(matches!(f.store.set_workspace_mode(&ChannelId::new(), WorkspaceMode::Worktree), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn archived_channels_disappear_from_the_project_but_keep_their_history() {
        let f = fixture();
        let m = Message { id: MessageId::new(), channel_id: f.channel.id.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "hi".into() }] };
        f.store.insert_message(&m).unwrap();
        f.store.archive_channel(&f.channel.id).unwrap();
        assert!(f.store.channels_for_project(&f.project.id).unwrap().is_empty());
        assert_eq!(f.store.first_channel(&f.project.id).unwrap(), None);
        assert_eq!(f.store.messages_for_channel(&f.channel.id).unwrap().len(), 1);
        assert!(matches!(f.store.archive_channel(&ChannelId::new()), Err(StoreError::NotFound(_))));
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
        assert_eq!(f.store.events_for_session(&f.session).unwrap(), vec![e1, e2]);
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
        f.store.append_event(&stamp(&mut seq, &f.session, "a")).unwrap();
        f.store.append_event(&stamp(&mut seq, &f.session, "b")).unwrap();
        assert_eq!(f.store.last_seq(&f.session).unwrap(), 2);
    }

    #[test]
    fn resume_handle_round_trips_and_can_be_cleared() {
        let f = fixture();
        assert_eq!(f.store.resume_handle(&f.session).unwrap(), None);
        f.store.set_resume_handle(&f.session, Some("claude-abc")).unwrap();
        assert_eq!(f.store.resume_handle(&f.session).unwrap().as_deref(), Some("claude-abc"));
        f.store.set_resume_handle(&f.session, None).unwrap();
        assert_eq!(f.store.resume_handle(&f.session).unwrap(), None);
    }

    #[test]
    fn turn_updates_validate_transitions() {
        let f = fixture();
        let t = Turn::new(f.session.clone());
        f.store.insert_turn(&t).unwrap();
        f.store.update_turn_state(&t.id, TurnState::Running).unwrap();
        let err = f.store.update_turn_state(&t.id, TurnState::Queued).unwrap_err();
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
        assert_eq!(f.store.load_approval(&a.id).unwrap().state, ApprovalState::Pending);
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
                Block::Code { language: Some("rust".into()), text: "fn main(){}".into() },
                Block::Approval { approval_id: "a1".into(), tool: "Bash".into(), summary: "touch x".into() },
            ],
        };
        f.store.insert_message(&m1).unwrap();
        f.store.insert_message(&m2).unwrap();
        assert_eq!(f.store.messages_for_channel(&f.channel.id).unwrap(), vec![m1, m2]);
    }

    #[test]
    fn reconcile_fails_orphaned_turns_and_stales_their_approvals() {
        let f = fixture();
        let orphan = Turn::new(f.session.clone());
        f.store.insert_turn(&orphan).unwrap();
        f.store.update_turn_state(&orphan.id, TurnState::Running).unwrap();
        f.store.update_turn_state(&orphan.id, TurnState::AwaitingApproval).unwrap();
        let pending = Approval::new(orphan.id.clone(), "Bash", json!({}));
        f.store.save_approval(&pending).unwrap();

        let mut decided = Approval::new(orphan.id.clone(), "Read", json!({}));
        decided.decide(Decision::Approve, Decider::User, "ask").unwrap();
        f.store.save_approval(&decided).unwrap();

        let done = Turn::new(f.session.clone());
        f.store.insert_turn(&done).unwrap();
        f.store.update_turn_state(&done.id, TurnState::Running).unwrap();
        f.store.update_turn_state(&done.id, TurnState::Completed).unwrap();

        let report = f.store.reconcile(&HashSet::new()).unwrap();
        assert_eq!(report.turns_failed, vec![orphan.id.clone()]);
        assert_eq!(report.approvals_staled, vec![pending.id.clone()]);
        assert_eq!(f.store.turn_state(&orphan.id).unwrap(), TurnState::Failed);
        assert_eq!(f.store.turn_state(&done.id).unwrap(), TurnState::Completed);
        assert_eq!(f.store.load_approval(&pending.id).unwrap().state, ApprovalState::Stale);
        assert_eq!(f.store.load_approval(&decided.id).unwrap().state, ApprovalState::Decided(Decision::Approve));
    }

    #[test]
    fn reconcile_leaves_turns_of_live_sessions_alone() {
        let f = fixture();
        let t = Turn::new(f.session.clone());
        f.store.insert_turn(&t).unwrap();
        f.store.update_turn_state(&t.id, TurnState::Running).unwrap();
        let live: HashSet<SessionId> = [f.session.clone()].into();
        assert_eq!(f.store.reconcile(&live).unwrap(), ReconcileReport::default());
        assert_eq!(f.store.turn_state(&t.id).unwrap(), TurnState::Running);
    }

    #[test]
    fn data_survives_reopening_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        let session = SessionId::new();
        {
            let store = Store::open(&path).unwrap();
            let project = Project { id: ProjectId::new(), name: "p".into(), path: "/p".into() };
            store.insert_project(&project).unwrap();
            let agent = AgentId::new();
            store
                .insert_agent(
                    &agent,
                    &AgentConfig {
                        name: "A".into(),
                        avatar: None,
                        provider: "claude".into(),
                        model: None,
                        system_prompt: String::new(),
                        policy: ApprovalPolicy::DenyAll, plugin_write: false,
                    },
                )
                .unwrap();
            let ch = Channel {
                id: ChannelId::new(),
                project_id: project.id,
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
