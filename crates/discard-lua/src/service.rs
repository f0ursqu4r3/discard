//! The plugin lifecycle: draft, review, approve, install, load, roll back.
//!
//! Rules enforced here (see PLAN.md, "Agent-authored plugins"):
//! - Agents only write drafts. Only `approve`, called from Rust-rendered UI, makes code live.
//! - Review binds to exact bytes: `approve` installs the snapshot that was reviewed, not the draft directory.
//! - At startup a plugin loads only if its bytes hash to a signed approval. Anything else is disabled.
//! - A plugin that fails three times in a row is reverted to its previous approved version.

use crate::drafts::{DraftError, DraftStore};
use crate::ledger::{Approval, Ledger, LedgerError};
use crate::manager::PluginManager;
use crate::manifest::Manifest;
use crate::repo::{PluginRepo, RepoError};
use crate::runtime::{LoadedInfo, PluginError, PluginLimits};
use crate::repo;
use crate::snapshot::{Snapshot, SnapshotError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Failures in a row before Discard reverts a plugin.
pub const REVERT_THRESHOLD: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Draft(#[from] DraftError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error("{0}")]
    Invalid(String),
    #[error("the draft is identical to the installed plugin")]
    NoChange,
    #[error("there is no pending review `{0}`")]
    NoSuchReview(String),
    #[error("the plugin did not load, so it was not installed: {0}")]
    LoadFailed(String),
    #[error("plugin `{0}` has no earlier approved version")]
    NoPreviousVersion(String),
    #[error("plugin `{0}` is not installed")]
    NotInstalled(String),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}

pub type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    New,
    Update,
}

/// What the user sees before approving. Built from an immutable copy of the draft.
#[derive(Debug, Clone)]
pub struct Review {
    pub id: String,
    pub name: String,
    pub kind: ReviewKind,
    pub hash: String,
    pub manifest: Manifest,
    /// Unified diff against the installed version, or the whole plugin for a new one.
    pub diff: String,
    /// What the plugin asks to be able to do.
    pub capability_lines: Vec<String>,
    /// For an update: lines starting with `+` (newly requested) or `-` (dropped).
    pub capability_changes: Vec<String>,
    /// The plugin asks to run programs, which means full local access.
    pub full_local_access: bool,
    /// "Trust future edits" may be offered (the plugin asks for `ui` and nothing else).
    pub trust_offered: bool,
    snapshot: Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Running,
    Failed(String),
    /// The files on disk do not match an approved version. The plugin is not loaded.
    ModifiedOutsideReview,
    DisabledSafeMode,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub modified_outside_review: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// Plugins whose edits were accepted without review because the user trusted them (ui-only, same capabilities).
    pub trusted_updates: Vec<String>,
    pub reverted: Vec<String>,
    /// Set when no signing key is available. No user plugin is loaded.
    pub key_problem: Option<String>,
    pub safe_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatus {
    pub name: String,
    pub state: PluginState,
    pub trusted_edits: bool,
}

#[derive(Debug, Clone)]
pub struct TestReport {
    pub loaded: bool,
    pub error: Option<String>,
    pub slots: Vec<(String, std::result::Result<Value, String>)>,
    pub commands: Vec<String>,
    pub theme_tokens: Vec<String>,
    pub capabilities: Vec<String>,
}

pub struct PluginService {
    repo: PluginRepo,
    drafts: DraftStore,
    ledger: Ledger,
    manager: Arc<Mutex<PluginManager>>,
    pending: Mutex<Vec<Review>>,
    safe_mode: bool,
    states: Mutex<BTreeMap<String, PluginState>>,
    sandbox_limits: PluginLimits,
    ui_state: Mutex<Value>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The manifest of a snapshot, with errors an agent can act on.
fn manifest_of(snapshot: &Snapshot) -> Result<Manifest> {
    match snapshot.manifest() {
        Ok(m) => Ok(m),
        Err(SnapshotError::MissingFile(_)) => Err(ServiceError::Invalid("the plugin needs a plugin.toml".into())),
        Err(e) => Err(ServiceError::Invalid(e.to_string())),
    }
}

fn check_entry(snapshot: &Snapshot) -> Result<()> {
    snapshot.entry_source().map(|_| ()).map_err(|_| ServiceError::Invalid("the plugin needs an init.lua".into()))
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

impl PluginService {
    /// `dir` holds `plugins/` (the git repository) and `plugin-drafts/`.
    pub fn new(dir: &Path, ledger: Ledger, manager: Arc<Mutex<PluginManager>>, safe_mode: bool) -> Result<Self> {
        Ok(Self {
            repo: PluginRepo::open(&dir.join("plugins"))?,
            drafts: DraftStore::new(&dir.join("plugin-drafts"))?,
            ledger,
            manager,
            pending: Mutex::new(Vec::new()),
            safe_mode,
            states: Mutex::new(BTreeMap::new()),
            sandbox_limits: PluginLimits::default(),
            ui_state: Mutex::new(Value::Null),
        })
    }

    /// Limits for the sandbox that checks drafts. The live manager has its own limits.
    pub fn with_sandbox_limits(mut self, limits: PluginLimits) -> Self {
        self.sandbox_limits = limits;
        self
    }

    pub fn drafts(&self) -> &DraftStore {
        &self.drafts
    }

    /// Load a snapshot in a throwaway manager. Nothing is installed.
    fn sandbox_load(&self, snapshot: &Snapshot) -> std::result::Result<(PluginManager, LoadedInfo), PluginError> {
        let mut sandbox = PluginManager::new(self.sandbox_limits.clone());
        let info = sandbox.load(snapshot.clone())?;
        Ok((sandbox, info))
    }

    /// Load the draft in a sandbox and render each slot with `sample_state`. Nothing is installed.
    pub fn test_draft(&self, name: &str, sample_state: &Value) -> Result<TestReport> {
        let snapshot = self.drafts.snapshot(name)?;
        let manifest = manifest_of(&snapshot)?;
        let capabilities = manifest.capabilities.describe();
        let (mut sandbox, info) = match self.sandbox_load(&snapshot) {
            Ok(loaded) => loaded,
            Err(e) => {
                return Ok(TestReport { loaded: false, error: Some(e.to_string()), slots: Vec::new(), commands: Vec::new(), theme_tokens: Vec::new(), capabilities });
            }
        };
        let plugin = info.manifest.name.clone();
        let mut slots = Vec::new();
        for slot in &info.slots {
            let result = match sandbox.render_slot(slot, sample_state) {
                Some(rendered) => serde_json::to_value(&rendered.element).map_err(|e| e.to_string()),
                None => Err(sandbox.errors().get(&plugin).cloned().unwrap_or_else(|| "the slot did not render".into())),
            };
            slots.push((slot.clone(), result));
        }
        Ok(TestReport { loaded: true, error: None, slots, commands: info.commands.iter().map(|c| c.name.clone()).collect(), theme_tokens: info.theme.token_names(), capabilities })
    }

    /// Start (or restart) a draft from the installed version of a plugin.
    pub fn fork_installed(&self, name: &str) -> Result<()> {
        let installed = self.repo.capture(name).map_err(|e| match e {
            RepoError::NotInstalled(n) => ServiceError::NotInstalled(n),
            other => other.into(),
        })?;
        self.drafts.fork(name, &installed)?;
        Ok(())
    }

    /// Take an immutable copy of the draft and queue it for the user to review.
    pub fn submit(&self, name: &str) -> Result<Review> {
        let snapshot = self.drafts.snapshot(name)?;
        let manifest = manifest_of(&snapshot)?;
        check_entry(&snapshot)?;
        if manifest.name != name {
            return Err(ServiceError::Invalid(format!("the manifest name `{}` must match the draft name `{name}`", manifest.name)));
        }
        // The user should not have to read code that does not even load.
        self.sandbox_load(&snapshot).map_err(|e| ServiceError::LoadFailed(e.to_string()))?;

        let installed = self.repo.capture(name).ok();
        if installed.as_ref().is_some_and(|old| old.hash() == snapshot.hash()) {
            return Err(ServiceError::NoChange);
        }
        let new_lines = manifest.capabilities.describe();
        let mut changes = Vec::new();
        let kind = match &installed {
            Some(old) => {
                let old_lines = manifest_of(old).map(|m| m.capabilities.describe()).unwrap_or_default();
                changes.extend(new_lines.iter().filter(|l| !old_lines.contains(l)).map(|l| format!("+ {l}")));
                changes.extend(old_lines.iter().filter(|l| !new_lines.contains(l)).map(|l| format!("- {l}")));
                ReviewKind::Update
            }
            None => ReviewKind::New,
        };
        let review = Review {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind,
            hash: snapshot.hash(),
            diff: repo::diff(installed.as_ref(), &snapshot),
            capability_lines: new_lines,
            capability_changes: changes,
            full_local_access: manifest.capabilities.grants_full_local_access(),
            trust_offered: manifest.capabilities.is_ui_only(),
            manifest,
            snapshot,
        };
        let mut pending = lock(&self.pending);
        pending.retain(|r| r.name != name);
        pending.push(review.clone());
        Ok(review)
    }

    pub fn pending(&self) -> Vec<Review> {
        lock(&self.pending).clone()
    }

    /// The user approved this review. Installs exactly the reviewed bytes, records the approval, and loads the plugin.
    /// `trust_edits` is honored only when the plugin is ui-only.
    pub fn approve(&self, review_id: &str, trust_edits: bool) -> Result<LoadedInfo> {
        let review = {
            let mut pending = lock(&self.pending);
            let index = pending.iter().position(|r| r.id == review_id).ok_or_else(|| ServiceError::NoSuchReview(review_id.to_string()))?;
            pending.remove(index)
        };
        // The review holds the bytes the user saw. Check they are still intact, then use only them.
        if review.snapshot.hash() != review.hash {
            return Err(ServiceError::Invalid("the reviewed files changed in memory; refusing to install".into()));
        }
        let name = review.name.as_str();
        let manifest = manifest_of(&review.snapshot)?;
        // Fail closed before touching the disk: without a signing key an approval cannot be recorded.
        self.ledger.check_key()?;

        let old = self.repo.capture(name).ok();
        let verb = if old.is_some() { "update" } else { "install" };
        let commit = self.repo.install(name, &review.snapshot, &format!("{verb} {name} ({})", short(&review.hash)))?;
        let approval = Approval {
            name: name.to_string(),
            hash: review.hash.clone(),
            capabilities: manifest.capabilities.clone(),
            trusted_edits: trust_edits && manifest.capabilities.is_ui_only(),
            commit,
            approved_at: now(),
        };
        let undo = |this: &Self| match &old {
            Some(old) => {
                let _ = this.repo.install(name, old, &format!("restore {name} after a failed update"));
                if let Ok(Some(a)) = this.ledger.history(name).map(|h| h.into_iter().find(|a| a.hash == old.hash())) {
                    let _ = this.ledger.make_current(name, &a.hash);
                }
            }
            None => {
                let _ = this.repo.remove(name, &format!("remove {name} after a failed install"));
                let _ = this.ledger.forget(name);
            }
        };
        if let Err(e) = self.ledger.record(&approval) {
            undo(self);
            return Err(e.into());
        }
        let loaded = lock(&self.manager).load(review.snapshot.clone());
        match loaded {
            Ok(info) => {
                let _ = self.ledger.clear_failures(name);
                lock(&self.states).insert(name.to_string(), PluginState::Running);
                Ok(info)
            }
            Err(e) => {
                undo(self);
                Err(ServiceError::LoadFailed(e.to_string()))
            }
        }
    }

    pub fn reject(&self, review_id: &str) -> bool {
        let mut pending = lock(&self.pending);
        let before = pending.len();
        pending.retain(|r| r.id != review_id);
        pending.len() != before
    }

    /// Load installed plugins whose bytes match a signed approval.
    pub fn load_all(&self) -> LoadReport {
        let mut report = LoadReport { safe_mode: self.safe_mode, ..LoadReport::default() };
        let names = self.repo.list().unwrap_or_default();
        if self.safe_mode {
            let mut states = lock(&self.states);
            for name in names {
                states.insert(name, PluginState::DisabledSafeMode);
            }
            return report;
        }
        if let Err(e) = self.ledger.check_key() {
            report.key_problem = Some(e.to_string());
            let mut states = lock(&self.states);
            for name in names {
                states.insert(name, PluginState::Failed(format!("no signing key: {e}")));
            }
            return report;
        }
        for name in names {
            self.load_one(&name, &mut report);
        }
        report
    }

    fn load_one(&self, name: &str, report: &mut LoadReport) {
        let outside = |report: &mut LoadReport| {
            report.modified_outside_review.push(name.to_string());
            lock(&self.states).insert(name.to_string(), PluginState::ModifiedOutsideReview);
        };
        // Capture once. The hash is checked on these bytes and these same bytes are loaded.
        let snapshot = match self.repo.capture(name) {
            Ok(s) => s,
            Err(e) => {
                report.failed.push((name.to_string(), e.to_string()));
                return;
            }
        };
        let hash = snapshot.hash();
        let approval = match self.ledger.current(name) {
            Ok(a) => a,
            Err(e) => {
                report.failed.push((name.to_string(), e.to_string()));
                return;
            }
        };
        let Some(approval) = approval else { return outside(report) };

        if approval.hash != hash {
            // Edited outside review. Only a trusted, ui-only plugin with unchanged capabilities may skip review.
            let unchanged = manifest_of(&snapshot)
                .is_ok_and(|m| m.name == name && m.capabilities == approval.capabilities && m.capabilities.is_ui_only());
            if !(approval.trusted_edits && unchanged) {
                return outside(report);
            }
            let committed = self.repo.install(name, &snapshot, &format!("trusted edit of {name} ({})", short(&hash)));
            let recorded = committed.map_err(ServiceError::from).and_then(|commit| {
                self.ledger
                    .record(&Approval { hash: hash.clone(), commit, approved_at: now(), ..approval.clone() })
                    .map_err(ServiceError::from)
            });
            if let Err(e) = recorded {
                report.failed.push((name.to_string(), e.to_string()));
                return;
            }
            report.trusted_updates.push(name.to_string());
        }

        let loaded = lock(&self.manager).load(snapshot);
        match loaded {
            Ok(_) => {
                report.loaded.push(name.to_string());
                lock(&self.states).insert(name.to_string(), PluginState::Running);
                let _ = self.ledger.clear_failures(name);
            }
            Err(e) => {
                report.failed.push((name.to_string(), e.to_string()));
                lock(&self.states).insert(name.to_string(), PluginState::Failed(e.to_string()));
                let count = self.ledger.note_failure(name).unwrap_or(REVERT_THRESHOLD);
                if count >= REVERT_THRESHOLD && self.rollback(name).is_ok() {
                    report.reverted.push(name.to_string());
                }
            }
        }
    }

    /// Go back to the previous approved version. A plugin with no earlier version is uninstalled.
    pub fn rollback(&self, name: &str) -> Result<()> {
        if !self.repo.list()?.iter().any(|n| n == name) {
            return Err(ServiceError::NotInstalled(name.to_string()));
        }
        let current = self.ledger.current(name)?.ok_or_else(|| ServiceError::NoPreviousVersion(name.to_string()))?;
        let history = self.ledger.history(name)?;
        let position = history.iter().position(|a| a.hash == current.hash).ok_or_else(|| ServiceError::NoPreviousVersion(name.to_string()))?;
        if position == 0 {
            return self.uninstall(name);
        }
        let previous = &history[position - 1];
        let snapshot = self.repo.snapshot_at(name, &previous.commit)?;
        if snapshot.hash() != previous.hash {
            return Err(ServiceError::Invalid(format!("the stored version of `{name}` no longer matches its approval")));
        }
        self.repo.install(name, &snapshot, &format!("roll back {name} to {}", short(&previous.hash)))?;
        self.ledger.make_current(name, &previous.hash)?;
        let _ = self.ledger.clear_failures(name);
        match lock(&self.manager).load(snapshot) {
            Ok(_) => {
                lock(&self.states).insert(name.to_string(), PluginState::Running);
            }
            Err(e) => {
                lock(&self.states).insert(name.to_string(), PluginState::Failed(e.to_string()));
            }
        }
        Ok(())
    }

    /// Revert plugins that failed `REVERT_THRESHOLD` times in a row. Returns the names.
    pub fn maintain(&self) -> Vec<String> {
        let failing = lock(&self.manager).failing(REVERT_THRESHOLD);
        failing.into_iter().filter(|name| self.rollback(name).is_ok()).collect()
    }

    pub fn statuses(&self) -> Vec<PluginStatus> {
        let names = self.repo.list().unwrap_or_default();
        let states = lock(&self.states).clone();
        let manager = lock(&self.manager);
        names
            .into_iter()
            .map(|name| {
                let state = match manager.status(&name) {
                    Some(crate::runtime::Status::Terminated(reason)) => PluginState::Failed(reason),
                    Some(crate::runtime::Status::Running) => states.get(&name).cloned().unwrap_or(PluginState::Running),
                    None => states.get(&name).cloned().unwrap_or(PluginState::ModifiedOutsideReview),
                };
                let trusted_edits = self.ledger.current(&name).ok().flatten().is_some_and(|a| a.trusted_edits);
                PluginStatus { name, state, trusted_edits }
            })
            .collect()
    }

    pub fn set_trusted_edits(&self, name: &str, trusted: bool) -> Result<bool> {
        let Some(current) = self.ledger.current(name)? else { return Ok(false) };
        if trusted && !current.capabilities.is_ui_only() {
            return Ok(false);
        }
        Ok(self.ledger.set_trusted(name, trusted)?)
    }

    pub fn uninstall(&self, name: &str) -> Result<()> {
        self.repo.remove(name, &format!("uninstall {name}")).map_err(|e| match e {
            RepoError::NotInstalled(n) => ServiceError::NotInstalled(n),
            other => other.into(),
        })?;
        self.ledger.forget(name)?;
        lock(&self.manager).unload(name);
        lock(&self.states).remove(name);
        Ok(())
    }

    /// The view calls this with the state it gives to slots, so `inspect` can show what plugins draw now.
    pub fn set_ui_state(&self, state: Value) {
        *lock(&self.ui_state) = state;
    }

    /// What every slot draws right now: `{ state, slots: { name: { plugin, tree } | { builtin: true } } }`.
    pub fn inspect(&self) -> Value {
        let state = lock(&self.ui_state).clone();
        let render_state = if state.is_null() { serde_json::json!({}) } else { state.clone() };
        let mut manager = lock(&self.manager);
        let mut names: Vec<String> = ["channel_header", "member_list"].iter().map(|s| s.to_string()).collect();
        names.extend(manager.slot_names());
        names.sort();
        names.dedup();
        let mut slots = serde_json::Map::new();
        for name in names {
            let entry = match manager.render_slot(&name, &render_state) {
                Some(r) => serde_json::json!({"plugin": r.plugin, "tree": r.element}),
                None => serde_json::json!({"builtin": true}),
            };
            slots.insert(name, entry);
        }
        serde_json::json!({"state": state, "slots": slots})
    }

    /// See `PluginManager::theme_sources`.
    pub fn theme_sources(&self) -> Vec<(String, String)> {
        self.manager.lock().unwrap_or_else(|p| p.into_inner()).theme_sources()
    }

    pub fn manager(&self) -> &Arc<Mutex<PluginManager>> {
        &self.manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{StaticKeyStore, UnavailableKeyStore};
    use crate::runtime::PluginLimits;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::Duration;

    struct Rig {
        svc: PluginService,
        dir: tempfile::TempDir,
    }

    fn manifest(name: &str, caps: &str) -> String {
        format!("name = \"{name}\"\nversion = \"1\"\napi = \"1\"\n{caps}")
    }

    const UI: &str = "[capabilities]\nui = true\n";

    fn files(name: &str, caps: &str, init: &str) -> BTreeMap<String, String> {
        [("plugin.toml".to_string(), manifest(name, caps)), ("init.lua".to_string(), init.to_string())].into()
    }

    fn limits() -> PluginLimits {
        PluginLimits { call_budget: Duration::from_millis(150), ..PluginLimits::default() }
    }

    fn rig_with(keys: Arc<dyn crate::keystore::KeyStore>, safe_mode: bool, dir: Option<tempfile::TempDir>) -> Rig {
        let dir = dir.unwrap_or_else(|| tempfile::tempdir().unwrap());
        let ledger = Ledger::open(&dir.path().join("plugins.db"), keys).unwrap();
        let manager = Arc::new(Mutex::new(PluginManager::new(limits())));
        let svc = PluginService::new(dir.path(), ledger, manager, safe_mode).unwrap();
        Rig { svc, dir }
    }

    fn rig() -> Rig {
        rig_with(Arc::new(StaticKeyStore([5; 32])), false, None)
    }

    fn header(text: &str) -> String {
        format!("discard.slot(\"channel_header\", function() return {{ \"text\", \"{text}\" }} end)")
    }

    fn submit_and_approve(r: &Rig, name: &str, caps: &str, init: &str) -> LoadedInfo {
        r.svc.drafts().create(name, &files(name, caps, init)).unwrap();
        let review = r.svc.submit(name).unwrap();
        r.svc.approve(&review.id, false).unwrap()
    }

    fn render_text(r: &Rig, slot: &str) -> Option<String> {
        let rendered = r.svc.manager().lock().unwrap().render_slot(slot, &json!({}))?;
        match &rendered.element.children[0] {
            crate::element::Node::Text(t) => Some(t.clone()),
            _ => None,
        }
    }

    fn live_file(r: &Rig, name: &str, file: &str) -> std::path::PathBuf {
        r.dir.path().join("plugins").join(name).join(file)
    }

    // ---- drafts never run ----

    #[test]
    fn a_draft_is_not_loaded_or_installed_until_approved() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("draft"))).unwrap();
        r.svc.submit("p").unwrap();
        assert_eq!(render_text(&r, "channel_header"), None);
        assert!(r.svc.manager().lock().unwrap().names().is_empty());
        assert!(!r.dir.path().join("plugins/p").exists());
    }

    // ---- test_draft ----

    #[test]
    fn test_draft_renders_slots_in_a_sandbox_and_installs_nothing() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &(header("hi") + "\ndiscard.command(\"c\", function() end) discard.theme({ primary = \"#112233\" })"))).unwrap();
        let t = r.svc.test_draft("p", &json!({})).unwrap();
        assert!(t.loaded && t.error.is_none());
        assert_eq!(t.slots.len(), 1);
        assert_eq!(t.slots[0].0, "channel_header");
        assert!(t.slots[0].1.is_ok());
        assert_eq!((t.commands.clone(), t.theme_tokens.clone()), (vec!["c".to_string()], vec!["primary".to_string()]));
        assert!(t.capabilities.iter().any(|c| c.contains("ui")));
        assert!(r.svc.manager().lock().unwrap().names().is_empty());
    }

    #[test]
    fn test_draft_reports_load_errors_and_render_errors_without_failing() {
        let r = rig();
        r.svc.drafts().create("bad", &files("bad", UI, "this is not lua")).unwrap();
        let t = r.svc.test_draft("bad", &json!({})).unwrap();
        assert!(!t.loaded);
        assert!(t.error.is_some());
        r.svc.drafts().create("slow", &files("slow", UI, "discard.slot(\"s\", function() error(\"boom\") end)")).unwrap();
        let t = r.svc.test_draft("slow", &json!({})).unwrap();
        assert!(t.loaded);
        assert!(t.slots[0].1.as_ref().unwrap_err().contains("boom"));
    }

    // ---- submit ----

    #[test]
    fn submit_builds_a_review_with_diff_capabilities_and_a_hash() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("v1"))).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert_eq!((review.name.as_str(), review.kind), ("p", ReviewKind::New));
        assert_eq!(review.hash.len(), 64);
        assert!(review.diff.contains("+++ init.lua") && review.diff.contains("v1"), "{}", review.diff);
        assert!(review.capability_lines.iter().any(|l| l.contains("ui")));
        assert!(review.trust_offered && !review.full_local_access);
        assert_eq!(r.svc.pending().len(), 1);
    }

    #[test]
    fn submitting_the_same_draft_again_replaces_the_pending_review() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("v1"))).unwrap();
        let first = r.svc.submit("p").unwrap();
        r.svc.drafts().write_file("p", "init.lua", &header("v2")).unwrap();
        let second = r.svc.submit("p").unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(r.svc.pending().len(), 1);
        assert_eq!(r.svc.pending()[0].id, second.id);
        assert!(r.svc.approve(&first.id, false).is_err(), "the old review is gone");
    }

    #[test]
    fn submit_rejects_invalid_drafts_before_a_user_has_to_read_them() {
        let r = rig();
        r.svc.drafts().create("a", &[("init.lua".to_string(), "x".to_string())].into()).unwrap();
        assert!(matches!(r.svc.submit("a"), Err(ServiceError::Invalid(m)) if m.contains("plugin.toml")));
        r.svc.drafts().create("b", &files("other-name", UI, &header("x"))).unwrap();
        assert!(matches!(r.svc.submit("b"), Err(ServiceError::Invalid(m)) if m.contains("other-name")));
        r.svc.drafts().create("c", &files("c", UI, "this is not lua")).unwrap();
        assert!(matches!(r.svc.submit("c"), Err(ServiceError::LoadFailed(_))));
        r.svc.drafts().create("d", &[("plugin.toml".to_string(), manifest("d", UI))].into()).unwrap();
        assert!(matches!(r.svc.submit("d"), Err(ServiceError::Invalid(m)) if m.contains("init.lua")));
        assert!(r.svc.pending().is_empty());
    }

    #[test]
    fn a_draft_with_no_changes_from_the_installed_plugin_is_not_submitted() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        assert!(matches!(r.svc.submit("p"), Err(ServiceError::NoChange)));
    }

    #[test]
    fn an_update_review_shows_what_changed_and_which_capabilities_were_added_or_dropped() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        let wider = "[capabilities]\nui = true\n[capabilities.exec]\ncommands = [\"git\"]\n";
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "plugin.toml", &manifest("p", wider)).unwrap();
        r.svc.drafts().write_file("p", "init.lua", &header("v2")).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert_eq!(review.kind, ReviewKind::Update);
        assert!(review.diff.contains("-") && review.diff.contains("v1") && review.diff.contains("v2"), "{}", review.diff);
        assert!(review.capability_changes.iter().any(|l| l.starts_with('+') && l.contains("full local access")), "{:?}", review.capability_changes);
        assert!(review.full_local_access);
        assert!(!review.trust_offered, "trust is never offered for a plugin that can run programs");
    }

    // ---- approve ----

    #[test]
    fn approving_installs_commits_records_and_loads_the_reviewed_plugin() {
        let r = rig();
        let info = submit_and_approve(&r, "p", UI, &header("v1"));
        assert_eq!(info.manifest.name, "p");
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v1"));
        assert!(r.dir.path().join("plugins/p/init.lua").exists());
        let approval = r.svc.ledger.current("p").unwrap().unwrap();
        assert_eq!(approval.hash, info.hash);
        assert_eq!(approval.commit.len(), 40);
        assert!(r.svc.pending().is_empty());
    }

    #[test]
    fn approval_installs_the_bytes_that_were_reviewed_even_if_the_draft_changes_afterwards() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("reviewed"))).unwrap();
        let review = r.svc.submit("p").unwrap();
        // The agent keeps editing after the user opened the review.
        r.svc.drafts().write_file("p", "init.lua", &header("SNEAKY CHANGE")).unwrap();
        r.svc.approve(&review.id, false).unwrap();
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("reviewed"));
        assert_eq!(std::fs::read_to_string(live_file(&r, "p", "init.lua")).unwrap(), header("reviewed"));
    }

    #[test]
    fn trust_for_future_edits_is_honored_only_for_ui_only_plugins() {
        let r = rig();
        r.svc.drafts().create("ui", &files("ui", UI, &header("x"))).unwrap();
        let id = r.svc.submit("ui").unwrap().id;
        r.svc.approve(&id, true).unwrap();
        assert!(r.svc.ledger.current("ui").unwrap().unwrap().trusted_edits);

        let wide = "[capabilities]\nui = true\napprovals_decide = true\n";
        r.svc.drafts().create("wide", &files("wide", wide, &header("x"))).unwrap();
        let id = r.svc.submit("wide").unwrap().id;
        r.svc.approve(&id, true).unwrap();
        assert!(!r.svc.ledger.current("wide").unwrap().unwrap().trusted_edits, "approvals.decide plugins are never trusted");
    }

    #[test]
    fn approving_an_update_replaces_the_running_version() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "init.lua", &header("v2")).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert!(matches!(r.svc.approve("no-such-review", false), Err(ServiceError::NoSuchReview(_))));
        r.svc.approve(&review.id, false).unwrap();
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v2"));
    }

    /// A plugin that loads in a generous sandbox but not under the live limits.
    const SLOW_INIT: &str = "local t = os.clock() while os.clock() - t < 0.4 do end";

    fn strict_rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("plugins.db"), Arc::new(StaticKeyStore([5; 32]))).unwrap();
        let manager = Arc::new(Mutex::new(PluginManager::new(PluginLimits { call_budget: Duration::from_millis(100), ..PluginLimits::default() })));
        let generous = PluginLimits { call_budget: Duration::from_secs(5), ..PluginLimits::default() };
        let svc = PluginService::new(dir.path(), ledger, manager, false).unwrap().with_sandbox_limits(generous);
        Rig { svc, dir }
    }

    #[test]
    fn a_new_plugin_that_fails_to_load_for_real_is_not_left_installed_or_approved() {
        let r = strict_rig();
        r.svc.drafts().create("p", &files("p", UI, SLOW_INIT)).unwrap();
        let review = r.svc.submit("p").expect("the generous sandbox accepts it");
        assert!(matches!(r.svc.approve(&review.id, false), Err(ServiceError::LoadFailed(_))));
        assert!(!r.dir.path().join("plugins/p").exists(), "the failed install must be removed");
        assert_eq!(r.svc.ledger.current("p").unwrap(), None, "and its approval forgotten");
        assert!(r.svc.manager().lock().unwrap().names().is_empty());
    }

    #[test]
    fn an_update_that_fails_to_load_for_real_leaves_the_old_version_installed_approved_and_running() {
        let r = strict_rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "init.lua", &format!("{SLOW_INIT}\n{}", header("v2"))).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert!(matches!(r.svc.approve(&review.id, false), Err(ServiceError::LoadFailed(_))));
        assert_eq!(std::fs::read_to_string(live_file(&r, "p", "init.lua")).unwrap(), header("v1"));
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v1"), "the old version keeps running");
        let current = r.svc.ledger.current("p").unwrap().unwrap();
        assert_eq!(current.hash, r.svc.repo.capture("p").unwrap().hash(), "the old version is the approved one");
    }

    #[test]
    fn reject_discards_the_review_without_installing() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("x"))).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert!(r.svc.reject(&review.id));
        assert!(!r.svc.reject(&review.id));
        assert!(r.svc.pending().is_empty());
        assert!(!r.dir.path().join("plugins/p").exists());
    }

    #[test]
    fn without_a_signing_key_nothing_can_be_approved() {
        let r = rig_with(Arc::new(UnavailableKeyStore("no keychain".into())), false, None);
        r.svc.drafts().create("p", &files("p", UI, &header("x"))).unwrap();
        let review = r.svc.submit("p").unwrap();
        assert!(r.svc.approve(&review.id, false).is_err());
        assert!(!r.dir.path().join("plugins/p").exists(), "a failed approval must not leave code installed");
        assert!(r.svc.manager().lock().unwrap().names().is_empty());
    }

    // ---- load_all ----

    fn restart(r: Rig, safe_mode: bool) -> Rig {
        let Rig { svc, dir } = r;
        drop(svc);
        rig_with(Arc::new(StaticKeyStore([5; 32])), safe_mode, Some(dir))
    }

    #[test]
    fn approved_plugins_load_again_after_a_restart() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        let r = restart(r, false);
        let report = r.svc.load_all();
        assert_eq!(report.loaded, ["p"]);
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v1"));
        assert_eq!(r.svc.statuses()[0].state, PluginState::Running);
    }

    #[test]
    fn a_plugin_edited_on_disk_after_approval_is_not_loaded() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("approved"));
        std::fs::write(live_file(&r, "p", "init.lua"), header("TAMPERED")).unwrap();
        let r = restart(r, false);
        let report = r.svc.load_all();
        assert_eq!(report.modified_outside_review, ["p"]);
        assert!(report.loaded.is_empty());
        assert_eq!(render_text(&r, "channel_header"), None);
        assert_eq!(r.svc.statuses()[0].state, PluginState::ModifiedOutsideReview);
    }

    #[test]
    fn a_plugin_dropped_into_the_directory_without_review_is_not_loaded() {
        let r = rig();
        let dir = r.dir.path().join("plugins/sneaky");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), manifest("sneaky", UI)).unwrap();
        std::fs::write(dir.join("init.lua"), header("evil")).unwrap();
        let report = r.svc.load_all();
        assert_eq!(report.modified_outside_review, ["sneaky"]);
        assert_eq!(render_text(&r, "channel_header"), None);
    }

    #[test]
    fn a_forged_ledger_row_does_not_approve_tampered_code() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("approved"));
        std::fs::write(live_file(&r, "p", "init.lua"), header("TAMPERED")).unwrap();
        // An attacker who can edit the database tries to bless the new bytes.
        let evil = r.svc.repo.capture("p").unwrap().hash();
        {
            let conn = rusqlite::Connection::open(r.dir.path().join("plugins.db")).unwrap();
            conn.execute("UPDATE approvals SET hash = ?1 WHERE name = 'p'", [&evil]).unwrap();
            conn.execute("UPDATE current SET hash = ?1 WHERE name = 'p'", [&evil]).unwrap();
        }
        let r = restart(r, false);
        assert_eq!(r.svc.load_all().modified_outside_review, ["p"]);
        assert_eq!(render_text(&r, "channel_header"), None);
    }

    #[test]
    fn if_the_signing_key_changes_or_is_missing_no_user_plugin_loads() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        let Rig { svc, dir } = r;
        drop(svc);
        let other = rig_with(Arc::new(StaticKeyStore([9; 32])), false, Some(dir));
        assert_eq!(other.svc.load_all().modified_outside_review, ["p"], "approvals signed with another key are not trusted");
        let Rig { svc, dir } = other;
        drop(svc);
        let none = rig_with(Arc::new(UnavailableKeyStore("no keychain".into())), false, Some(dir));
        let report = none.svc.load_all();
        assert!(report.loaded.is_empty());
        assert!(report.key_problem.as_deref().is_some_and(|m| m.contains("no keychain")), "{report:?}");
    }

    #[test]
    fn safe_mode_loads_no_user_plugins_and_says_so() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        let r = restart(r, true);
        let report = r.svc.load_all();
        assert!(report.safe_mode && report.loaded.is_empty());
        assert_eq!(r.svc.statuses()[0].state, PluginState::DisabledSafeMode);
        assert_eq!(render_text(&r, "channel_header"), None);
    }

    #[test]
    fn edits_to_a_trusted_ui_only_plugin_load_without_review_and_are_committed() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("v1"))).unwrap();
        let id = r.svc.submit("p").unwrap().id;
        r.svc.approve(&id, true).unwrap();
        let before = r.svc.ledger.current("p").unwrap().unwrap().hash;
        std::fs::write(live_file(&r, "p", "init.lua"), header("v2 by trusted edit")).unwrap();
        let r = restart(r, false);
        let report = r.svc.load_all();
        assert_eq!(report.trusted_updates, ["p"]);
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v2 by trusted edit"));
        let after = r.svc.ledger.current("p").unwrap().unwrap();
        assert_ne!(after.hash, before);
        assert!(after.trusted_edits);
        assert_eq!(r.svc.repo.log("p").unwrap().len(), 2, "the trusted edit is a commit");
    }

    #[test]
    fn a_trusted_plugin_that_changes_its_capabilities_needs_review_again() {
        let r = rig();
        r.svc.drafts().create("p", &files("p", UI, &header("v1"))).unwrap();
        let id = r.svc.submit("p").unwrap().id;
        r.svc.approve(&id, true).unwrap();
        let wider = "[capabilities]\nui = true\n[capabilities.exec]\ncommands = [\"sh\"]\n";
        std::fs::write(live_file(&r, "p", "plugin.toml"), manifest("p", wider)).unwrap();
        let r = restart(r, false);
        let report = r.svc.load_all();
        assert_eq!(report.modified_outside_review, ["p"]);
        assert!(report.trusted_updates.is_empty());
    }

    #[test]
    fn edits_to_an_untrusted_plugin_always_need_review() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        std::fs::write(live_file(&r, "p", "init.lua"), header("v2")).unwrap();
        let r = restart(r, false);
        assert_eq!(r.svc.load_all().modified_outside_review, ["p"]);
    }

    // ---- rollback and auto-revert ----

    #[test]
    fn rollback_returns_to_the_previous_approved_version_and_loads_it() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "init.lua", &header("v2")).unwrap();
        let id = r.svc.submit("p").unwrap().id;
        r.svc.approve(&id, false).unwrap();
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v2"));
        r.svc.rollback("p").unwrap();
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("v1"));
        assert_eq!(std::fs::read_to_string(live_file(&r, "p", "init.lua")).unwrap(), header("v1"));
        let r = restart(r, false);
        assert_eq!(r.svc.load_all().loaded, ["p"], "the rolled-back version stays approved across restarts");
    }

    #[test]
    fn rolling_back_a_plugin_with_one_version_uninstalls_it() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.rollback("p").unwrap();
        assert!(!r.dir.path().join("plugins/p").exists());
        assert_eq!(render_text(&r, "channel_header"), None);
        assert!(matches!(r.svc.rollback("p"), Err(ServiceError::NotInstalled(_))));
    }

    #[test]
    fn a_plugin_that_fails_three_times_in_a_row_is_reverted_by_maintain() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("good"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "init.lua", "discard.slot(\"channel_header\", function() error(\"broken\") end)").unwrap();
        let id = r.svc.submit("p").unwrap().id;
        r.svc.approve(&id, false).unwrap();
        for _ in 0..REVERT_THRESHOLD {
            assert_eq!(render_text(&r, "channel_header"), None);
        }
        assert_eq!(r.svc.maintain(), ["p"]);
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("good"));
        assert!(r.svc.maintain().is_empty(), "nothing left to revert");
    }

    #[test]
    fn a_plugin_that_loops_forever_is_reverted_at_once() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("good"));
        r.svc.drafts().fork("p", &r.svc.repo.capture("p").unwrap()).unwrap();
        r.svc.drafts().write_file("p", "init.lua", "discard.slot(\"channel_header\", function() while true do end end)").unwrap();
        let id = r.svc.submit("p").unwrap().id;
        r.svc.approve(&id, false).unwrap();
        assert_eq!(render_text(&r, "channel_header"), None);
        assert_eq!(r.svc.maintain(), ["p"]);
        assert_eq!(render_text(&r, "channel_header").as_deref(), Some("good"));
    }

    // ---- inspect ----

    #[test]
    fn inspect_shows_which_plugin_draws_each_slot_and_the_tree() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("mine"));
        r.svc.set_ui_state(json!({"channel": "general"}));
        let v = r.svc.inspect();
        assert_eq!(v["state"]["channel"], "general");
        assert_eq!(v["slots"]["channel_header"]["plugin"], "p");
        assert_eq!(v["slots"]["channel_header"]["tree"]["tag"], "text");
        assert_eq!(v["slots"]["member_list"], json!({"builtin": true}), "a slot no plugin registered is drawn by Discard");
    }

    #[test]
    fn inspect_works_before_any_state_is_set_and_with_no_plugins() {
        let v = rig().svc.inspect();
        assert!(v["state"].is_null());
        assert_eq!(v["slots"]["channel_header"], json!({"builtin": true}));
    }

    #[test]
    fn inspect_lists_slots_that_plugins_invented() {
        let r = rig();
        submit_and_approve(&r, "p", UI, "discard.slot(\"status_bar\", function() return { \"text\", \"x\" } end)");
        assert_eq!(r.svc.inspect()["slots"]["status_bar"]["plugin"], "p");
    }

    // ---- misc ----

    #[test]
    fn uninstall_removes_files_approval_and_the_running_plugin() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        r.svc.uninstall("p").unwrap();
        assert!(!r.dir.path().join("plugins/p").exists());
        assert_eq!(r.svc.ledger.current("p").unwrap(), None);
        assert!(r.svc.manager().lock().unwrap().names().is_empty());
    }

    #[test]
    fn set_trusted_edits_only_works_for_ui_only_plugins() {
        let r = rig();
        submit_and_approve(&r, "p", UI, &header("v1"));
        assert!(r.svc.set_trusted_edits("p", true).unwrap());
        assert!(r.svc.ledger.current("p").unwrap().unwrap().trusted_edits);
        let wide = "[capabilities]\nui = true\napprovals_decide = true\n";
        submit_and_approve(&r, "w", wide, &header("x"));
        assert!(!r.svc.set_trusted_edits("w", true).unwrap());
    }
}
