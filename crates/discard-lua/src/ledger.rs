//! The approval ledger: which exact plugin bytes the user approved. Rows are signed with HMAC-SHA256 using a key
//! from the OS secret store, so a process that edits the database file cannot forge an approval.

use crate::keystore::{KeyError, KeyStore};
use crate::manifest::Capabilities;
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LedgerError>;

#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub name: String,
    pub hash: String,
    pub capabilities: Capabilities,
    /// The user allowed later code edits without review (only offered for `ui`-only plugins).
    pub trusted_edits: bool,
    /// The commit in the plugin repository that holds these bytes.
    pub commit: String,
    pub approved_at: i64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS approvals (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    hash TEXT NOT NULL,
    caps_json TEXT NOT NULL,
    trusted INTEGER NOT NULL,
    commit_id TEXT NOT NULL,
    approved_at INTEGER NOT NULL,
    mac TEXT NOT NULL,
    UNIQUE (name, hash)
);
CREATE TABLE IF NOT EXISTS current (name TEXT PRIMARY KEY, hash TEXT NOT NULL, mac TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS failures (name TEXT PRIMARY KEY, count INTEGER NOT NULL);
";

pub struct Ledger {
    conn: Mutex<Connection>,
    keys: Arc<dyn KeyStore>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Fields are joined with a separator that cannot appear in a hash or a validated plugin name.
fn mac_input(parts: &[&str]) -> Vec<u8> {
    parts.join("\u{1f}").into_bytes()
}

fn sign(key: &[u8; 32], parts: &[&str]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&mac_input(parts));
    hex(&mac.finalize().into_bytes())
}

fn verify(key: &[u8; 32], parts: &[&str], tag: &str) -> bool {
    let Some(tag) = unhex(tag) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&mac_input(parts));
    mac.verify_slice(&tag).is_ok()
}

fn approval_parts<'a>(a: &'a Approval, caps_json: &'a str, trusted: &'a str, at: &'a str) -> [&'a str; 7] {
    ["approval", &a.name, &a.hash, caps_json, trusted, &a.commit, at]
}

impl Ledger {
    pub fn open(path: &Path, keys: Arc<dyn KeyStore>) -> Result<Self> {
        Self::with_connection(Connection::open(path)?, keys)
    }

    pub fn open_in_memory(keys: Arc<dyn KeyStore>) -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?, keys)
    }

    fn with_connection(conn: Connection, keys: Arc<dyn KeyStore>) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn), keys })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Check that a signing key is available.
    pub fn check_key(&self) -> Result<()> {
        self.keys.key()?;
        Ok(())
    }

    /// Record that the user approved these exact bytes. They become the current approval for `name`.
    pub fn record(&self, a: &Approval) -> Result<()> {
        let key = self.keys.key()?;
        let caps = serde_json::to_string(&a.capabilities)?;
        let (trusted, at) = (if a.trusted_edits { "1" } else { "0" }, a.approved_at.to_string());
        let mac = sign(&key, &approval_parts(a, &caps, trusted, &at));
        let current_mac = sign(&key, &["current", &a.name, &a.hash]);
        let mut conn = self.db();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO approvals (name, hash, caps_json, trusted, commit_id, approved_at, mac) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(name, hash) DO UPDATE SET caps_json = excluded.caps_json, trusted = excluded.trusted,
               commit_id = excluded.commit_id, approved_at = excluded.approved_at, mac = excluded.mac",
            params![a.name, a.hash, caps, i64::from(a.trusted_edits), a.commit, a.approved_at, mac],
        )?;
        tx.execute(
            "INSERT INTO current (name, hash, mac) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET hash = excluded.hash, mac = excluded.mac",
            params![a.name, a.hash, current_mac],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Read one approval row and check its signature.
    fn read_approval(&self, conn: &Connection, key: &[u8; 32], name: &str, hash: &str) -> Result<Option<Approval>> {
        let row = conn
            .query_row(
                "SELECT caps_json, trusted, commit_id, approved_at, mac FROM approvals WHERE name = ?1 AND hash = ?2",
                params![name, hash],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?)),
            )
            .optional()?;
        let Some((caps_json, trusted, commit, at, mac)) = row else { return Ok(None) };
        let approval_shape = |capabilities, trusted_edits| Approval {
            name: name.to_string(),
            hash: hash.to_string(),
            capabilities,
            trusted_edits,
            commit: commit.clone(),
            approved_at: at,
        };
        let probe = approval_shape(Capabilities::default(), false);
        let at_text = at.to_string();
        if !verify(key, &approval_parts(&probe, &caps_json, &trusted.to_string(), &at_text), &mac) {
            return Ok(None);
        }
        let capabilities: Capabilities = match serde_json::from_str(&caps_json) {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        Ok(Some(approval_shape(capabilities, trusted == 1)))
    }

    /// The current approval, if it exists and its signature is valid. A forged or edited row reads as `None`.
    pub fn current(&self, name: &str) -> Result<Option<Approval>> {
        let key = self.keys.key()?;
        let conn = self.db();
        let pointer: Option<(String, String)> = conn
            .query_row("SELECT hash, mac FROM current WHERE name = ?1", params![name], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((hash, mac)) = pointer else { return Ok(None) };
        if !verify(&key, &["current", name, &hash], &mac) {
            return Ok(None);
        }
        self.read_approval(&conn, &key, name, &hash)
    }

    /// True only if `hash` is the current, validly signed approval for `name`.
    pub fn is_approved(&self, name: &str, hash: &str) -> Result<bool> {
        Ok(self.current(name)?.is_some_and(|a| a.hash == hash))
    }

    /// Every validly signed approval for `name`, oldest first.
    pub fn history(&self, name: &str) -> Result<Vec<Approval>> {
        let key = self.keys.key()?;
        let conn = self.db();
        let hashes: Vec<String> = {
            let mut stmt = conn.prepare("SELECT hash FROM approvals WHERE name = ?1 ORDER BY seq")?;
            let rows = stmt.query_map(params![name], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::new();
        for hash in hashes {
            if let Some(a) = self.read_approval(&conn, &key, name, &hash)? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// Make an earlier approved version current again (a rollback).
    pub fn make_current(&self, name: &str, hash: &str) -> Result<bool> {
        let key = self.keys.key()?;
        let conn = self.db();
        if self.read_approval(&conn, &key, name, hash)?.is_none() {
            return Ok(false);
        }
        let mac = sign(&key, &["current", name, hash]);
        conn.execute(
            "INSERT INTO current (name, hash, mac) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET hash = excluded.hash, mac = excluded.mac",
            params![name, hash, mac],
        )?;
        Ok(true)
    }

    /// Change the "trust future edits" flag. Re-signs the row.
    pub fn set_trusted(&self, name: &str, trusted: bool) -> Result<bool> {
        let Some(mut approval) = self.current(name)? else { return Ok(false) };
        approval.trusted_edits = trusted;
        let key = self.keys.key()?;
        let caps = serde_json::to_string(&approval.capabilities)?;
        let (flag, at) = (if trusted { "1" } else { "0" }, approval.approved_at.to_string());
        let mac = sign(&key, &approval_parts(&approval, &caps, flag, &at));
        self.db().execute(
            "UPDATE approvals SET trusted = ?3, mac = ?4 WHERE name = ?1 AND hash = ?2",
            params![name, approval.hash, i64::from(trusted), mac],
        )?;
        Ok(true)
    }

    /// Count a failed load or a runtime violation. Returns the number of failures in a row.
    pub fn note_failure(&self, name: &str) -> Result<u32> {
        let conn = self.db();
        conn.execute("INSERT INTO failures (name, count) VALUES (?1, 1) ON CONFLICT(name) DO UPDATE SET count = count + 1", params![name])?;
        Ok(conn.query_row("SELECT count FROM failures WHERE name = ?1", params![name], |r| r.get(0))?)
    }

    pub fn clear_failures(&self, name: &str) -> Result<()> {
        self.db().execute("DELETE FROM failures WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Remove every record for `name`.
    pub fn forget(&self, name: &str) -> Result<()> {
        let mut conn = self.db();
        let tx = conn.transaction()?;
        for table in ["approvals", "current", "failures"] {
            tx.execute(&format!("DELETE FROM {table} WHERE name = ?1"), params![name])?;
        }
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    fn raw(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{StaticKeyStore, UnavailableKeyStore};
    use crate::manifest::{ExecScope, NetScope};

    fn ledger() -> Ledger {
        Ledger::open_in_memory(Arc::new(StaticKeyStore([1; 32]))).unwrap()
    }

    fn approval(name: &str, hash: &str) -> Approval {
        Approval { name: name.into(), hash: hash.into(), capabilities: Capabilities { ui: true, ..Capabilities::default() }, trusted_edits: false, commit: "c0ffee".into(), approved_at: 1_700_000_000 }
    }

    #[test]
    fn a_recorded_approval_reads_back_exactly() {
        let l = ledger();
        let mut a = approval("p", "h1");
        a.capabilities.net = Some(NetScope { hosts: vec!["example.com:443".into()] });
        a.capabilities.exec = Some(ExecScope { commands: vec!["git".into()] });
        l.record(&a).unwrap();
        assert_eq!(l.current("p").unwrap(), Some(a));
        assert!(l.is_approved("p", "h1").unwrap());
        assert!(!l.is_approved("p", "other").unwrap());
        assert!(!l.is_approved("nope", "h1").unwrap());
    }

    #[test]
    fn a_new_approval_replaces_the_current_one_and_history_keeps_both() {
        let l = ledger();
        l.record(&approval("p", "h1")).unwrap();
        l.record(&approval("p", "h2")).unwrap();
        assert_eq!(l.current("p").unwrap().unwrap().hash, "h2");
        let hashes: Vec<_> = l.history("p").unwrap().into_iter().map(|a| a.hash).collect();
        assert_eq!(hashes, ["h1", "h2"]);
    }

    #[test]
    fn plugins_are_independent() {
        let l = ledger();
        l.record(&approval("a", "ha")).unwrap();
        l.record(&approval("b", "hb")).unwrap();
        assert!(l.is_approved("a", "ha").unwrap() && !l.is_approved("a", "hb").unwrap());
    }

    #[test]
    fn make_current_rolls_back_to_an_earlier_approved_hash_only() {
        let l = ledger();
        l.record(&approval("p", "h1")).unwrap();
        l.record(&approval("p", "h2")).unwrap();
        assert!(l.make_current("p", "h1").unwrap());
        assert_eq!(l.current("p").unwrap().unwrap().hash, "h1");
        assert!(!l.make_current("p", "never-approved").unwrap());
        assert_eq!(l.current("p").unwrap().unwrap().hash, "h1");
    }

    #[test]
    fn editing_the_approved_hash_in_the_database_invalidates_the_approval() {
        let l = ledger();
        l.record(&approval("p", "good")).unwrap();
        l.raw().execute("UPDATE approvals SET hash = 'evil' WHERE name = 'p'", []).unwrap();
        l.raw().execute("UPDATE current SET hash = 'evil' WHERE name = 'p'", []).unwrap();
        assert_eq!(l.current("p").unwrap(), None);
        assert!(!l.is_approved("p", "evil").unwrap());
    }

    #[test]
    fn pointing_current_at_another_validly_signed_row_for_a_different_plugin_does_not_work() {
        let l = ledger();
        l.record(&approval("a", "ha")).unwrap();
        l.record(&approval("b", "hb")).unwrap();
        l.raw().execute("UPDATE current SET hash = 'hb' WHERE name = 'a'", []).unwrap();
        assert_eq!(l.current("a").unwrap(), None);
    }

    #[test]
    fn widening_capabilities_or_trust_in_the_database_invalidates_the_approval() {
        let l = ledger();
        l.record(&approval("p", "h1")).unwrap();
        l.raw().execute("UPDATE approvals SET trusted = 1 WHERE name = 'p'", []).unwrap();
        assert_eq!(l.current("p").unwrap(), None);
        let l = ledger();
        l.record(&approval("p", "h1")).unwrap();
        l.raw().execute("UPDATE approvals SET caps_json = '{\"ui\":true,\"exec\":{\"commands\":[\"sh\"]}}' WHERE name = 'p'", []).unwrap();
        assert_eq!(l.current("p").unwrap(), None);
    }

    #[test]
    fn a_row_signed_with_another_key_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        Ledger::open(&path, Arc::new(StaticKeyStore([1; 32]))).unwrap().record(&approval("p", "h1")).unwrap();
        let other = Ledger::open(&path, Arc::new(StaticKeyStore([2; 32]))).unwrap();
        assert_eq!(other.current("p").unwrap(), None, "a different key must not validate old approvals");
        assert!(Ledger::open(&path, Arc::new(StaticKeyStore([1; 32]))).unwrap().current("p").unwrap().is_some());
    }

    #[test]
    fn without_a_key_nothing_can_be_recorded_or_trusted() {
        let l = Ledger::open_in_memory(Arc::new(UnavailableKeyStore("no keychain".into()))).unwrap();
        assert!(matches!(l.record(&approval("p", "h1")), Err(LedgerError::Key(_))));
        assert!(matches!(l.current("p"), Err(LedgerError::Key(_))));
    }

    #[test]
    fn set_trusted_re_signs_the_row() {
        let l = ledger();
        l.record(&approval("p", "h1")).unwrap();
        assert!(l.set_trusted("p", true).unwrap());
        assert!(l.current("p").unwrap().unwrap().trusted_edits);
        assert!(l.set_trusted("p", false).unwrap());
        assert!(!l.current("p").unwrap().unwrap().trusted_edits);
        assert!(!l.set_trusted("missing", true).unwrap());
    }

    #[test]
    fn failures_count_up_and_clear() {
        let l = ledger();
        assert_eq!((l.note_failure("p").unwrap(), l.note_failure("p").unwrap(), l.note_failure("p").unwrap()), (1, 2, 3));
        assert_eq!(l.note_failure("q").unwrap(), 1);
        l.clear_failures("p").unwrap();
        assert_eq!(l.note_failure("p").unwrap(), 1);
    }

    #[test]
    fn forget_removes_everything_for_one_plugin() {
        let l = ledger();
        l.record(&approval("a", "ha")).unwrap();
        l.record(&approval("b", "hb")).unwrap();
        l.note_failure("a").unwrap();
        l.forget("a").unwrap();
        assert_eq!(l.current("a").unwrap(), None);
        assert!(l.history("a").unwrap().is_empty());
        assert_eq!(l.note_failure("a").unwrap(), 1);
        assert!(l.is_approved("b", "hb").unwrap());
    }

    #[test]
    fn approvals_survive_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        {
            Ledger::open(&path, Arc::new(StaticKeyStore([9; 32]))).unwrap().record(&approval("p", "h1")).unwrap();
        }
        assert!(Ledger::open(&path, Arc::new(StaticKeyStore([9; 32]))).unwrap().is_approved("p", "h1").unwrap());
    }
}
