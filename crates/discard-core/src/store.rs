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
pub enum PrivateMemoryScope { Agent(AgentId), AgentProject { agent: AgentId, project: ProjectId } }
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeleteAgentResult { pub private_scopes: Vec<PrivateMemoryScope> }

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
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS direct_message_list (agent_id TEXT PRIMARY KEY REFERENCES agents(id));")?;
        Ok(())
    }

    pub fn bind_branch(&self, channel: &ChannelId, branch: &str) -> Result<()> {
        self.conn.execute("INSERT INTO channel_branches (channel_id, branch) VALUES (?1, ?2)", params![channel.0, branch])?;
        Ok(())
    }
    pub fn channel_branch(&self, channel: &ChannelId) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT branch FROM channel_branches WHERE channel_id = ?1", params![channel.0], |r| r.get(0)).optional()?)
    }

    pub fn visible_projects(&self) -> Result<Vec<Project>> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS hidden_projects (project_id TEXT PRIMARY KEY REFERENCES projects(id));")?;
        let hidden: Vec<String> = self.conn.prepare("SELECT project_id FROM hidden_projects")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_,_>>()?;
        Ok(self.list_projects()?.into_iter().filter(|p| !hidden.contains(&p.id.0)).collect())
    }
    pub fn hide_project(&self, id: &ProjectId) -> Result<()> {
        self.visible_projects()?; self.selected_project()?;
        self.conn.execute("INSERT OR IGNORE INTO hidden_projects VALUES (?1)", params![id.0])?;
        self.conn.execute("DELETE FROM project_selection WHERE project_id=?1", params![id.0])?;
        Ok(())
    }
    pub fn selected_project(&self) -> Result<Option<Project>> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS project_selection (singleton INTEGER PRIMARY KEY CHECK(singleton=1), project_id TEXT NOT NULL REFERENCES projects(id));")?;
        Ok(self.conn.query_row("SELECT p.id,p.name,p.path FROM projects p JOIN project_selection s ON s.project_id=p.id WHERE s.singleton=1", [], |r| Ok(Project { id: ProjectId(r.get(0)?), name: r.get(1)?, path: r.get(2)? })).optional()?)
    }
    pub fn select_project(&self, project: &ProjectId) -> Result<()> {
        self.visible_projects()?;
        self.conn.execute("DELETE FROM hidden_projects WHERE project_id=?1", params![project.0])?;
        self.selected_project()?;
        self.conn.execute("INSERT INTO project_selection VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET project_id=excluded.project_id", params![project.0])?;
        Ok(())
    }
    pub fn import_project_folder(&self, path: &Path) -> Result<Project> {
        let canonical = path.canonicalize().map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if !canonical.is_dir() { return Err(StoreError::Corrupt("Select a folder.".into())); }
        for project in self.list_projects()? {
            if Path::new(&project.path).canonicalize().ok().as_ref() == Some(&canonical) { self.select_project(&project.id)?; return Ok(project); }
        }
        let project = Project { id: ProjectId::new(), name: canonical.file_name().unwrap_or_default().to_string_lossy().into_owned(), path: canonical.to_string_lossy().into_owned() };
        let tx = self.conn.unchecked_transaction()?;
        self.insert_project(&project)?;
        self.select_project(&project.id)?;
        tx.commit()?;
        Ok(project)
    }
    pub fn change_project_folder(&self, id: &ProjectId, path: &Path) -> Result<()> {
        let canonical = path.canonicalize().map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if !canonical.is_dir() { return Err(StoreError::Corrupt("Select a folder.".into())); }
        let projects = self.list_projects()?;
        let current = projects.iter().find(|p| p.id == *id).ok_or_else(|| StoreError::NotFound(id.0.clone()))?;
        if Path::new(&current.path).canonicalize().ok().as_ref() == Some(&canonical) { return Ok(()); }
        let destination = projects.iter().find(|p| p.id != *id && Path::new(&p.path).canonicalize().ok().as_ref() == Some(&canonical));
        if destination.is_some() { return Err(StoreError::Invalid("This folder belongs to another project.".into())); }
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute("UPDATE projects SET path=?2 WHERE id=?1", params![id.0, canonical.to_string_lossy()])?;
        // A folder change retains the project and conversation identities.
        for (channel, archived) in self.all_channels_for_project(id)? {
            if !archived {
                self.delete_worktree(&channel.id)?;
                self.conn.execute("DELETE FROM channel_branches WHERE channel_id=?1", params![channel.id.0])?;
                self.create_session(&SessionId::new(), &channel.id)?;
                if !crate::worktree::WorktreeManager::is_repo(&canonical) { self.set_workspace_mode(&channel.id, WorkspaceMode::MainCheckout)?; }
            }
        }
        self.select_project(id)?;
        tx.commit()?;
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

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut statement = self.conn.prepare("SELECT id, name, path FROM projects ORDER BY rowid")?;
        let rows = statement.query_map([], |r| Ok(Project { id: ProjectId(r.get(0)?), name: r.get(1)?, path: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn create_agent(&self, cfg: &AgentConfig) -> Result<AgentId> {
        cfg.validate().map_err(StoreError::Invalid)?;
        if cfg.model.is_none() { return Err(StoreError::Invalid("Select a model.".into())); }
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
        let result=(|| {
            let chat=self.create_conversation(agent,None,"General")?;
            self.conn.execute("UPDATE channels SET is_default=1 WHERE id=?1",params![chat.id.0])?;
            Ok(chat)
        })();
        match result {
            Ok(chat) => { self.conn.execute_batch("RELEASE default_conversation")?; Ok(chat) }
            Err(error) => { self.conn.execute_batch("ROLLBACK TO default_conversation; RELEASE default_conversation")?; Err(error) }
        }
    }

    pub fn default_conversation(&self, agent: &AgentId) -> Result<Channel> {
        let id: Option<String> = self.conn.query_row("SELECT id FROM channels WHERE agent_id=?1 AND is_default=1", params![agent.0], |r| r.get(0)).optional()?;
        self.conversations_for_agent(agent)?.into_iter().find(|c| Some(&c.id.0)==id.as_ref()).ok_or_else(||StoreError::NotFound("default conversation".into()))
    }

    /// DM list membership is independent of creating an agent or its empty default chat.
    pub fn add_direct_message(&self, agent: &AgentId) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO direct_message_list (agent_id) VALUES (?1)", params![agent.0])?;
        Ok(())
    }

    pub fn direct_message_agents(&self) -> Result<Vec<AgentId>> {
        let mut stmt = self.conn.prepare("SELECT agent_id FROM direct_message_list ORDER BY rowid")?;
        Ok(stmt.query_map([], |r| r.get::<_, String>(0).map(AgentId))?.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn set_agent_archived(&self, id: &AgentId, archived: bool) -> Result<()> {
        if self.conn.execute("UPDATE agents SET archived=?2 WHERE id=?1 AND deleted=0", params![id.0,archived])? == 0 { return Err(StoreError::NotFound(id.0.clone())); }
        Ok(())
    }

    pub fn agent_archived(&self, id: &AgentId) -> Result<bool> {
        self.conn.query_row("SELECT archived FROM agents WHERE id=?1 AND deleted=0",params![id.0],|r|r.get(0)).optional()?.ok_or_else(||StoreError::NotFound(id.0.clone()))
    }

    /// Include archived agents for Settings. Exclude deleted tombstones.
    pub fn all_agents(&self) -> Result<Vec<(AgentId, AgentConfig, bool)>> {
        let mut stmt=self.conn.prepare("SELECT id,config_toml,archived FROM agents WHERE deleted=0 ORDER BY rowid")?;
        let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?;
        rows.map(|r| { let (id,text,archived)=r?; Ok((AgentId(id),AgentConfig::from_toml(&text)?,archived)) }).collect()
    }

    pub fn delete_agent(&self, id: &AgentId, options: DeleteAgentOptions) -> Result<DeleteAgentResult> {
        self.agent_archived(id)?;
        let tx=self.conn.unchecked_transaction()?;
        let channels=self.conversations_for_agent(id)?;
        let mut result=DeleteAgentResult::default();
        if options.delete_private_memory {
            result.private_scopes.push(PrivateMemoryScope::Agent(id.clone()));
            let projects: HashSet<ProjectId> = channels.iter().filter_map(|c|c.project_id.clone()).collect();
            result.private_scopes.extend(projects.into_iter().map(|project|PrivateMemoryScope::AgentProject { agent:id.clone(),project }));
        }
        if options.delete_conversations {
            for channel in channels {
                for sql in [
                    "DELETE FROM approval_audit WHERE approval_id IN (SELECT a.id FROM approvals a JOIN turns t ON a.turn_id=t.id JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
                    "DELETE FROM approvals WHERE turn_id IN (SELECT t.id FROM turns t JOIN sessions s ON t.session_id=s.id WHERE s.channel_id=?1)",
                    "DELETE FROM turns WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
                    "DELETE FROM events WHERE session_id IN (SELECT id FROM sessions WHERE channel_id=?1)",
                    "DELETE FROM conversation_context WHERE channel_id=?1", "DELETE FROM response_memory WHERE channel_id=?1", "DELETE FROM sessions WHERE channel_id=?1", "DELETE FROM messages WHERE channel_id=?1", "DELETE FROM worktrees WHERE channel_id=?1", "DELETE FROM channel_branches WHERE channel_id=?1", "DELETE FROM channels WHERE id=?1"
                ] { self.conn.execute(sql,params![channel.id.0])?; }
            }
        }
        // Keep a minimal tombstone so retained history still has a readable author.
        let tombstone=AgentConfig { reasoning_effort: None, name:"Deleted agent".into(),avatar:None,provider:String::new(),model:None,system_prompt:String::new(),policy:crate::capability::ApprovalPolicy::DenyAll,plugin_write:false };
        self.conn.execute("UPDATE agents SET config_toml=?2,archived=1,deleted=1 WHERE id=?1",params![id.0,tombstone.to_toml()?])?;
        tx.commit()?;
        Ok(result)
    }

    pub fn create_conversation(&self, agent: &AgentId, project: Option<&ProjectId>, name: &str) -> Result<Channel> {
        if name.trim().is_empty() { return Err(StoreError::Invalid("Enter a conversation name.".into())); }
        if self.agent_archived(agent)? { return Err(StoreError::Invalid("Restore the agent first.".into())); }
        let channel=Channel { id:ChannelId::new(),project_id:project.cloned(),name:name.trim().into(),agent_id:agent.clone(),workspace_mode:WorkspaceMode::MainCheckout };
        // This may be called inside create_agent's transaction.
        // Use a savepoint so create_agent can own the outer transaction.
        self.conn.execute_batch("SAVEPOINT create_conversation")?;
        let result=(|| { self.insert_channel(&channel)?; self.create_session(&SessionId::new(),&channel.id)?; Ok(()) })();
        if let Err(error)=result { self.conn.execute_batch("ROLLBACK TO create_conversation; RELEASE create_conversation")?; return Err(error); }
        self.conn.execute_batch("RELEASE create_conversation")?;
        Ok(channel)
    }

    /// All saved conversations, including retained histories of deleted agents.
    pub fn all_conversations(&self) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare("SELECT id FROM channels ORDER BY ord")?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.load_channel(&ChannelId(id))).collect()
    }

    pub fn conversations_for_agent(&self, agent: &AgentId) -> Result<Vec<Channel>> {
        let mut stmt=self.conn.prepare("SELECT id,project_id,name,workspace_mode FROM channels WHERE agent_id=?1 ORDER BY ord")?;
        let rows=stmt.query_map(params![agent.0],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?;
        rows.map(|r| { let (id,project,name,mode)=r?; Ok(Channel { id:ChannelId(id),project_id:project.map(ProjectId),name,agent_id:agent.clone(),workspace_mode:WorkspaceMode::parse(&mode).ok_or_else(||StoreError::Corrupt(mode))? }) }).collect()
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
            .query_row("SELECT config_toml FROM agents WHERE id = ?1", params![id.0], |r| r.get(0))
            .optional()?;
        let text = text.ok_or_else(|| StoreError::NotFound(format!("agent {}", id.0)))?;
        Ok(AgentConfig::from_toml(&text)?)
    }

    pub fn list_agents(&self) -> Result<Vec<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare("SELECT id, config_toml FROM agents WHERE archived=0 AND deleted=0 ORDER BY rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut agents = Vec::new();
        for row in rows {
            let (id, config) = row?;
            agents.push((AgentId(id), AgentConfig::from_toml(&config)?));
        }
        Ok(agents)
    }

    /// The first agent that a channel of `project` uses (archived channels count) with this provider and
    /// `plugin_write` value. Bootstrap uses this lookup; user-created conversations select an agent ID.
    pub fn find_project_agent(&self, project: &ProjectId, provider: &str, plugin_write: bool) -> Result<Option<(AgentId, AgentConfig)>> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.config_toml FROM channels c JOIN agents a ON a.id = c.agent_id WHERE c.project_id = ?1 AND a.archived=0 AND a.deleted=0 ORDER BY c.ord",
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
        cfg.validate().map_err(StoreError::Invalid)?;
        if cfg.model.is_none() { return Err(StoreError::Invalid("Select a model.".into())); }
        let updated = self.conn.execute("UPDATE agents SET config_toml = ?1 WHERE id = ?2 AND deleted=0", params![cfg.to_toml()?, id.0])?;
        if updated == 0 { return Err(StoreError::NotFound(format!("agent {}", id.0))); }
        Ok(())
    }

    pub fn load_channel(&self, id: &ChannelId) -> Result<Channel> {
        let (project, name, agent, mode) = self.conn.query_row("SELECT project_id,name,agent_id,workspace_mode FROM channels WHERE id=?1", params![id.0], |r| {
            Ok((r.get::<_,Option<String>>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,String>(3)?))
        })?;
        let workspace_mode = WorkspaceMode::parse(&mode).ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
        Ok(Channel { id: id.clone(), project_id: project.map(ProjectId), name, agent_id: AgentId(agent), workspace_mode })
    }
    pub fn channel_archived(&self, id: &ChannelId) -> Result<bool> {
        Ok(self.conn.query_row("SELECT archived FROM channels WHERE id=?1", params![id.0], |r|r.get(0))?)
    }
    pub fn insert_channel(&self, c: &Channel) -> Result<()> {
        self.conn.execute(
            "INSERT INTO channels (id, project_id, name, agent_id, workspace_mode) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.id.0, c.project_id.as_ref().map(|id| &id.0), c.name, c.agent_id.0, c.workspace_mode.as_str()],
        )?;
        Ok(())
    }

    /// Persist a complete channel before starting its runtime. Roll back on any storage failure.
    pub fn create_named_channel(&self, channel: &Channel, branch: Option<&str>, worktree: Option<&WorktreeRecord>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.insert_channel(channel)?;
        self.create_session(&SessionId::new(), &channel.id)?;
        if let Some(branch) = branch { self.bind_branch(&channel.id, branch)?; }
        if let Some(worktree) = worktree { self.save_worktree(worktree)?; }
        tx.commit()?;
        Ok(())
    }

    pub fn channels_for_project(&self, p: &ProjectId) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, agent_id, workspace_mode FROM channels WHERE project_id = ?1 AND archived = 0 ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![p.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, project_id, name, agent_id, mode) = row?;
            let workspace_mode = WorkspaceMode::parse(&mode)
                .ok_or_else(|| StoreError::Corrupt(format!("workspace_mode {mode}")))?;
            out.push(Channel { id: ChannelId(id), project_id: project_id.map(ProjectId), name, agent_id: AgentId(agent_id), workspace_mode });
        }
        Ok(out)
    }

    pub fn all_channels_for_project(&self, p: &ProjectId) -> Result<Vec<(Channel, bool)>> {
        let mut stmt = self.conn.prepare("SELECT id, project_id, name, agent_id, workspace_mode, archived FROM channels WHERE project_id = ?1 ORDER BY ord")?;
        let rows = stmt.query_map(params![p.0], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?)))?;
        let mut out=Vec::new();
        for row in rows { let (id,project_id,name,agent_id,mode,archived)=row?; let workspace_mode=WorkspaceMode::parse(&mode).ok_or_else(||StoreError::Corrupt(format!("workspace_mode {mode}")))?; out.push((Channel { id:ChannelId(id),project_id:project_id.map(ProjectId),name,agent_id:AgentId(agent_id),workspace_mode },archived)); }
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
    pub fn restore_channel(&self, channel: &ChannelId) -> Result<()> {
        let row = self.load_channel(channel)?;
        if self.agent_archived(&row.agent_id)? { return Err(StoreError::Invalid("Restore the agent first.".into())); }
        self.conn.execute("UPDATE channels SET archived=0 WHERE id=?1", params![channel.0])?;
        Ok(())
    }
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

    pub fn conversation_context(&self, channel: &ChannelId) -> Result<crate::conversation_context::ConversationContext> {
        let json: Option<String> = self.conn.query_row("SELECT state_json FROM conversation_context WHERE channel_id=?1", params![channel.0], |r| r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?.unwrap_or_default())
    }
    pub fn set_conversation_context(&self, channel: &ChannelId, state: &crate::conversation_context::ConversationContext) -> Result<()> {
        self.conn.execute("INSERT INTO conversation_context VALUES (?1,?2) ON CONFLICT(channel_id) DO UPDATE SET state_json=excluded.state_json", params![channel.0, serde_json::to_string(state)?])?;
        Ok(())
    }
    pub fn response_memory(&self, channel: &ChannelId) -> Result<Vec<(MessageId, crate::memory_items::MemoryChange)>> {
        let mut stmt=self.conn.prepare("SELECT response_id,change_json FROM response_memory WHERE channel_id=?1 ORDER BY rowid")?;
        let rows=stmt.query_map(params![channel.0], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;
        rows.map(|row| {let (id,json)=row?; Ok((MessageId(id),serde_json::from_str(&json)?))}).collect()
    }
    pub fn record_response_memory(&self, channel: &ChannelId, response: &MessageId, change: &crate::memory_items::MemoryChange) -> Result<bool> {
        Ok(self.record_response_memory_at(channel, response, change)?.is_some())
    }
    /// Prefer a proposal's original response over a later turn that observes its approval.
    pub fn record_response_memory_at(&self, channel: &ChannelId, response: &MessageId, change: &crate::memory_items::MemoryChange) -> Result<Option<MessageId>> {
        use crate::memory_items::MemoryChange;
        let mut target=response.clone();
        if let MemoryChange::Saved {item,..}=change && item.origin==crate::memory_items::MutationOrigin::Approved && let Some(proposal_id)=&item.proposal_id {
            if let Some((id,_))=self.response_memory(channel)?.into_iter().find(|(_,c)|matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id)) { target=id; }
        }
        let key=match change {MemoryChange::Saved {key,..}=>key.clone(),MemoryChange::Proposed(p)=>p.id.clone()};
        let inserted=self.conn.execute("INSERT OR IGNORE INTO response_memory VALUES (?1,?2,?3,?4)",params![channel.0,target.0,key,serde_json::to_string(change)?])? > 0;
        Ok(inserted.then_some(target))
    }
    /// Retry approval links using immutable receipts and already-persisted proposal rows.
    /// No response ID is inferred or created when the proposal row is still absent.
    pub fn reconcile_response_memory(&self, channel: &ChannelId, approvals: &[crate::memory_items::MemoryChange]) -> Result<Vec<(MessageId,crate::memory_items::MemoryChange)>> {
        use crate::memory_items::MemoryChange;
        let records=self.response_memory(channel)?;
        let mut recorded=Vec::new();
        for approval in approvals {
            let MemoryChange::Saved {key,item}=approval else { continue };
            if records.iter().any(|(_,c)|matches!(c,MemoryChange::Saved{key:existing,..} if existing==key)) { continue; }
            let Some(proposal_id)=&item.proposal_id else { continue };
            if let Some((response,_))=records.iter().find(|(_,c)|matches!(c,MemoryChange::Proposed(p) if &p.id==proposal_id))
                && let Some(id)=self.record_response_memory_at(channel,response,approval)? { recorded.push((id,approval.clone())); }
        }
        Ok(recorded)
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

    /// Literal, case-insensitive search of saved direct message text, newest first.
    /// Includes archived and retained chats; never reads project conversations.
    pub fn search_direct_history(&self, agent: &AgentId, query: &str) -> Result<HistorySearch> {
        let needle = query.trim().to_lowercase();
        let mut results = Vec::new();
        if needle.is_empty() { return Ok(HistorySearch { results, has_more: false }); }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.channel_id, c.name, m.role, m.blocks_json
             FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE c.agent_id = ?1 AND c.project_id IS NULL ORDER BY m.ord DESC"
        )?;
        let rows = stmt.query_map(params![agent.0], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, String>(3)?, r.get::<_, String>(4)?
        )))?;
        for row in rows {
            let (message_id, channel_id, channel_name, role, json) = row?;
            let blocks = serde_json::from_str::<Vec<Block>>(&json)?;
            let text = crate::context_menu::message_text(&blocks);
            let folded = text.to_lowercase();
            let Some(index) = folded.find(&needle) else { continue };
            if results.len() == 50 { return Ok(HistorySearch { results, has_more: true }); }
            // Map the folded byte offset back to the original text before truncation.
            let mut folded_bytes = 0;
            let start = text.chars().take_while(|c| {
                if folded_bytes >= index { return false; }
                folded_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
                true
            }).count().saturating_sub(60);
            let mut chars = text.chars().skip(start);
            let excerpt: String = chars.by_ref().take(240).collect();
            let preview = format!("{}{}{}", if start > 0 { "…" } else { "" }, excerpt, if chars.next().is_some() { "…" } else { "" });
            results.push(HistoryMatch { channel_id, channel_name, message_id, role, preview });
        }
        Ok(HistorySearch { results, has_more: false })
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
                    reasoning_effort: None,
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
            project_id: Some(project.id.clone()),
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
    fn list_projects_returns_projects_in_insert_order() { let f=fixture(); let p=Project { id: ProjectId::new(), name:"second".into(),path:"/second".into() }; f.store.insert_project(&p).unwrap(); assert_eq!(f.store.list_projects().unwrap(), vec![f.project, p]); }
    #[test]
    fn all_channels_include_archived_ones_with_the_flag() { let f=fixture(); f.store.archive_channel(&f.channel.id).unwrap(); let channels=f.store.all_channels_for_project(&f.project.id).unwrap(); assert_eq!(channels, vec![(f.channel,true)]); }
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
            .insert_channel(&Channel { id: ChannelId::new(), project_id: Some(f.project.id.clone()), name: "b".into(), agent_id: builder_id.clone(), workspace_mode: WorkspaceMode::MainCheckout })
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
        assert_eq!(store.schema_version().unwrap(), 3);
        let channels = store.channels_for_project(&ProjectId("p".into())).unwrap();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].workspace_mode, WorkspaceMode::MainCheckout, "old channels stay in the main checkout");
        assert_eq!(store.worktree_for(&ChannelId("c".into())).unwrap(), None);
    }

    #[test]
    fn worktree_records_round_trip_update_and_delete() {
        let f = fixture();
        let rec = WorktreeRecord { channel_id: f.channel.id.clone(), path: "/w/x".into(), branch: "discard/x".into(), base_commit: "abc".into(), repo: "/r".into(), owned: true };
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
    fn direct_history_search_is_scoped_literal_and_includes_archived_chats() {
        let f = fixture();
        let direct = Channel { id: ChannelId::new(), project_id: None, name: "Old chat".into(),
            agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        f.store.insert_channel(&direct).unwrap();
        f.store.archive_channel(&direct.id).unwrap();
        let other = AgentId::new();
        f.store.insert_agent(&other, &f.store.load_agent(&f.agent).unwrap()).unwrap();
        let other_chat = Channel { id: ChannelId::new(), agent_id: other, ..direct.clone() };
        f.store.insert_channel(&other_chat).unwrap();
        for channel in [&direct, &f.channel, &other_chat] {
            f.store.insert_message(&Message { id: MessageId::new(), channel_id: channel.id.clone(),
                role: Role::User, blocks: vec![Block::Markdown { text: "Café 100%_ready".into() }] }).unwrap();
        }
        let found = f.store.search_direct_history(&f.agent, " CAFÉ ").unwrap();
        assert_eq!(found.results.len(), 1);
        assert_eq!(found.results[0].channel_id, direct.id.0);
        assert_eq!(found.results[0].preview, "Café 100%_ready");
        assert_eq!(f.store.search_direct_history(&f.agent, "%_").unwrap().results.len(), 1);
        assert!(f.store.search_direct_history(&f.agent, "' OR 1=1 --").unwrap().results.is_empty());
        assert!(f.store.search_direct_history(&f.agent, "  ").unwrap().results.is_empty());
        assert!(f.store.search_direct_history(&AgentId::new(), "Café").unwrap().results.is_empty());
    }

    #[test]
    fn direct_history_search_bounds_results_and_centers_unicode_previews() {
        let f = fixture();
        let chat = Channel { id: ChannelId::new(), project_id: None, name: "Chat".into(),
            agent_id: f.agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        f.store.insert_channel(&chat).unwrap();
        let text = format!("{}İCAFÉ{}", "🙂".repeat(300), "é".repeat(300));
        let mut last = MessageId::new();
        for _ in 0..51 {
            last = MessageId::new();
            f.store.insert_message(&Message { id: last.clone(), channel_id: chat.id.clone(),
                role: Role::Agent, blocks: vec![Block::Code { language: None, text: text.clone() }] }).unwrap();
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
            store.insert_agent(&agent, &AgentConfig {
                reasoning_effort: None,
                name: "Friend".into(), avatar: None, provider: "codex".into(), model: Some("exact".into()),
                system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false,
            }).unwrap();
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
                        reasoning_effort: None,
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
        let first = root.path().join("first"); let next = root.path().join("next");
        std::fs::create_dir_all(&first).unwrap(); std::fs::create_dir_all(&next).unwrap();
        let db = root.path().join("store.db");
        let store = Store::open(&db).unwrap();
        let project = store.import_project_folder(&first).unwrap();
        assert_eq!(store.import_project_folder(&first.join(".")).unwrap().id, project.id);
        let ws = crate::bootstrap::ensure_workspace(&store, Path::new(&project.path), "claude", crate::bootstrap::Role::Default, crate::bootstrap::WorkspaceChoice::Main).unwrap();
        let old_session = store.latest_session(&ws.channel.id).unwrap();
        let message = Message { id: MessageId::new(), channel_id: ws.channel.id.clone(), role: Role::User, blocks: vec![] };
        store.insert_message(&message).unwrap();
        store.change_project_folder(&project.id, &next).unwrap();
        assert_ne!(store.latest_session(&ws.channel.id).unwrap(), old_session);
        assert_eq!(store.messages_for_channel(&ws.channel.id).unwrap(), vec![message]);
        assert_eq!(store.selected_project().unwrap().unwrap().id, project.id);
        drop(store);
        let store = Store::open(&db).unwrap();
        assert_eq!(store.selected_project().unwrap().unwrap().path, next.canonicalize().unwrap().to_string_lossy());
        let other = store.import_project_folder(&first).unwrap();
        assert_ne!(other.id, project.id);
        let target_workspace = crate::bootstrap::ensure_workspace(&store, Path::new(&other.path), "claude", crate::bootstrap::Role::Default, crate::bootstrap::WorkspaceChoice::Main).unwrap();
        let target_session = store.latest_session(&target_workspace.channel.id).unwrap();
        assert!(matches!(store.change_project_folder(&project.id, &first),Err(StoreError::Invalid(_))));
        assert_eq!(store.selected_project().unwrap().unwrap().id, other.id);
        let channels = store.channels_for_project(&other.id).unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.iter().all(|c| c.id != ws.channel.id));
        assert_eq!(store.messages_for_channel(&ws.channel.id).unwrap().len(), 1);
        assert_eq!(store.latest_session(&target_workspace.channel.id).unwrap(), target_session);
        assert!(store.import_project_folder(&db).is_err());
    }
}

#[cfg(test)]
mod agent_lifecycle_tests {
    use super::*;
    use crate::capability::ApprovalPolicy;
    fn config() -> AgentConfig { AgentConfig { reasoning_effort: None, name: crate::agent::generated_name(), avatar: None, provider: "claude".into(), model: Some("same-model".into()), system_prompt: "Be clear".into(), policy: ApprovalPolicy::Ask, plugin_write: false } }
    #[test]
    fn independent_agents_have_editable_names_and_default_direct_chats() {
        let store = Store::open_in_memory().unwrap(); let mut cfg = config();
        assert!(!cfg.name.trim().is_empty());
        let a = store.create_agent(&cfg).unwrap(); let b = store.create_agent(&cfg).unwrap(); assert_ne!(a,b);
        cfg.name = "Edited".into(); store.update_agent(&a,&cfg).unwrap(); assert_eq!(store.load_agent(&a).unwrap().name,"Edited");
        let direct = store.default_conversation(&a).unwrap(); assert_eq!(direct.project_id,None);
        assert_eq!(store.default_conversation(&a).unwrap().id,direct.id);
        assert!(store.latest_session(&direct.id).unwrap().is_some());
        store.create_conversation(&a,None,"Other").unwrap();
        let project = Project { id: ProjectId::new(), name:"P".into(), path:"/p".into() }; store.insert_project(&project).unwrap();
        let one = store.create_conversation(&a,Some(&project.id),"One").unwrap();
        store.create_conversation(&a,Some(&project.id),"Two").unwrap();
        assert_eq!(store.conversations_for_agent(&a).unwrap().len(),4);
        assert_eq!(store.conversations_for_agent(&a).unwrap()[2].project_id,one.project_id);
    }
    #[test]
    fn archive_restore_and_delete_keep_history_and_return_only_private_scopes() {
        let store = Store::open_in_memory().unwrap(); let a=store.create_agent(&config()).unwrap();
        let direct=store.default_conversation(&a).unwrap();
        store.insert_message(&Message { id:MessageId::new(), channel_id:direct.id.clone(), role:Role::User, blocks:vec![] }).unwrap();
        store.set_agent_archived(&a,true).unwrap(); assert!(store.agent_archived(&a).unwrap()); assert!(store.list_agents().unwrap().is_empty());
        store.set_agent_archived(&a,false).unwrap(); assert!(!store.agent_archived(&a).unwrap());
        let result=store.delete_agent(&a,DeleteAgentOptions { delete_conversations:false, delete_private_memory:true }).unwrap();
        assert_eq!(result.private_scopes,vec![PrivateMemoryScope::Agent(a.clone())]);
        assert_eq!(store.messages_for_channel(&direct.id).unwrap().len(),1);
        assert_eq!(store.load_agent(&a).unwrap().name,"Deleted agent");
        assert!(store.set_agent_archived(&a,false).is_err());
        let b=store.create_agent(&config()).unwrap(); let chat=store.default_conversation(&b).unwrap();
        store.delete_agent(&b,DeleteAgentOptions { delete_conversations:true,delete_private_memory:false }).unwrap();
        assert!(store.conversations_for_agent(&b).unwrap().is_empty()); assert!(store.latest_session(&chat.id).unwrap().is_none());
    }
    #[test]
    fn agent_validation_rejects_blank_fields() {
        let store=Store::open_in_memory().unwrap(); let mut cfg=config(); cfg.name=" ".into(); assert!(store.create_agent(&cfg).is_err());
        cfg=config(); cfg.model=None; assert!(store.create_agent(&cfg).is_err()); cfg=config(); cfg.provider=" ".into(); assert!(store.create_agent(&cfg).is_err()); cfg=config(); cfg.model=Some(" ".into()); assert!(store.create_agent(&cfg).is_err());
    }
    #[test]
    fn deletion_choices_are_independent_and_do_not_remove_shared_projects_or_other_agents() {
        for delete_conversations in [false,true] {
            for delete_private_memory in [false,true] {
                let store=Store::open_in_memory().unwrap();
                let agent=store.create_agent(&config()).unwrap();
                let other=store.create_agent(&config()).unwrap();
                let project=Project { id:ProjectId::new(),name:"Shared".into(),path:"/shared".into() };
                store.insert_project(&project).unwrap();
                let direct=store.default_conversation(&agent).unwrap();
                let chat=store.create_conversation(&agent,Some(&project.id),"Project").unwrap();
                let other_chat=store.create_conversation(&other,Some(&project.id),"Other agent").unwrap();
                for channel in [&direct,&chat,&other_chat] {
                    store.insert_message(&Message { id:MessageId::new(),channel_id:channel.id.clone(),role:Role::User,blocks:vec![Block::Markdown { text:"History".into() }] }).unwrap();
                    let session=store.latest_session(&channel.id).unwrap().unwrap();
                    let turn=Turn::new(session.clone()); store.insert_turn(&turn).unwrap();
                    let approval=Approval::new(turn.id.clone(),"Read",serde_json::json!({})); store.save_approval(&approval).unwrap();
                    store.append_event(&Event { id:EventId::new(), session_id:session,seq:1,kind:EventKind::TextDelta { text:"History".into() },raw:None }).unwrap();
                }
                let result=store.delete_agent(&agent,DeleteAgentOptions { delete_conversations,delete_private_memory }).unwrap();
                if delete_private_memory {
                    assert_eq!(result.private_scopes.len(),2);
                    assert!(result.private_scopes.contains(&PrivateMemoryScope::Agent(agent.clone())));
                    assert!(result.private_scopes.contains(&PrivateMemoryScope::AgentProject { agent:agent.clone(),project:project.id.clone() }));
                } else { assert!(result.private_scopes.is_empty()); }
                for channel in [&direct,&chat] {
                    assert_eq!(store.messages_for_channel(&channel.id).unwrap().len(),usize::from(!delete_conversations));
                }
                assert_eq!(store.messages_for_channel(&other_chat.id).unwrap().len(),1);
                assert_eq!(store.list_projects().unwrap(),vec![project]);
                assert_eq!(store.list_agents().unwrap().len(),1);
                if !delete_conversations { assert_eq!(store.load_agent(&chat.agent_id).unwrap().name,"Deleted agent"); }
            }
        }
    }

    #[test]
    fn fixed_context_and_failed_creation_leave_no_partial_conversations() {
        let store=Store::open_in_memory().unwrap(); let agent=store.create_agent(&config()).unwrap();
        let chat=store.default_conversation(&agent).unwrap();
        let project=Project { id:ProjectId::new(),name:"P".into(),path:"/p".into() }; store.insert_project(&project).unwrap();
        assert!(store.conn.execute("UPDATE channels SET project_id=?2 WHERE id=?1",params![chat.id.0,project.id.0]).is_err());
        assert!(store.create_conversation(&agent,Some(&ProjectId::new()),"Missing project").is_err());
        assert_eq!(store.conversations_for_agent(&agent).unwrap(),vec![chat.clone()]);
        assert_eq!(store.ensure_default_conversation(&agent).unwrap().id,chat.id);
        assert_eq!(store.conversations_for_agent(&agent).unwrap().len(),1);
    }

    #[test]
    fn clearing_an_explicit_model_is_rejected_without_mutating_config() {
        let store=Store::open_in_memory().unwrap(); let cfg=config();
        let id=store.create_agent(&cfg).unwrap(); let mut cleared=cfg.clone(); cleared.model=None;
        assert!(matches!(store.update_agent(&id,&cleared),Err(StoreError::Invalid(_))));
        assert_eq!(store.load_agent(&id).unwrap(),cfg);
    }

}
