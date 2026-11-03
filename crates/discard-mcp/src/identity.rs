//! Who is calling. Each provider session gets its own token, bound to one agent, project, and tool surface.

use discard_core::memory::ScopeKind;
use discard_core::model::{AgentId, ProjectId};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Memory tools and plugin tools use separate connections with separate tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    Memory,
    Plugins,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub agent: AgentId,
    pub project: ProjectId,
    pub surface: Surface,
    pub scopes: BTreeSet<ScopeKindOrd>,
    pub can_write: bool,
}

/// `ScopeKind` with an ordering, so identities can hold them in a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeKindOrd {
    Agent,
    Project,
    Global,
}

impl From<ScopeKind> for ScopeKindOrd {
    fn from(k: ScopeKind) -> Self {
        match k {
            ScopeKind::Agent => ScopeKindOrd::Agent,
            ScopeKind::Project => ScopeKindOrd::Project,
            ScopeKind::Global => ScopeKindOrd::Global,
        }
    }
}

pub const MAX_ACTIVE_TOKENS: usize = 1024;

pub struct TokenRegistry {
    inner: Mutex<HashMap<String, (Identity, Instant)>>,
}

impl Default for TokenRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// A new unguessable token (64 hex characters) that expires after `ttl`.
    pub fn issue(&self, identity: Identity, ttl: Duration) -> Result<String, String> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        map.retain(|_, (_, expires)| *expires > now);
        if map.len() >= MAX_ACTIVE_TOKENS {
            return Err("too many active session tokens".to_string());
        }
        // Two random v4 UUIDs: 244 random bits from the operating system.
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        map.insert(token.clone(), (identity, now + ttl));
        Ok(token)
    }

    pub fn lookup(&self, token: &str) -> Option<Identity> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(token) {
            Some((identity, expires)) if *expires > Instant::now() => Some(identity.clone()),
            Some(_) => {
                map.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(token).is_some()
    }

    pub fn active(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(surface: Surface) -> Identity {
        Identity {
            agent: AgentId("a1".into()),
            project: ProjectId("p1".into()),
            surface,
            scopes: [ScopeKindOrd::Agent, ScopeKindOrd::Project].into(),
            can_write: true,
        }
    }

    #[test]
    fn a_token_resolves_to_the_identity_it_was_issued_for() {
        let r = TokenRegistry::new();
        let t = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        assert_eq!(r.lookup(&t), Some(identity(Surface::Memory)));
    }

    #[test]
    fn tokens_are_long_unique_and_hex() {
        let r = TokenRegistry::new();
        let a = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        let b = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn unknown_empty_and_near_miss_tokens_are_rejected() {
        let r = TokenRegistry::new();
        let t = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        assert_eq!(r.lookup(""), None);
        assert_eq!(r.lookup("nope"), None);
        assert_eq!(r.lookup(&t[..63]), None);
        assert_eq!(r.lookup(&format!("{t}0")), None);
        assert_eq!(r.lookup(&t.to_uppercase()), None);
    }

    #[test]
    fn revoked_tokens_stop_working() {
        let r = TokenRegistry::new();
        let t = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        assert!(r.revoke(&t));
        assert_eq!(r.lookup(&t), None);
        assert!(!r.revoke(&t));
    }

    #[test]
    fn expired_tokens_stop_working_and_are_dropped() {
        let r = TokenRegistry::new();
        let t = r.issue(identity(Surface::Memory), Duration::from_millis(20)).unwrap();
        assert!(r.lookup(&t).is_some());
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(r.lookup(&t), None);
        assert_eq!(r.active(), 0);
    }

    #[test]
    fn each_surface_gets_its_own_identity() {
        let r = TokenRegistry::new();
        let m = r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        let p = r.issue(identity(Surface::Plugins), Duration::from_secs(60)).unwrap();
        assert_eq!(r.lookup(&m).unwrap().surface, Surface::Memory);
        assert_eq!(r.lookup(&p).unwrap().surface, Surface::Plugins);
    }

    #[test]
    fn the_registry_has_a_size_limit() {
        let r = TokenRegistry::new();
        for _ in 0..MAX_ACTIVE_TOKENS {
            r.issue(identity(Surface::Memory), Duration::from_secs(60)).unwrap();
        }
        assert!(r.issue(identity(Surface::Memory), Duration::from_secs(60)).is_err());
    }
}
