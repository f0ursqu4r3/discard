//! Ephemeral, user-only requests. No pending approval survives an app restart.
use crate::model::{AgentId, ChannelId};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(10 * 60);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRequestKind {
    WorktreeCreate,
    MainCheckoutWrite,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteRequest {
    pub id: String,
    pub kind: WriteRequestKind,
    pub channel_id: String,
    pub agent_id: String,
    pub branch: String,
    pub path: String,
    pub state: String,
    pub result: Option<String>,
}
struct Pending {
    request: WriteRequest,
    created: Instant,
}
#[derive(Default)]
pub struct WriteRequests {
    inner: Mutex<HashMap<String, Pending>>,
}
impl WriteRequests {
    fn expire(rows: &mut HashMap<String, Pending>) {
        for pending in rows.values_mut() {
            if pending.request.state == "pending" && pending.created.elapsed() >= TTL {
                pending.request.state = "expired".into();
                pending.request.result = Some("Approval expired. Ask for a new request.".into());
            }
        }
    }
    pub fn request(
        &self,
        kind: WriteRequestKind,
        channel: &ChannelId,
        agent: &AgentId,
        branch: String,
        path: String,
    ) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        if let Some(existing) = rows.values().find(|p| {
            p.request.channel_id == channel.0
                && p.request.kind == kind
                && matches!(p.request.state.as_str(), "pending" | "running")
        }) {
            return Ok(existing.request.clone());
        }
        if rows.len() >= 100 {
            let oldest = rows
                .iter()
                .filter(|(_, p)| !matches!(p.request.state.as_str(), "pending" | "running"))
                .min_by_key(|(_, p)| p.created)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                rows.remove(&id);
            } else {
                return Err("Too many pending write requests.".into());
            }
        }
        let request = WriteRequest {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            channel_id: channel.0.clone(),
            agent_id: agent.0.clone(),
            branch,
            path,
            state: "pending".into(),
            result: None,
        };
        rows.insert(
            request.id.clone(),
            Pending {
                request: request.clone(),
                created: Instant::now(),
            },
        );
        Ok(request)
    }
    pub fn list(&self, channel: &ChannelId) -> Vec<WriteRequest> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let mut rows: Vec<_> = rows
            .values()
            .filter(|p| p.request.channel_id == channel.0)
            .collect();
        rows.sort_by_key(|p| p.created);
        rows.into_iter().map(|p| p.request.clone()).collect()
    }
    pub fn begin(
        &self,
        channel: &ChannelId,
        id: &str,
        approve: bool,
    ) -> Result<WriteRequest, String> {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut rows);
        let row = rows
            .get_mut(id)
            .filter(|p| p.request.channel_id == channel.0)
            .ok_or("This request does not belong to this channel.")?;
        if row.request.state != "pending" {
            return Err("This request is no longer pending.".into());
        }
        row.request.state = if approve { "running" } else { "denied" }.into();
        if !approve {
            row.request.result = Some(
                match row.request.kind {
                    WriteRequestKind::WorktreeCreate => "Worktree creation denied.",
                    WriteRequestKind::MainCheckoutWrite => "Main-checkout write access denied.",
                }
                .into(),
            );
        }
        Ok(row.request.clone())
    }
    pub fn finish(&self, id: &str, result: Result<String, String>) -> WriteRequest {
        let mut rows = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let row = rows
            .get_mut(id)
            .expect("begin retains running requests until finish");
        if row.request.state == "running" {
            row.request.state = if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
            .into();
            row.request.result = Some(result.unwrap_or_else(|error| error));
        }
        row.request.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(requests: &WriteRequests, channel: &ChannelId) -> WriteRequest {
        requests
            .request(
                WriteRequestKind::WorktreeCreate,
                channel,
                &AgentId::new(),
                "feature".into(),
                "/target".into(),
            )
            .unwrap()
    }
    #[test]
    fn requests_expire_after_ttl() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        requests
            .inner
            .lock()
            .unwrap()
            .get_mut(&r.id)
            .unwrap()
            .created = Instant::now() - TTL;
        assert!(requests.begin(&channel, &r.id, true).is_err());
        assert_eq!(requests.list(&channel)[0].state, "expired");
    }
    #[test]
    fn a_request_is_decided_once() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert_eq!(
            requests.begin(&channel, &r.id, true).unwrap().state,
            "running"
        );
        assert!(requests.begin(&channel, &r.id, true).is_err());
        assert_eq!(requests.finish(&r.id, Ok("done".into())).state, "completed");
    }
    #[test]
    fn decide_for_another_channel_is_refused() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert!(requests.begin(&ChannelId::new(), &r.id, true).is_err());
        assert_eq!(
            requests.begin(&channel, &r.id, false).unwrap().state,
            "denied"
        );
    }
    #[test]
    fn one_pending_request_per_channel_and_kind() {
        let requests = WriteRequests::default();
        let channel = ChannelId::new();
        let r = request(&requests, &channel);
        assert_eq!(request(&requests, &channel).id, r.id);
        assert!(
            requests
                .request(
                    WriteRequestKind::MainCheckoutWrite,
                    &channel,
                    &AgentId::new(),
                    "main".into(),
                    "/repo".into()
                )
                .is_ok()
        );
        assert_eq!(requests.list(&channel).len(), 2);
    }
}
