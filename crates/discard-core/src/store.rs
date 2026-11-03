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

    pub fn insert_channel(&self, c: &Channel) -> Result<()> {
        self.conn.execute(
            "INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.id.0, c.project_id.0, c.name, c.agent_id.0, c.workspace_mode.as_str()],
        )?;
        Ok(())
    }

    pub fn channels_for_project(&self, p: &ProjectId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, agent_id, workspace_mode FROM channels WHERE project_id = ?1 ORDER BY ord",
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
                    policy: ApprovalPolicy::Ask,
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

    fn stamp(seq: &mut Sequencer, s: &SessionId, text: &str) -> Event {
        seq.stamp(s, RawEvent { kind: EventKind::TextDelta { text: text.into() }, raw: Some(json!({"v": 1})) })
    }

    #[test]
    fn migration_sets_schema_version_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.db");
        let v1 = Store::open(&path).unwrap().schema_version().unwrap();
        let v2 = Store::open(&path).unwrap().schema_version().unwrap();
        assert!(v1 >= 1);
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
                        policy: ApprovalPolicy::DenyAll,
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
