//! Git worktrees: one per channel, so agents working in parallel do not overwrite each other.
//!
//! Rules (see PLAN.md, "Workspace isolation"):
//! - The worktree is created before the provider session starts. A running session is never moved.
//! - Creation is serialized per repository.
//! - A worktree branches from the repository's current `HEAD`. Uncommitted files are not copied.
//! - The branch is always kept. A worktree is removed only if nothing would be lost.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorktreeError {
    #[error("`{0}` is not inside a git repository")]
    NotARepo(String),
    #[error("the repository has no commits yet. Make one, then try again")]
    NoCommits,
    #[error("git failed: {0}")]
    Git(String),
    #[error("storage error: {0}")]
    Io(String),
    #[error("the worktree has work that would be lost: {}", .0.join("; "))]
    WouldLoseWork(Vec<String>),
}

pub type Result<T> = std::result::Result<T, WorktreeError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchEntry {
    pub branch: String,
    pub worktree: Option<PathBuf>,
    pub is_main_checkout: bool,
    pub detached: bool,
    pub is_discard: bool,
}

/// Combine local branch refs with their checked-out worktrees.
pub fn parse_branches(worktree_porcelain: &str, branch_refs: &str) -> Vec<BranchEntry> {
    let mut checkouts = HashMap::new();
    let mut detached_entries = Vec::new();
    for (index, block) in worktree_porcelain.split("\n\n").filter(|block| !block.trim().is_empty()).enumerate() {
        let mut path = None;
        let mut branch = None;
        let mut bare = false;
        let mut detached = false;
        let mut prunable = false;
        let mut head = None;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("worktree ") { path = Some(PathBuf::from(value)); }
            if let Some(value) = line.strip_prefix("branch refs/heads/") { branch = Some(value); }
            if line == "bare" { bare = true; }
            if line == "detached" { detached = true; }
            if line.starts_with("prunable") { prunable = true; }
            if let Some(value) = line.strip_prefix("HEAD ") { head = Some(value); }
        }
        if !bare && !prunable && let Some(path) = path {
            if detached {
                if let Some(head) = head.filter(|head| !head.is_empty()) {
                    detached_entries.push(BranchEntry { branch: head.chars().take(7).collect(), worktree: Some(path), is_main_checkout: index == 0, detached: true, is_discard: false });
                }
            } else if let Some(branch) = branch {
                checkouts.insert(branch, (path, index == 0));
            }
        }
    }
    let mut entries: Vec<_> = branch_refs.lines().filter(|name| !name.is_empty()).map(|name| {
        let checkout = checkouts.get(name);
        let is_main_checkout = checkout.is_some_and(|(_, main)| *main);
        BranchEntry { branch: name.to_string(), worktree: checkout.map(|(path, _)| path.clone()), is_main_checkout, detached: false, is_discard: name.starts_with("discard/") }
    }).collect();
    entries.extend(detached_entries);
    entries.sort_by(|a, b| {
        let rank = |e: &BranchEntry| if e.is_main_checkout { 0 } else if e.worktree.is_some() { 1 } else { 2 };
        rank(a).cmp(&rank(b)).then_with(|| a.branch.cmp(&b.branch))
    });
    entries.dedup_by(|a, b| a.branch == b.branch && a.detached == b.detached && a.worktree == b.worktree);
    entries
}

/// Read the local branches and worktrees of a repository.
pub fn list_branches(repo: &Path) -> Result<Vec<BranchEntry>> {
    git(repo, &["rev-parse", "--git-dir"]).map_err(|_| WorktreeError::NotARepo(repo.display().to_string()))?;
    let worktrees = git(repo, &["worktree", "list", "--porcelain"])?;
    let branches = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
    Ok(parse_branches(&worktrees, &branches))
}

/// A repository discovered inside a project. Linked worktrees share one entry.
#[derive(Debug, Clone)]
pub struct DiscoveredRepository {
    /// Empty for the project root, preserving existing branch channel bindings.
    pub id: String,
    pub path: PathBuf,
    pub label: String,
}

/// Discover repositories without following directory symlinks or entering generated trees.
pub fn discover_repositories(project: &Path) -> Result<Vec<DiscoveredRepository>> {
    let project = project.canonicalize().map_err(io_err)?;
    let mut pending = vec![project.clone()];
    let mut found: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = std::collections::BTreeMap::new();
    while let Some(path) = pending.pop() {
        if path.join(".git").exists() {
            if let Ok(common) = git(&path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
                let common = PathBuf::from(common);
                if let Ok(common) = common.canonicalize() {
                    found.entry(common).or_default().push(path.clone());
                }
            }
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if path == project => return Err(io_err(error)),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | "node_modules" | "target" | "vendor" | "dist" | "build" | ".next" | ".nuxt" | ".venv" | "venv" | "__pycache__")) {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    let mut repositories = Vec::new();
    for (common, mut paths) in found {
        // Prefer the project root, then the main checkout, then a stable path.
        paths.sort_by_key(|path| (path != &project, path.join(".git") != common, path.clone()));
        let path = paths.remove(0);
        let id = if path == project { String::new() } else { path.to_string_lossy().into_owned() };
        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        repositories.push(DiscoveredRepository { id, path, label });
    }
    let labels: Vec<_> = repositories.iter().map(|repo| repo.label.clone()).collect();
    for repo in &mut repositories {
        if labels.iter().filter(|label| **label == repo.label).count() > 1 {
            repo.label = repo.path.strip_prefix(&project).unwrap_or(&repo.path).to_string_lossy().into_owned();
            if repo.label.is_empty() { repo.label = ".".into(); }
        }
    }
    repositories.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(repositories)
}

/// Root repository keys retain their existing branch names. JSON cannot be a Git branch name.
pub fn repository_branch_key(repository: &str, branch: &str) -> String {
    if repository.is_empty() { branch.into() }
    else { serde_json::to_string(&(repository, branch)).expect("string tuple serializes") }
}

/// Read branches and order them by their worktree ancestry.
pub fn list_branch_tree(repo: &Path) -> Result<Vec<crate::branch_tree::TreeRow>> {
    let entries = list_branches(repo)?;
    let mut distances = HashMap::new();
    for entry in entries.iter().filter(|entry| entry.worktree.is_some()) {
        if let Ok(branches) = git(repo, &["for-each-ref", "--contains", &entry.branch, "--format=%(refname:short)", "refs/heads"]) {
            for branch in branches.lines() {
                let range = format!("{}..{}", entry.branch, branch);
                if let Ok(count) = git(repo, &["rev-list", "--count", &range]) && let Ok(count) = count.parse() {
                    distances.insert((entry.branch.clone(), branch.to_string()), count);
                }
            }
        }
    }
    Ok(crate::branch_tree::build_tree(&entries, &|worktree, branch| distances.get(&(worktree.to_string(), branch.to_string())).copied()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// The worktree's root directory.
    pub root: PathBuf,
    pub branch: String,
    /// The commit the branch started from.
    pub base_commit: String,
    /// The repository's main checkout.
    pub repo: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub worktree: Worktree,
    /// Where the agent should work: the worktree root, plus the subdirectory the project dir had inside the repo.
    pub cwd: PathBuf,
    /// True if this call created the worktree.
    pub created: bool,
    /// Things the user should know, such as uncommitted files that were not copied.
    pub notes: Vec<String>,
}

/// What a worktree holds that is not safe elsewhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkStatus {
    pub uncommitted: Vec<String>,
    pub untracked: Vec<String>,
    pub ignored_with_content: Vec<String>,
    /// Commits on the branch that no other branch has.
    pub unmerged_commits: usize,
}

impl WorkStatus {
    pub fn is_clean(&self) -> bool {
        self.uncommitted.is_empty() && self.untracked.is_empty() && self.ignored_with_content.is_empty() && self.unmerged_commits == 0
    }

    /// One line per kind of work at risk. Empty when clean.
    pub fn describe(&self) -> Vec<String> {
        let list = |items: &[String]| {
            let shown: Vec<&str> = items.iter().take(5).map(String::as_str).collect();
            let more = items.len().saturating_sub(5);
            if more > 0 { format!("{} (and {more} more)", shown.join(", ")) } else { shown.join(", ") }
        };
        let mut out = Vec::new();
        if !self.uncommitted.is_empty() {
            out.push(format!("uncommitted changes: {}", list(&self.uncommitted)));
        }
        if !self.untracked.is_empty() {
            out.push(format!("new files that are not in git: {}", list(&self.untracked)));
        }
        if !self.ignored_with_content.is_empty() {
            out.push(format!("ignored files that hold data: {}", list(&self.ignored_with_content)));
        }
        if self.unmerged_commits > 0 {
            out.push(format!("{} unmerged commit(s) that are not on any other branch (they stay on the kept branch)", self.unmerged_commits));
        }
        out
    }
}

pub struct WorktreeManager {
    base: PathBuf,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

fn git_cmd(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    // Hooks in the user's repository must not run for Discard's own operations.
    cmd.args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"]).args(args).current_dir(dir).env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// Run git and return stdout. A failure carries git's own message.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_cmd(dir, args).output().map_err(|e| WorktreeError::Git(format!("could not run git: {e}")))?;
    if !out.status.success() {
        return Err(WorktreeError::Git(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn io_err(e: std::io::Error) -> WorktreeError {
    WorktreeError::Io(e.to_string())
}

/// Lowercase letters, digits, and single dashes. At most `max` characters.
fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    trimmed.chars().take(max).collect::<String>().trim_matches('-').to_string()
}

fn short_id(channel_id: &str) -> String {
    let id: String = channel_id.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    if id.is_empty() { "0".to_string() } else { id[id.len().saturating_sub(12)..].to_owned() }
}

/// Commits on `branch` that no other local or remote branch has. Refs go through stdin so a long list is safe.
fn unmerged_commits(repo: &Path, branch: &str) -> usize {
    use std::io::Write;
    let own = format!("refs/heads/{branch}");
    let Ok(refs) = git(repo, &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"]) else { return 0 };
    let mut input = format!("{own}\n");
    for other in refs.lines().filter(|r| *r != own) {
        input.push_str(&format!("^{other}\n"));
    }
    let Ok(mut child) = git_cmd(repo, &["rev-list", "--count", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return 0;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else { return 0 };
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

impl WorktreeManager {
    /// `base` is where worktrees are created (outside every project).
    pub fn new(base: &Path) -> Self {
        Self { base: base.to_path_buf(), locks: Mutex::new(HashMap::new()) }
    }

    fn lock_for(&self, repo: &Path) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(locks.entry(repo.to_path_buf()).or_default())
    }

    /// True if `dir` is inside a git work tree.
    pub fn is_repo(dir: &Path) -> bool {
        dir.is_dir() && git(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|s| s == "true")
    }

    /// Create the worktree for a channel, or return the existing one. `channel_id` makes the names unique.
    pub fn prepare(&self, project_dir: &Path, channel_id: &str, channel_name: &str) -> Result<Prepared> {
        self.prepare_with_branch(project_dir, channel_id, channel_name, None)
    }
    pub fn prepare_branch(&self, project_dir: &Path, channel_id: &str, branch: &str) -> Result<Prepared> {
        git(project_dir, &["check-ref-format", "--branch", branch])?;
        git(project_dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")])?;
        self.prepare_with_branch(project_dir, channel_id, branch, Some(branch))
    }
    /// Validate a recorded directory instead of silently resuming a session in a different checkout.
    pub fn recorded(&self, project_dir: &Path, root: &Path, branch: &str) -> Result<Prepared> {
        let entries = list_branches(project_dir)?;
        let canonical = root.canonicalize().map_err(io_err)?;
        let entry = entries.iter().find(|entry| entry.worktree.as_ref().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&canonical))
            .ok_or_else(|| WorktreeError::Git("The channel's worktree is no longer registered.".into()))?;
        if entry.branch != branch { return Err(WorktreeError::Git("The channel's worktree is now on a different branch. Open that branch in a new channel.".into())); }
        let prefix = git(project_dir, &["rev-parse", "--show-prefix"])?;
        let cwd = if prefix.is_empty() { root.to_path_buf() } else { root.join(prefix.trim_end_matches('/')) };
        if !cwd.is_dir() { return Err(WorktreeError::Git("The project directory is missing in this worktree.".into())); }
        Ok(Prepared { cwd, worktree: Worktree { root: root.to_path_buf(), branch: branch.into(), base_commit: git(root, &["rev-parse", "HEAD"])? , repo: project_dir.into() }, created: false, notes: vec![] })
    }
    fn prepare_with_branch(&self, project_dir: &Path, channel_id: &str, channel_name: &str, existing_branch: Option<&str>) -> Result<Prepared> {
        if !Self::is_repo(project_dir) {
            return Err(WorktreeError::NotARepo(project_dir.display().to_string()));
        }
        let repo = PathBuf::from(git(project_dir, &["rev-parse", "--show-toplevel"])?);
        let repo = fs::canonicalize(&repo).unwrap_or(repo);
        let prefix = git(project_dir, &["rev-parse", "--show-prefix"])?;
        let common = PathBuf::from(git(&repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?);
        let lock = self.lock_for(&common);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(branch) = existing_branch {
            if let Some(root) = list_branches(&repo)?.into_iter().find(|entry| entry.branch == branch).and_then(|entry| entry.worktree) {
                return self.recorded(project_dir, &root, branch);
            }
        }
        let head = git(&repo, &["rev-parse", "--verify", "HEAD"]).map_err(|_| WorktreeError::NoCommits)?;
        let name = format!("{}-{}", Some(slug(channel_name, 24)).filter(|s| !s.is_empty()).unwrap_or_else(|| "channel".into()), short_id(channel_id));
        let branch = existing_branch.map(str::to_owned).unwrap_or_else(|| format!("discard/{name}"));
        let repo_key = format!("{}-{}", Some(slug(&repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), 24)).filter(|s| !s.is_empty()).unwrap_or_else(|| "repo".into()),
            Sha256::digest(repo.to_string_lossy().as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect::<String>());
        let parent = self.base.join(repo_key);
        let root = parent.join(&name);
        let base_file = parent.join(format!(".{name}.base"));
        let cwd = |root: &Path| if prefix.is_empty() { root.to_path_buf() } else { root.join(prefix.trim_end_matches('/')) };

        // A directory deleted by hand leaves a stale registration. Clear it before looking.
        git(&repo, &["worktree", "prune"])?;
        let registered = git(&repo, &["worktree", "list", "--porcelain"])?
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .any(|p| fs::canonicalize(p).ok() == fs::canonicalize(&root).ok() && root.exists());
        if registered {
            let base_commit = fs::read_to_string(&base_file).map(|s| s.trim().to_string()).unwrap_or_else(|_| head.clone());
            let worktree = Worktree { root: root.clone(), branch, base_commit, repo };
            return Ok(Prepared { cwd: cwd(&root), worktree, created: false, notes: Vec::new() });
        }
        if root.exists() {
            return Err(WorktreeError::Git(format!("{} exists but is not a registered worktree", root.display())));
        }

        fs::create_dir_all(&parent).map_err(io_err)?;
        let root_str = root.to_string_lossy().into_owned();
        let branch_exists = git(&repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
        let added = if branch_exists {
            git(&repo, &["worktree", "add", &root_str, &branch])
        } else {
            git(&repo, &["worktree", "add", "-b", &branch, &root_str, &head])
        };
        if let Err(e) = added {
            let _ = fs::remove_dir_all(&root);
            let _ = git(&repo, &["worktree", "prune"]);
            return Err(e);
        }
        let base_commit = if branch_exists { git(&repo, &["rev-parse", &branch]).unwrap_or_else(|_| head.clone()) } else { head.clone() };
        let _ = fs::write(&base_file, &base_commit);

        let mut notes = Vec::new();
        let dirty = git(&repo, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
        let files: Vec<&str> = dirty.lines().filter(|l| l.len() > 3).map(|l| &l[3..]).collect();
        if !files.is_empty() {
            let shown: Vec<&str> = files.iter().take(5).copied().collect();
            let more = if files.len() > 5 { format!(" (and {} more)", files.len() - 5) } else { String::new() };
            notes.push(format!("Uncommitted changes in the main checkout were not copied to this worktree: {}{more}. Commit them first if the agent needs them.", shown.join(", ")));
        }
        let worktree = Worktree { root: root.clone(), branch, base_commit, repo };
        Ok(Prepared { cwd: cwd(&root), worktree, created: true, notes })
    }

    pub fn status(&self, worktree: &Worktree) -> Result<WorkStatus> {
        let mut status = WorkStatus::default();
        if worktree.root.is_dir() {
            let out = git(&worktree.root, &["status", "--porcelain=v1", "--untracked-files=all", "--ignored=matching"])?;
            for line in out.lines().filter(|l| l.len() > 3) {
                let (code, path) = (&line[..2], line[3..].trim_matches('"').to_string());
                match code {
                    "??" => status.untracked.push(path),
                    "!!" => {
                        let full = worktree.root.join(path.trim_end_matches('/'));
                        let holds_data = match fs::symlink_metadata(&full) {
                            Ok(m) if m.is_dir() => fs::read_dir(&full).map(|mut d| d.next().is_some()).unwrap_or(true),
                            Ok(m) => m.len() > 0,
                            Err(_) => false,
                        };
                        if holds_data {
                            status.ignored_with_content.push(path);
                        }
                    }
                    _ => status.uncommitted.push(path),
                }
            }
        }
        status.unmerged_commits = unmerged_commits(&worktree.repo, &worktree.branch);
        Ok(status)
    }

    /// Remove the worktree directory. The branch is kept. Refuses if `status` is not clean, unless `force`.
    pub fn remove(&self, worktree: &Worktree, force: bool) -> Result<()> {
        let lock = self.lock_for(&worktree.repo);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let status = self.status(worktree)?;
        // Unmerged commits are safe on the kept branch. Only files that live in the directory block removal.
        let mut at_risk = status.clone();
        at_risk.unmerged_commits = 0;
        if !at_risk.is_clean() && !force {
            return Err(WorktreeError::WouldLoseWork(at_risk.describe()));
        }
        if worktree.root.exists() {
            git(&worktree.repo, &["worktree", "remove", "--force", &worktree.root.to_string_lossy()])?;
        }
        git(&worktree.repo, &["worktree", "prune"])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn branch_parser_handles_empty_and_single_branch() {
        assert!(parse_branches("", "").is_empty());
        let entries = parse_branches("worktree /repo\nHEAD abc\nbranch refs/heads/main\n", "main\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], BranchEntry { branch: "main".into(), worktree: Some("/repo".into()), is_main_checkout: true, detached: false, is_discard: false });
    }

    #[test]
    fn branch_parser_orders_worktrees_and_skips_detached_and_bare() {
        let porcelain = "worktree /repo\nbranch refs/heads/z-main\n\nworktree /z\nbranch refs/heads/z/work\n\nworktree /a\nbranch refs/heads/discard/a\n\nworktree /detached\ndetached\n\nworktree /bare\nbare\nbranch refs/heads/bare-branch\n";
        let entries = parse_branches(porcelain, "z/work\nno/tree\nz-main\ndiscard/a\nbare-branch\n");
        assert_eq!(entries.iter().map(|e| e.branch.as_str()).collect::<Vec<_>>(), ["z-main", "discard/a", "z/work", "bare-branch", "no/tree"]);
        assert!(entries[0].is_main_checkout && !entries[0].detached);
        assert!(entries[1].is_discard && entries[1].worktree.as_deref() == Some(Path::new("/a")));
        assert_eq!(entries[2].worktree.as_deref(), Some(Path::new("/z")));
        assert!(entries[3..].iter().all(|e| e.worktree.is_none() && !e.detached));
    }

    #[test]
    fn list_branches_reads_a_real_repository() {
        let r = rig();
        let path = r._dir.path().join("other");
        git(&r.repo, &["worktree", "add", "-q", "-b", "feature/one", path.to_str().unwrap()]);
        let entries = list_branches(&r.repo).unwrap();
        assert_eq!(entries.iter().map(|e| e.branch.as_str()).collect::<Vec<_>>(), ["main", "feature/one"]);
        assert_eq!(entries[1].worktree.as_ref().unwrap().canonicalize().unwrap(), path.canonicalize().unwrap());
        assert!(matches!(list_branches(r._dir.path()), Err(WorktreeError::NotARepo(_))));
    }

    #[test]
    fn parser_lists_a_detached_worktree_by_short_sha() {
        let entries = parse_branches("worktree /repo\nHEAD aaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree /d\nHEAD abcdef1234567890\ndetached\n", "main\n");
        assert!(entries.contains(&BranchEntry { branch: "abcdef1".into(), worktree: Some("/d".into()), is_main_checkout: false, detached: true, is_discard: false }));
    }

    #[test]
    fn parser_treats_a_prunable_worktree_as_no_worktree() {
        let entries = parse_branches("worktree /repo\nbranch refs/heads/main\n\nworktree /gone\nHEAD abc\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n", "main\ngone\n");
        assert_eq!(entries.iter().find(|e| e.branch == "gone").unwrap().worktree, None);
    }

    #[test]
    fn parser_skips_a_detached_block_with_no_head_line() {
        assert!(parse_branches("worktree /x\ndetached\n", "").is_empty());
    }

    #[test]
    fn parser_never_marks_a_bare_first_block_as_main() {
        let entries = parse_branches("worktree /bare\nbare\n\nworktree /w\nbranch refs/heads/w\n", "w\n");
        assert!(!entries[0].is_main_checkout);
    }

    #[test]
    fn list_branch_tree_nests_by_ancestry_in_a_real_repository() {
        use crate::branch_tree::RowKind;
        let r = rig();
        git(&r.repo, &["branch", "old"]);
        git(&r.repo, &["commit", "--allow-empty", "-q", "-m", "main"]);
        let path = r._dir.path().join("other");
        git(&r.repo, &["worktree", "add", "-q", "-b", "feature/one", path.to_str().unwrap()]);
        git(&path, &["commit", "--allow-empty", "-q", "-m", "one"]);
        git(&path, &["checkout", "-q", "-b", "feature/two"]);
        git(&path, &["commit", "--allow-empty", "-q", "-m", "two"]);
        git(&path, &["checkout", "-q", "feature/one"]);
        let rows = list_branch_tree(&r.repo).unwrap();
        assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.kind, r.parent.as_deref())).collect::<Vec<_>>(), [
            ("main", RowKind::Main, None), ("old", RowKind::Branch, Some("main")),
            ("feature/one", RowKind::Worktree, None), ("feature/two", RowKind::Branch, Some("feature/one")),
        ]);
    }

    #[test]
    fn list_branch_tree_is_empty_for_a_repository_with_no_commits() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        assert!(list_branch_tree(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn list_branch_tree_reports_a_non_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(list_branch_tree(dir.path()), Err(WorktreeError::NotARepo(_))));
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Rig {
        repo: PathBuf,
        mgr: WorktreeManager,
        _dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("my repo");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("sub/b.txt"), "b\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let mgr = WorktreeManager::new(&dir.path().join("worktrees"));
        Rig { repo, mgr, _dir: dir }
    }

    #[test]
    fn is_repo_recognizes_repositories_and_their_subdirectories() {
        let r = rig();
        assert!(WorktreeManager::is_repo(&r.repo));
        assert!(WorktreeManager::is_repo(&r.repo.join("sub")));
        assert!(!WorktreeManager::is_repo(r._dir.path()));
        assert!(!WorktreeManager::is_repo(Path::new("/nonexistent/dir")));
    }

    #[test]
    fn prepare_creates_a_worktree_on_a_new_branch_from_head() {
        let r = rig();
        let p = r.mgr.prepare(&r.repo, "01234567-aaaa", "general").unwrap();
        assert!(p.created);
        assert!(p.worktree.root.join("a.txt").exists(), "committed files are present");
        assert_eq!(std::fs::read_to_string(p.worktree.root.join("a.txt")).unwrap(), "one\n");
        assert!(p.worktree.branch.starts_with("discard/general-01234567"), "{}", p.worktree.branch);
        assert_eq!(git(&p.worktree.root, &["rev-parse", "--abbrev-ref", "HEAD"]), p.worktree.branch);
        assert_eq!(git(&p.worktree.root, &["rev-parse", "HEAD"]), p.worktree.base_commit);
        assert_eq!(p.cwd, p.worktree.root);
        assert!(!p.worktree.root.starts_with(&r.repo), "worktrees live outside the project");
    }

    #[test]
    fn a_second_call_returns_the_same_worktree_without_creating_another() {
        let r = rig();
        let first = r.mgr.prepare(&r.repo, "id-0001", "general").unwrap();
        std::fs::write(first.worktree.root.join("work.txt"), "wip").unwrap();
        let second = r.mgr.prepare(&r.repo, "id-0001", "general").unwrap();
        assert!(!second.created);
        assert_eq!(second.worktree, first.worktree);
        assert!(second.worktree.root.join("work.txt").exists(), "existing work is untouched");
    }

    #[test]
    fn a_project_subdirectory_maps_to_the_same_subdirectory_in_the_worktree() {
        let r = rig();
        let p = r.mgr.prepare(&r.repo.join("sub"), "id-0002", "general").unwrap();
        assert_eq!(p.cwd, p.worktree.root.join("sub"));
        assert!(p.cwd.join("b.txt").exists());
    }

    #[test]
    fn uncommitted_changes_in_the_main_checkout_are_not_copied_and_the_user_is_told() {
        let r = rig();
        std::fs::write(r.repo.join("a.txt"), "edited\n").unwrap();
        std::fs::write(r.repo.join("new.txt"), "new\n").unwrap();
        let p = r.mgr.prepare(&r.repo, "id-0003", "general").unwrap();
        assert_eq!(std::fs::read_to_string(p.worktree.root.join("a.txt")).unwrap(), "one\n");
        assert!(!p.worktree.root.join("new.txt").exists());
        assert!(p.notes.iter().any(|n| n.contains("not copied") && n.contains("a.txt")), "{:?}", p.notes);
        assert_eq!(std::fs::read_to_string(r.repo.join("a.txt")).unwrap(), "edited\n", "the main checkout is left alone");
    }

    #[test]
    fn a_clean_main_checkout_gives_no_notes() {
        let r = rig();
        assert!(r.mgr.prepare(&r.repo, "id-0004", "general").unwrap().notes.is_empty());
    }

    #[test]
    fn each_channel_gets_its_own_worktree_and_branch() {
        let r = rig();
        let a = r.mgr.prepare(&r.repo, "aaaaaaaa-1", "general").unwrap();
        let b = r.mgr.prepare(&r.repo, "bbbbbbbb-2", "general").unwrap();
        assert_ne!(a.worktree.root, b.worktree.root);
        assert_ne!(a.worktree.branch, b.worktree.branch, "the same channel name must not collide");
        std::fs::write(a.worktree.root.join("only-a.txt"), "x").unwrap();
        assert!(!b.worktree.root.join("only-a.txt").exists());
    }

    #[test]
    fn channel_names_with_odd_characters_make_safe_branch_names() {
        let r = rig();
        let p = r.mgr.prepare(&r.repo, "id-0005", "My Channel! ../../etc/passwd ~^:").unwrap();
        assert!(p.worktree.branch.starts_with("discard/"));
        assert!(p.worktree.branch.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '/')), "{}", p.worktree.branch);
        assert!(p.worktree.root.starts_with(r._dir.path().join("worktrees")), "no path escape");
        let empty = r.mgr.prepare(&r.repo, "id-0006", "   ").unwrap();
        assert!(empty.worktree.branch.starts_with("discard/channel-"), "{}", empty.worktree.branch);
    }

    #[test]
    fn creation_is_serialized_so_parallel_channels_all_succeed() {
        let r = rig();
        let mgr = Arc::new(r.mgr);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (mgr, repo) = (Arc::clone(&mgr), r.repo.clone());
                std::thread::spawn(move || mgr.prepare(&repo, &format!("par-{i:04}"), "c").unwrap())
            })
            .collect();
        let mut roots: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().worktree.root).collect();
        roots.sort();
        roots.dedup();
        assert_eq!(roots.len(), 8);
        assert_eq!(git(&r.repo, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 9, "main plus eight");
    }

    #[test]
    fn a_non_repository_and_an_empty_repository_are_clear_errors() {
        let r = rig();
        let plain = r._dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(matches!(r.mgr.prepare(&plain, "x", "c"), Err(WorktreeError::NotARepo(_))));
        let empty = r._dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        git(&empty, &["init", "-q"]);
        assert_eq!(r.mgr.prepare(&empty, "x", "c"), Err(WorktreeError::NoCommits));
        assert!(!r._dir.path().join("worktrees").exists() || std::fs::read_dir(r._dir.path().join("worktrees")).map(|d| d.count()).unwrap_or(0) == 0, "a failed attempt leaves nothing behind");
    }

    #[test]
    fn user_git_hooks_do_not_run_when_creating_a_worktree() {
        let r = rig();
        let hooks = r._dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let marker = r._dir.path().join("hook-ran");
        let hook = hooks.join("post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        git(&r.repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        r.mgr.prepare(&r.repo, "id-0007", "general").unwrap();
        assert!(!marker.exists(), "a post-checkout hook must not run");
    }

    #[test]
    fn a_fresh_worktree_is_clean() {
        let r = rig();
        let p = r.mgr.prepare(&r.repo, "id-0008", "general").unwrap();
        let s = r.mgr.status(&p.worktree).unwrap();
        assert!(s.is_clean(), "{s:?}");
        assert!(s.describe().is_empty());
    }

    #[test]
    fn status_reports_edits_new_files_ignored_data_and_unmerged_commits() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0009", "general").unwrap().worktree;
        std::fs::write(w.root.join("a.txt"), "changed\n").unwrap();
        std::fs::write(w.root.join("fresh.txt"), "new\n").unwrap();
        std::fs::write(w.root.join("debug.log"), "important log\n").unwrap();
        let s = r.mgr.status(&w).unwrap();
        assert_eq!(s.uncommitted, ["a.txt"]);
        assert_eq!(s.untracked, ["fresh.txt"]);
        assert_eq!(s.ignored_with_content, ["debug.log"]);
        assert_eq!(s.unmerged_commits, 0);
        git(&w.root, &["add", "-A"]);
        git(&w.root, &["commit", "-q", "-m", "work"]);
        let s = r.mgr.status(&w).unwrap();
        assert_eq!(s.unmerged_commits, 1, "a commit no other branch has is at risk");
        assert!(s.uncommitted.is_empty() && s.untracked.is_empty());
        let lines = s.describe();
        assert!(lines.iter().any(|l| l.contains("unmerged") || l.contains("not on any other branch")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("debug.log")), "{lines:?}");
    }

    #[test]
    fn a_commit_that_was_merged_elsewhere_is_no_longer_at_risk() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0010", "general").unwrap().worktree;
        std::fs::write(w.root.join("f.txt"), "x\n").unwrap();
        git(&w.root, &["add", "-A"]);
        git(&w.root, &["commit", "-q", "-m", "work"]);
        git(&r.repo, &["merge", "-q", "--ff-only", &w.branch]);
        assert_eq!(r.mgr.status(&w).unwrap().unmerged_commits, 0);
    }

    #[test]
    fn remove_deletes_a_clean_worktree_and_keeps_the_branch() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0011", "general").unwrap().worktree;
        r.mgr.remove(&w, false).unwrap();
        assert!(!w.root.exists());
        assert!(git(&r.repo, &["branch", "--list", &w.branch]).contains(&w.branch), "the branch must be kept");
        assert!(!git(&r.repo, &["worktree", "list"]).contains(w.root.to_str().unwrap()));
        // It can be created again for the same channel, reusing the kept branch.
        let again = r.mgr.prepare(&r.repo, "id-0011", "general").unwrap();
        assert!(again.created);
        assert_eq!(again.worktree.branch, w.branch);
    }

    #[test]
    fn remove_refuses_to_lose_work_and_says_what_would_be_lost() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0012", "general").unwrap().worktree;
        std::fs::write(w.root.join("precious.txt"), "do not lose\n").unwrap();
        let err = r.mgr.remove(&w, false).unwrap_err();
        assert!(matches!(&err, WorktreeError::WouldLoseWork(items) if items.iter().any(|i| i.contains("precious.txt"))), "{err:?}");
        assert!(w.root.join("precious.txt").exists(), "nothing was deleted");
    }

    #[test]
    fn remove_with_force_deletes_the_directory_but_still_keeps_committed_work_on_the_branch() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0013", "general").unwrap().worktree;
        std::fs::write(w.root.join("c.txt"), "committed\n").unwrap();
        git(&w.root, &["add", "-A"]);
        git(&w.root, &["commit", "-q", "-m", "keep me"]);
        std::fs::write(w.root.join("loose.txt"), "loose\n").unwrap();
        r.mgr.remove(&w, true).unwrap();
        assert!(!w.root.exists());
        assert!(git(&r.repo, &["log", "--oneline", &w.branch]).contains("keep me"), "the commit survives on the branch");
    }

    #[test]
    fn removing_a_worktree_that_is_already_gone_is_not_an_error() {
        let r = rig();
        let w = r.mgr.prepare(&r.repo, "id-0014", "general").unwrap().worktree;
        std::fs::remove_dir_all(&w.root).unwrap();
        r.mgr.remove(&w, false).unwrap();
    }

    #[test]
    fn a_worktree_directory_deleted_by_hand_is_recreated_on_the_next_prepare() {
        let r = rig();
        let first = r.mgr.prepare(&r.repo, "id-0015", "general").unwrap();
        std::fs::remove_dir_all(&first.worktree.root).unwrap();
        let again = r.mgr.prepare(&r.repo, "id-0015", "general").unwrap();
        assert!(again.created && again.worktree.root.join("a.txt").exists());
    }
}
