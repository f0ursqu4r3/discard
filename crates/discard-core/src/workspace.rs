//! Per-channel workspaces on top of git worktrees: records them, prepares them for the engine, archives them safely.

use crate::engine::{PreparedWorkspace, WorkspaceService};
use crate::event::SessionId;
use crate::model::{ChannelId, WorkspaceMode};
use crate::store::{Store, StoreError, WorktreeRecord};
use crate::worktree::{Worktree, WorktreeError, WorktreeManager};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct GitWorkspace {
    manager: WorktreeManager,
    store: Arc<Mutex<Store>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveOutcome {
    /// The channel is archived. If it had a worktree, the directory is gone and the branch was kept.
    Archived { kept_branch: Option<String> },
    /// Removing the worktree would lose work. Nothing was changed. Ask the user, then call again with `force`.
    NeedsConfirmation(Vec<String>),
}

impl GitWorkspace {
    pub fn new(worktrees_dir: &Path, store: Arc<Mutex<Store>>) -> Self {
        Self { manager: WorktreeManager::new(worktrees_dir), store }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn archive(&self, channel: &ChannelId, force: bool) -> Result<ArchiveOutcome, String> {
        let record = self.store().worktree_for(channel).map_err(|e| e.to_string())?;
        let Some(record) = record else {
            self.store().archive_channel(channel).map_err(|e| e.to_string())?;
            return Ok(ArchiveOutcome::Archived { kept_branch: None });
        };
        let worktree = Worktree { root: PathBuf::from(&record.path), branch: record.branch.clone(), base_commit: record.base_commit.clone(), repo: PathBuf::from(&record.repo) };
        match self.manager.remove(&worktree, force) {
            Ok(()) => {}
            Err(WorktreeError::WouldLoseWork(items)) => return Ok(ArchiveOutcome::NeedsConfirmation(items)),
            Err(e) => return Err(e.to_string()),
        }
        let store = self.store();
        store.delete_worktree(channel).map_err(|e| e.to_string())?;
        store.archive_channel(channel).map_err(|e| e.to_string())?;
        Ok(ArchiveOutcome::Archived { kept_branch: Some(record.branch) })
    }
}

impl WorkspaceService for GitWorkspace {
    fn prepare(&self, project_dir: &Path, channel: &ChannelId, channel_name: &str) -> Result<PreparedWorkspace, String> {
        let prepared = self.manager.prepare(project_dir, &channel.0, channel_name).map_err(|e| e.to_string())?;
        let wt = &prepared.worktree;
        let record = WorktreeRecord {
            channel_id: channel.clone(),
            path: wt.root.to_string_lossy().into_owned(),
            branch: wt.branch.clone(),
            base_commit: wt.base_commit.clone(),
            repo: wt.repo.to_string_lossy().into_owned(),
        };
        self.store().save_worktree(&record).map_err(|e| e.to_string())?;
        Ok(PreparedWorkspace { cwd: prepared.cwd, notes: prepared.notes })
    }
}

/// Change where a channel works. The provider's old session belonged to the old directory, so this
/// always starts a new session. Returns it.
pub fn change_workspace_mode(store: &Store, channel: &ChannelId, mode: WorkspaceMode) -> Result<SessionId, StoreError> {
    store.set_workspace_mode(channel, mode)?;
    let session = SessionId::new();
    store.create_session(&session, channel)?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::capability::ApprovalPolicy;
    use crate::model::{AgentId, Channel, Project, ProjectId};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Rig {
        ws: GitWorkspace,
        store: Arc<Mutex<Store>>,
        repo: PathBuf,
        channel: Channel,
        _dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "p".into(), path: repo.to_string_lossy().into() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store.insert_agent(&agent, &AgentConfig { name: "A".into(), avatar: None, provider: "claude".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: project.id, name: "general".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
        store.insert_channel(&channel).unwrap();
        store.create_session(&SessionId::new(), &channel.id).unwrap();
        let store = Arc::new(Mutex::new(store));
        let ws = GitWorkspace::new(&dir.path().join("worktrees"), Arc::clone(&store));
        Rig { ws, store, repo, channel, _dir: dir }
    }

    #[test]
    fn prepare_creates_the_worktree_records_it_and_returns_its_directory() {
        let r = rig();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        assert!(prepared.cwd.join("a.txt").exists());
        let rec = r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().expect("a record");
        assert_eq!(PathBuf::from(&rec.path), prepared.cwd);
        assert!(rec.branch.starts_with("discard/general-"));
        assert_eq!(rec.base_commit, git(&r.repo, &["rev-parse", "HEAD"]));
    }

    #[test]
    fn prepare_is_idempotent_and_keeps_one_record() {
        let r = rig();
        let first = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        std::fs::write(first.cwd.join("wip.txt"), "x").unwrap();
        let second = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        assert_eq!(first.cwd, second.cwd);
        assert!(second.cwd.join("wip.txt").exists());
        assert!(second.notes.is_empty());
    }

    #[test]
    fn prepare_passes_on_notes_about_uncommitted_files() {
        let r = rig();
        std::fs::write(r.repo.join("a.txt"), "dirty\n").unwrap();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        assert!(prepared.notes.iter().any(|n| n.contains("not copied")));
    }

    #[test]
    fn prepare_errors_are_readable() {
        let r = rig();
        let plain = r._dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let e = r.ws.prepare(&plain, &r.channel.id, "general").unwrap_err();
        assert!(e.contains("not inside a git repository"), "{e}");
        let empty = r._dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        git(&empty, &["init", "-q"]);
        assert!(r.ws.prepare(&empty, &r.channel.id, "general").unwrap_err().contains("no commits"));
    }

    #[test]
    fn archiving_a_clean_worktree_removes_it_keeps_the_branch_and_hides_the_channel() {
        let r = rig();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        let branch = r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().unwrap().branch;
        let out = r.ws.archive(&r.channel.id, false).unwrap();
        assert_eq!(out, ArchiveOutcome::Archived { kept_branch: Some(branch.clone()) });
        assert!(!prepared.cwd.exists());
        assert!(git(&r.repo, &["branch", "--list", &branch]).contains(&branch));
        let store = r.store.lock().unwrap();
        assert_eq!(store.worktree_for(&r.channel.id).unwrap(), None);
        assert!(store.channels_for_project(&r.channel.project_id).unwrap().is_empty());
    }

    #[test]
    fn archiving_a_dirty_worktree_asks_first_and_changes_nothing() {
        let r = rig();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        std::fs::write(prepared.cwd.join("precious.txt"), "do not lose").unwrap();
        let out = r.ws.archive(&r.channel.id, false).unwrap();
        let ArchiveOutcome::NeedsConfirmation(items) = out else { panic!("expected a confirmation request") };
        assert!(items.iter().any(|i| i.contains("precious.txt")), "{items:?}");
        assert!(prepared.cwd.join("precious.txt").exists());
        let store = r.store.lock().unwrap();
        assert!(store.worktree_for(&r.channel.id).unwrap().is_some());
        assert_eq!(store.channels_for_project(&r.channel.project_id).unwrap().len(), 1, "the channel is still there");
    }

    #[test]
    fn a_confirmed_archive_removes_the_worktree_even_with_loose_files() {
        let r = rig();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "general").unwrap();
        std::fs::write(prepared.cwd.join("loose.txt"), "x").unwrap();
        assert!(matches!(r.ws.archive(&r.channel.id, true), Ok(ArchiveOutcome::Archived { .. })));
        assert!(!prepared.cwd.exists());
    }

    #[test]
    fn archiving_a_channel_without_a_worktree_just_hides_it() {
        let r = rig();
        assert_eq!(r.ws.archive(&r.channel.id, false).unwrap(), ArchiveOutcome::Archived { kept_branch: None });
        assert!(r.store.lock().unwrap().channels_for_project(&r.channel.project_id).unwrap().is_empty());
    }

    #[test]
    fn changing_the_workspace_mode_starts_a_new_session() {
        let r = rig();
        let store = r.store.lock().unwrap();
        let old = store.latest_session(&r.channel.id).unwrap().unwrap();
        let new = change_workspace_mode(&store, &r.channel.id, WorkspaceMode::MainCheckout).unwrap();
        assert_ne!(old, new, "the old provider session belonged to the old directory");
        assert_eq!(store.latest_session(&r.channel.id).unwrap(), Some(new));
        assert_eq!(store.channels_for_project(&r.channel.project_id).unwrap()[0].workspace_mode, WorkspaceMode::MainCheckout);
    }
}
