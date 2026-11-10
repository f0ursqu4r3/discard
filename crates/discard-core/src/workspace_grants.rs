//! A main-checkout grant is consumed by one response from one agent in one channel.
use crate::model::{AgentId, ChannelId};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(10 * 60);
#[derive(Default)]
pub struct MainCheckoutGrants {
    inner: Mutex<HashMap<ChannelId, (AgentId, Instant)>>,
}
impl MainCheckoutGrants {
    pub fn grant(&self, channel: &ChannelId, agent: &AgentId) {
        let mut grants = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        grants.retain(|_, (_, created)| created.elapsed() < TTL);
        grants.insert(channel.clone(), (agent.clone(), Instant::now()));
    }
    pub fn take(&self, channel: &ChannelId, agent: &AgentId) -> bool {
        let mut grants = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        grants.retain(|_, (_, created)| created.elapsed() < TTL);
        if grants.get(channel).is_some_and(|(owner, _)| owner == agent) {
            grants.remove(channel);
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_grant_is_used_once() {
        let grants = MainCheckoutGrants::default();
        let c = ChannelId::new();
        let a = AgentId::new();
        grants.grant(&c, &a);
        assert!(grants.take(&c, &a));
        assert!(!grants.take(&c, &a));
    }
    #[test]
    fn a_grant_belongs_to_one_agent() {
        let grants = MainCheckoutGrants::default();
        let c = ChannelId::new();
        let a = AgentId::new();
        grants.grant(&c, &a);
        assert!(!grants.take(&c, &AgentId::new()));
        assert!(grants.take(&c, &a));
    }
    #[test]
    fn a_grant_expires_after_ttl() {
        let grants = MainCheckoutGrants::default();
        let c = ChannelId::new();
        let a = AgentId::new();
        grants.grant(&c, &a);
        grants.inner.lock().unwrap().get_mut(&c).unwrap().1 = Instant::now() - TTL;
        assert!(!grants.take(&c, &a));
    }
    #[test]
    fn a_grant_never_crosses_channels() {
        let grants = MainCheckoutGrants::default();
        let c = ChannelId::new();
        let a = AgentId::new();
        grants.grant(&c, &a);
        assert!(!grants.take(&ChannelId::new(), &a));
        assert!(grants.take(&c, &a));
    }
}
