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

    /// Select an existing checkout for conversation context without creating a worktree.
    pub fn conversation_directory(&self, project_dir: &Path, channel: &ChannelId) -> Result<PathBuf, String> {
        let store = self.store();
        let metadata = store.project_channel_metadata(channel).map_err(|e| e.to_string())?;
        let Some(metadata) = metadata.filter(|metadata| matches!(metadata.kind,
            crate::model::ProjectChannelKind::Branch | crate::model::ProjectChannelKind::Legacy)) else {
            return Ok(project_dir.to_path_buf());
        };
        if let Some(record) = store.worktree_for(channel).map_err(|e| e.to_string())? {
            return Ok(PathBuf::from(record.path));
        }
        Ok(if metadata.repository.is_empty() { project_dir.to_path_buf() } else { PathBuf::from(metadata.repository) })
    }

    pub fn target_for_branch(&self, repo: &Path, channel: &ChannelId, branch: &str) -> Result<PathBuf, String> {
        self.manager.target_for_branch(repo, &channel.0, branch).map_err(|e| e.to_string())
    }

    pub fn create_for_branch(&self, repo: &Path, channel: &ChannelId, branch: &str) -> Result<WorktreeRecord, String> {
        if self.store().worktree_for(channel).map_err(|e| e.to_string())?.is_some() { return Err("This channel already has a worktree.".into()); }
        let prepared = self.manager.prepare_owned_branch(repo, &channel.0, branch).map_err(|e| match e { WorktreeError::Git(message) => message, other => other.to_string() })?;
        let wt = prepared.worktree;
        let record = WorktreeRecord {
            channel_id: channel.clone(),
            path: wt.root.to_string_lossy().into_owned(),
            branch: wt.branch.clone(),
            base_commit: wt.base_commit.clone(),
            repo: wt.repo.to_string_lossy().into_owned(),
            owned: true,
        };
        let saved = { let store = self.store();
            if store.worktree_for(channel).map_err(|e| e.to_string())?.is_some() { Err("This channel already has a worktree.".into()) }
            else { store.save_worktree(&record).map_err(|e| e.to_string()) }
        };
        if let Err(error) = saved { let _ = self.manager.remove(&wt, false); return Err(error); }
        Ok(record)
    }

    /// Resolve saved authority under the store lock, then verify Git outside it.
    pub fn write_request_target(&self, channel: &ChannelId, agent: &crate::model::AgentId, project: Option<&crate::model::ProjectId>, kind: crate::write_requests::WriteRequestKind) -> Result<(PathBuf, String, PathBuf), String> {
        let (repo, branch, binding) = {
            let store = self.store();
            let actual = store.load_channel(channel).map_err(|e| e.to_string())?;
            if project.is_none() || actual.project_id.as_ref() != project { return Err("Write requests require this conversation's project.".into()); }
            if store.channel_archived(channel).map_err(|e| e.to_string())? || store.agent_archived(agent).map_err(|e| e.to_string())? { return Err("This agent or channel is archived.".into()); }
            let config = store.load_agent(agent).map_err(|e| e.to_string())?;
            if !config.workspace_write || config.policy != crate::capability::ApprovalPolicy::Ask { return Err("This agent is not allowed to edit files.".into()); }
            let metadata = store.project_channel_metadata(channel).map_err(|e| e.to_string())?.filter(|m| m.kind == crate::model::ProjectChannelKind::Branch).ok_or("Write requests require a branch channel.")?;
            let branch = metadata.branch.ok_or("This channel has no branch.")?;
            let repo = if metadata.repository.is_empty() { store.list_projects().map_err(|e| e.to_string())?.into_iter().find(|p| Some(&p.id) == project).ok_or("Project unavailable.")?.path } else { metadata.repository };
            (PathBuf::from(repo), branch, store.worktree_for(channel).map_err(|e| e.to_string())?)
        };
        let entries = crate::worktree::list_branches(&repo).map_err(|e| e.to_string())?;
        let entry = entries.iter().find(|e| e.branch == branch).ok_or("This branch no longer exists.")?;
        match kind {
            crate::write_requests::WriteRequestKind::WorktreeCreate => {
                if entry.is_main_checkout { return Err("This branch is checked out in the main checkout. Use main_checkout_request_write instead.".into()); }
                if binding.is_some() { return Err("This channel already has a worktree.".into()); }
                if entry.worktree.is_some() { return Err("This branch is checked out in another folder. Ask again after it is free.".into()); }
                let target = self.target_for_branch(&repo, channel, &branch)?;
                Ok((repo, branch, target))
            }
            crate::write_requests::WriteRequestKind::MainCheckoutWrite => {
                let record = binding.ok_or("This channel has no saved worktree binding.")?;
                let root = entry.worktree.as_ref().filter(|_| entry.is_main_checkout).ok_or("This branch is not checked out in the main checkout.")?.canonicalize().map_err(|e| e.to_string())?;
                if Path::new(&record.path).canonicalize().map_err(|e| e.to_string())? != root || record.branch != branch {
                    return Err("The main-checkout binding changed. Ask again.".into());
                }
                Ok((repo, branch, root))
            },
        }
    }

    pub fn archive(&self, channel: &ChannelId, force: bool) -> Result<ArchiveOutcome, String> {
        let record = self.store().worktree_for(channel).map_err(|e| e.to_string())?;
        let Some(record) = record else {
            self.store().archive_channel(channel).map_err(|e| e.to_string())?;
            return Ok(ArchiveOutcome::Archived { kept_branch: None });
        };
        // A channel that borrows a checkout never owns its directory, even with force.
        let shared = {
            let store = self.store();
            let mut shared = false;
            for project in store.list_projects().map_err(|e| e.to_string())? {
                for other in store.channels_for_project(&project.id).map_err(|e| e.to_string())? {
                    if other.id != *channel && store.worktree_for(&other.id).map_err(|e| e.to_string())?.is_some_and(|w| w.path == record.path) { shared = true; }
                }
            }
            shared
        };
        if !record.owned || shared {
            self.store().archive_channel(channel).map_err(|e| e.to_string())?;
            return Ok(ArchiveOutcome::Archived { kept_branch: Some(record.branch) });
        }
        if Path::new(&record.path).exists() { self.manager.recorded(Path::new(&record.repo), Path::new(&record.path), &record.branch).map_err(|e| e.to_string())?; }
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
        let (record, branch, metadata) = { let store = self.store();
            (store.worktree_for(channel).map_err(|e| e.to_string())?,
             store.channel_branch(channel).map_err(|e| e.to_string())?,
             store.project_channel_metadata(channel).map_err(|e| e.to_string())?) };
        let repository = metadata.map(|metadata| metadata.repository).unwrap_or_default();
        let project_dir = if repository.is_empty() { project_dir } else { Path::new(&repository) };
        if let Some(record) = record {
            let prepared = self.manager.recorded(Path::new(&record.repo), Path::new(&record.path), &record.branch).map_err(|e| e.to_string())?;
            return Ok(PreparedWorkspace { cwd: prepared.cwd, notes: prepared.notes });
        }
        let prepared = match branch {
            Some(branch) => self.manager.prepare_branch(project_dir, &channel.0, &branch),
            None => self.manager.prepare(project_dir, &channel.0, channel_name),
        }.map_err(|e| e.to_string())?;
        let wt = &prepared.worktree;
        let record = WorktreeRecord {
            channel_id: channel.clone(),
            path: wt.root.to_string_lossy().into_owned(),
            branch: wt.branch.clone(),
            base_commit: wt.base_commit.clone(),
            repo: wt.repo.to_string_lossy().into_owned(), owned: prepared.created };
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
        let out = crate::process::spawn_std(Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output())
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
        store.insert_agent(&agent, &AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "A".into(), avatar: None, provider: "claude".into(), model: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.id), name: "general".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
        store.insert_channel(&channel).unwrap();
        store.create_session(&SessionId::new(), &channel.id).unwrap();
        let store = Arc::new(Mutex::new(store));
        let ws = GitWorkspace::new(&dir.path().join("worktrees"), Arc::clone(&store));
        Rig { ws, store, repo, channel, _dir: dir }
    }

    #[test]
    fn create_for_branch_saves_an_owned_binding() {
        let r = rig(); git(&r.repo, &["branch", "feature"]);
        let record = r.ws.create_for_branch(&r.repo, &r.channel.id, "feature").unwrap();
        assert!(record.owned); assert!(Path::new(&record.path).join("a.txt").exists());
        assert_eq!(r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap(), Some(record));
    }
    #[test]
    fn create_for_branch_fails_when_the_branch_is_checked_out_elsewhere() {
        let r = rig(); git(&r.repo, &["branch", "feature"]);
        let other = r._dir.path().join("other"); git(&r.repo, &["worktree", "add", other.to_str().unwrap(), "feature"]);
        assert_eq!(r.ws.create_for_branch(&r.repo, &r.channel.id, "feature").unwrap_err(), "This branch is checked out in another folder. Ask again after it is free.");
        assert!(r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().is_none());
        assert!(!r.ws.target_for_branch(&r.repo, &r.channel.id, "feature").unwrap().exists());
    }
    #[test]
    fn create_for_branch_fails_for_a_deleted_branch() {
        let r = rig();
        assert!(r.ws.create_for_branch(&r.repo, &r.channel.id, "deleted").is_err());
        assert!(r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().is_none());
    }
    #[test]
    fn create_for_branch_fails_when_the_target_folder_exists() {
        let r = rig(); git(&r.repo, &["branch", "feature"]);
        let target = r.ws.target_for_branch(&r.repo, &r.channel.id, "feature").unwrap();
        std::fs::create_dir_all(&target).unwrap(); std::fs::write(target.join("keep"), "keep").unwrap();
        assert!(r.ws.create_for_branch(&r.repo, &r.channel.id, "feature").is_err());
        assert_eq!(std::fs::read_to_string(target.join("keep")).unwrap(), "keep");
        assert!(r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().is_none());
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
        assert!(store.channels_for_project(r.channel.project_id.as_ref().unwrap()).unwrap().is_empty());
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
        assert_eq!(store.channels_for_project(r.channel.project_id.as_ref().unwrap()).unwrap().len(), 1, "the channel is still there");
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
        assert!(r.store.lock().unwrap().channels_for_project(r.channel.project_id.as_ref().unwrap()).unwrap().is_empty());
    }

    #[test]
    fn changing_the_workspace_mode_starts_a_new_session() {
        let r = rig();
        let store = r.store.lock().unwrap();
        let old = store.latest_session(&r.channel.id).unwrap().unwrap();
        let new = change_workspace_mode(&store, &r.channel.id, WorkspaceMode::MainCheckout).unwrap();
        assert_ne!(old, new, "the old provider session belonged to the old directory");
        assert_eq!(store.latest_session(&r.channel.id).unwrap(), Some(new));
        assert_eq!(store.channels_for_project(r.channel.project_id.as_ref().unwrap()).unwrap()[0].workspace_mode, WorkspaceMode::MainCheckout);
    }

    #[test]
    fn existing_branch_is_used_without_creating_a_discard_branch() {
        let r = rig();
        git(&r.repo, &["branch", "feature"]);
        r.store.lock().unwrap().bind_branch(&r.channel.id, "feature").unwrap();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "feature").unwrap();
        assert_eq!(git(&prepared.cwd, &["branch", "--show-current"]), "feature");
        assert!(r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().unwrap().owned);
        assert!(!git(&r.repo, &["branch", "--list", "discard/*"]).contains("discard/"));
        assert_eq!(r.ws.prepare(&r.repo, &r.channel.id, "feature").unwrap().cwd, prepared.cwd);
    }

    #[test]
    fn checked_out_branch_is_borrowed_and_archive_never_deletes_it() {
        let r = rig();
        let external = r.repo.parent().unwrap().join("external");
        git(&r.repo, &["worktree", "add", "-b", "existing", external.to_str().unwrap()]);
        r.store.lock().unwrap().bind_branch(&r.channel.id, "existing").unwrap();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "existing").unwrap();
        assert_eq!(prepared.cwd.canonicalize().unwrap(), external.canonicalize().unwrap());
        assert!(!r.store.lock().unwrap().worktree_for(&r.channel.id).unwrap().unwrap().owned);
        std::fs::write(external.join("keep.txt"), "keep me").unwrap();
        assert!(matches!(r.ws.archive(&r.channel.id, true).unwrap(), ArchiveOutcome::Archived { .. }));
        assert_eq!(std::fs::read_to_string(external.join("keep.txt")).unwrap(), "keep me");
    }

    #[test]
    fn recorded_channel_refuses_an_external_branch_change() {
        let r = rig();
        let prepared = r.ws.prepare(&r.repo, &r.channel.id, "work").unwrap();
        git(&prepared.cwd, &["checkout", "-b", "changed-outside-discard"]);
        let error = r.ws.prepare(&r.repo, &r.channel.id, "work").unwrap_err();
        assert!(error.contains("different branch"), "{error}");
    }
}
