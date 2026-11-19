//! Native provider sessions: one row per channel and agent. The row holds the provider handle and
//! what the provider session has already received, so the next turn can send only what changed.

use super::{Result, Store};
use crate::model::{AgentId, ChannelId, MessageId};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;

pub(super) const AGENT_SESSIONS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent_sessions (
    channel_id TEXT NOT NULL REFERENCES channels(id),
    agent_id TEXT NOT NULL REFERENCES agents(id),
    handle TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    delivered_through TEXT,
    delivered_memory TEXT NOT NULL DEFAULT '{}',
    delivered_topic_version INTEGER,
    delivered_capture_paused INTEGER NOT NULL DEFAULT 0,
    bytes_in_session INTEGER NOT NULL DEFAULT 0,
    cwd TEXT NOT NULL,
    dropped_reason TEXT,
    PRIMARY KEY(channel_id, agent_id)
);";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub channel_id: ChannelId,
    pub agent_id: AgentId,
    pub handle: String,
    /// Canonical JSON text.
    pub fingerprint: String,
    pub delivered_through: Option<MessageId>,
    pub delivered_memory: BTreeMap<String, u64>,
    pub delivered_topic_version: Option<u64>,
    pub delivered_capture_paused: bool,
    pub bytes_in_session: u64,
    pub cwd: String,
    /// Set: the handle is unusable. The next response replays history and shows this reason.
    pub dropped_reason: Option<String>,
}

impl Store {
    pub fn agent_session(&self, channel: &ChannelId, agent: &AgentId) -> Result<Option<AgentSession>> {
        let raw = self.conn.query_row(
            "SELECT handle,fingerprint,delivered_through,delivered_memory,delivered_topic_version,
                    delivered_capture_paused,bytes_in_session,cwd,dropped_reason
             FROM agent_sessions WHERE channel_id=?1 AND agent_id=?2",
            params![channel.0, agent.0],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, bool>(5)?,
                    r.get::<_, i64>(6)?, r.get::<_, String>(7)?, r.get::<_, Option<String>>(8)?)),
        ).optional()?;
        let Some((handle, fingerprint, through, memory, topic, paused, bytes, cwd, dropped_reason)) = raw else {
            return Ok(None);
        };
        Ok(Some(AgentSession {
            channel_id: channel.clone(),
            agent_id: agent.clone(),
            handle,
            fingerprint,
            delivered_through: through.map(MessageId),
            delivered_memory: serde_json::from_str(&memory)?,
            delivered_topic_version: topic.map(|v| v as u64),
            delivered_capture_paused: paused,
            bytes_in_session: bytes as u64,
            cwd,
            dropped_reason,
        }))
    }

    /// Insert or replace the row. A `None` reason clears an earlier drop.
    pub fn put_agent_session(&self, row: &AgentSession) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent_sessions(channel_id,agent_id,handle,fingerprint,delivered_through,delivered_memory,
                delivered_topic_version,delivered_capture_paused,bytes_in_session,cwd,dropped_reason)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(channel_id,agent_id) DO UPDATE SET handle=excluded.handle,fingerprint=excluded.fingerprint,
                delivered_through=excluded.delivered_through,delivered_memory=excluded.delivered_memory,
                delivered_topic_version=excluded.delivered_topic_version,
                delivered_capture_paused=excluded.delivered_capture_paused,
                bytes_in_session=excluded.bytes_in_session,cwd=excluded.cwd,dropped_reason=excluded.dropped_reason",
            params![
                row.channel_id.0, row.agent_id.0, row.handle, row.fingerprint,
                row.delivered_through.as_ref().map(|m| m.0.clone()),
                serde_json::to_string(&row.delivered_memory)?,
                row.delivered_topic_version.map(|v| v as i64), row.delivered_capture_paused,
                row.bytes_in_session as i64, row.cwd, row.dropped_reason,
            ],
        )?;
        Ok(())
    }

    /// Mark the handle unusable. The row stays, so the next response can show the reason.
    pub fn drop_agent_session(&self, channel: &ChannelId, agent: &AgentId, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_sessions SET dropped_reason=?3 WHERE channel_id=?1 AND agent_id=?2",
            params![channel.0, agent.0, reason],
        )?;
        Ok(())
    }

    pub fn delete_agent_session(&self, channel: &ChannelId, agent: &AgentId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM agent_sessions WHERE channel_id=?1 AND agent_id=?2",
            params![channel.0, agent.0],
        )?;
        Ok(())
    }
}
