//! Git actions behind the branch and worktree menus of the channel list.
//!
//! Every command runs with an argument list and no shell. Names and paths are separate arguments. A refusal is a
//! plain message for the user, so the caller can show it as it is.

use crate::worktree::{BranchEntry, git};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

fn message(error: crate::worktree::WorktreeError) -> String {
    match error {
        crate::worktree::WorktreeError::Git(message) => message,
        other => other.to_string(),
    }
}

/// The names of the local branches of a repository.
fn local_branches(repo: &Path) -> Result<Vec<String>> {
    let out = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).map_err(message)?;
    Ok(out.lines().filter(|name| !name.is_empty()).map(str::to_owned).collect())
}

/// Check a new branch name and return it trimmed. The name must be non-empty, must pass
/// `git check-ref-format --branch`, must differ from `current`, and must not belong to another local branch.
pub fn validate_branch_name(repo: &Path, name: &str, current: Option<&str>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a branch name.".into());
    }
    if current == Some(name) {
        return Err("Enter a name that is not the current name.".into());
    }
    // Git expands `@{-1}` for `--branch`, so a name with `@{` would pass the check as another name.
    if name.starts_with('-') || name.contains("@{") || name.contains('\0') || name.contains('\n') {
        return Err(format!("`{name}` is not a valid branch name."));
    }
    match git(repo, &["check-ref-format", "--branch", name]) {
        Ok(checked) if checked == name => {}
        _ => return Err(format!("`{name}` is not a valid branch name.")),
    }
    if local_branches(repo)?.iter().any(|existing| existing == name) {
        return Err(format!("A branch named `{name}` already exists."));
    }
    Ok(name.to_owned())
}

/// The default branch: the branch of the main checkout. `None` when the main checkout has a detached HEAD.
pub fn default_branch(entries: &[BranchEntry]) -> Option<&str> {
    entries.iter().find(|entry| entry.is_main_checkout && !entry.detached).map(|entry| entry.branch.as_str())
}

/// The entry of the working tree at `root`, matched by canonical path.
pub fn entry_for_worktree<'a>(entries: &'a [BranchEntry], root: &Path) -> Option<&'a BranchEntry> {
    let root = root.canonicalize().ok()?;
    entries.iter().find(|entry| entry.worktree.as_ref().and_then(|path| path.canonicalize().ok()).as_ref() == Some(&root))
}

/// Rename a local branch with `git branch -m`. It works for a branch that is checked out.
pub fn rename_branch(repo: &Path, old: &str, new: &str) -> Result<()> {
    git(repo, &["branch", "-m", "--", old, new]).map(|_| ()).map_err(message)
}

/// Count the commits of `branch` that the default branch does not reach.
pub fn unmerged_commits(repo: &Path, default: &str, branch: &str) -> Result<usize> {
    let range = format!("refs/heads/{default}..refs/heads/{branch}");
    let out = git(repo, &["rev-list", "--count", &range, "--"]).map_err(message)?;
    out.trim().parse().map_err(|_| format!("Git returned an unexpected commit count: {out}"))
}

/// The result of a branch deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchDeletion {
    Deleted,
    /// The branch has unmerged commits and the request did not force the delete. Nothing changed.
    NeedsForce { unmerged: usize },
}

/// The facts the delete dialog and the delete itself share. The checks run again for each call, so a stale
/// dialog cannot delete a branch that changed.
pub fn deletion_plan(repo: &Path, branch: &str) -> Result<usize> {
    let entries = crate::worktree::list_branches(repo).map_err(message)?;
    let entry = entries.iter().find(|entry| entry.branch == branch && !entry.detached).ok_or("This branch no longer exists.")?;
    let default = default_branch(&entries).ok_or("The default branch is unknown, so Discard cannot compare commits.")?;
    if default == branch {
        return Err("The default branch cannot be deleted.".into());
    }
    if entry.worktree.is_some() {
        return Err("This branch is checked out in a working tree. Switch it first.".into());
    }
    unmerged_commits(repo, default, branch)
}

/// Delete a local branch. With no unmerged commits it runs `git branch -d`. With unmerged commits it refuses
/// unless `force` is set, and then runs `git branch -D`.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> Result<BranchDeletion> {
    let unmerged = deletion_plan(repo, branch)?;
    if unmerged > 0 && !force {
        return Ok(BranchDeletion::NeedsForce { unmerged });
    }
    let flag = if unmerged > 0 { "-D" } else { "-d" };
    git(repo, &["branch", flag, "--", branch]).map_err(message)?;
    Ok(BranchDeletion::Deleted)
}

/// Start a new branch at the HEAD of the working tree and switch to it. Uncommitted changes stay in the tree.
/// The caller validates the name first.
pub fn create_branch(worktree: &Path, name: &str) -> Result<()> {
    git(worktree, &["switch", "-c", name]).map(|_| ()).map_err(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{Worktree, WorktreeManager, RemoveOutcome, list_branches};
    use std::path::PathBuf;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = crate::process::spawn_std(Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
            .args(args).current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()))
            .and_then(|child| child.wait_with_output()).unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A temporary repository with one commit on `main`. Nothing here touches a path outside the temp directory.
    struct Rig { repo: PathBuf, dir: tempfile::TempDir }
    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-q", "-m", "init"]);
        Rig { repo, dir }
    }
    fn branches(repo: &Path) -> Vec<String> { local_branches(repo).unwrap() }

    #[test]
    fn a_branch_name_is_checked_for_each_refusal() {
        let r = rig();
        run(&r.repo, &["branch", "taken"]);
        assert_eq!(validate_branch_name(&r.repo, "  fresh/name  ", Some("old")).unwrap(), "fresh/name");
        assert_eq!(validate_branch_name(&r.repo, "   ", None).unwrap_err(), "Enter a branch name.");
        assert!(validate_branch_name(&r.repo, "old", Some("old")).unwrap_err().contains("not the current name"));
        assert!(validate_branch_name(&r.repo, "taken", Some("old")).unwrap_err().contains("already exists"));
        assert!(validate_branch_name(&r.repo, "main", None).unwrap_err().contains("already exists"), "the current branch exists too");
        for bad in ["has space", "a..b", "-dash", "bad~name", "end/", "x.lock", "@{-1}", "a\\b", "q?", "HEAD"] {
            assert!(validate_branch_name(&r.repo, bad, None).unwrap_err().contains("not a valid branch name"), "{bad}");
        }
    }

    #[test]
    fn a_checked_out_branch_renames_and_the_old_name_is_gone() {
        let r = rig();
        rename_branch(&r.repo, "main", "trunk").unwrap();
        assert_eq!(run(&r.repo, &["rev-parse", "--abbrev-ref", "HEAD"]), "trunk");
        assert_eq!(branches(&r.repo), ["trunk"]);
        assert!(rename_branch(&r.repo, "missing", "other").is_err());
        run(&r.repo, &["branch", "taken"]);
        assert!(rename_branch(&r.repo, "trunk", "taken").is_err(), "git refuses to overwrite a branch");
    }

    #[test]
    fn a_name_with_a_leading_dash_is_not_taken_as_an_option() {
        let r = rig();
        assert!(rename_branch(&r.repo, "main", "--delete").is_err());
        assert_eq!(branches(&r.repo), ["main"]);
    }

    #[test]
    fn the_default_branch_is_the_main_checkout_branch() {
        let r = rig();
        run(&r.repo, &["branch", "side"]);
        let entries = list_branches(&r.repo).unwrap();
        assert_eq!(default_branch(&entries), Some("main"));
        run(&r.repo, &["checkout", "-q", "--detach"]);
        assert_eq!(default_branch(&list_branches(&r.repo).unwrap()), None);
    }

    #[test]
    fn deleting_a_merged_branch_uses_a_safe_delete() {
        let r = rig();
        run(&r.repo, &["branch", "merged"]);
        assert_eq!(unmerged_commits(&r.repo, "main", "merged").unwrap(), 0);
        assert_eq!(delete_branch(&r.repo, "merged", false).unwrap(), BranchDeletion::Deleted);
        assert_eq!(branches(&r.repo), ["main"]);
        // A force request with no unmerged commits still takes the safe path.
        run(&r.repo, &["branch", "again"]);
        assert_eq!(delete_branch(&r.repo, "again", true).unwrap(), BranchDeletion::Deleted);
    }

    #[test]
    fn unmerged_commits_are_counted_and_need_a_force_choice() {
        let r = rig();
        run(&r.repo, &["checkout", "-q", "-b", "wip"]);
        run(&r.repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
        run(&r.repo, &["commit", "-q", "--allow-empty", "-m", "two"]);
        run(&r.repo, &["checkout", "-q", "main"]);
        assert_eq!(unmerged_commits(&r.repo, "main", "wip").unwrap(), 2);
        assert_eq!(delete_branch(&r.repo, "wip", false).unwrap(), BranchDeletion::NeedsForce { unmerged: 2 });
        assert_eq!(branches(&r.repo), ["main", "wip"], "a refused delete changes nothing");
        assert_eq!(delete_branch(&r.repo, "wip", true).unwrap(), BranchDeletion::Deleted);
        assert_eq!(branches(&r.repo), ["main"]);
    }

    #[test]
    fn the_default_branch_and_a_branch_that_a_working_tree_holds_cannot_be_deleted() {
        let r = rig();
        let tree = r.dir.path().join("tree");
        run(&r.repo, &["worktree", "add", "-q", "-b", "held", tree.to_str().unwrap()]);
        assert!(delete_branch(&r.repo, "held", true).unwrap_err().contains("checked out"));
        assert!(delete_branch(&r.repo, "main", true).unwrap_err().contains("default branch cannot"));
        assert!(delete_branch(&r.repo, "missing", false).unwrap_err().contains("no longer exists"));
        run(&r.repo, &["checkout", "-q", "--detach"]);
        assert!(delete_branch(&r.repo, "held", true).unwrap_err().contains("default branch is unknown"));
        assert_eq!(branches(&r.repo), ["held", "main"]);
    }

    #[test]
    fn a_new_branch_starts_at_head_and_keeps_uncommitted_changes() {
        let r = rig();
        std::fs::write(r.repo.join("a.txt"), "edited\n").unwrap();
        std::fs::write(r.repo.join("new.txt"), "new\n").unwrap();
        let head = run(&r.repo, &["rev-parse", "HEAD"]);
        create_branch(&r.repo, "feature/x").unwrap();
        assert_eq!(run(&r.repo, &["rev-parse", "--abbrev-ref", "HEAD"]), "feature/x");
        assert_eq!(run(&r.repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(std::fs::read_to_string(r.repo.join("a.txt")).unwrap(), "edited\n");
        assert!(r.repo.join("new.txt").exists());
        assert!(create_branch(&r.repo, "feature/x").is_err(), "git refuses an existing name");
    }

    #[test]
    fn a_new_branch_works_in_a_linked_worktree_and_in_a_detached_one() {
        let r = rig();
        let tree = r.dir.path().join("tree");
        run(&r.repo, &["worktree", "add", "-q", "-b", "side", tree.to_str().unwrap()]);
        create_branch(&tree, "from-tree").unwrap();
        assert_eq!(run(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]), "from-tree");
        let detached = r.dir.path().join("detached");
        run(&r.repo, &["worktree", "add", "-q", "--detach", detached.to_str().unwrap()]);
        create_branch(&detached, "from-detached").unwrap();
        assert_eq!(run(&detached, &["rev-parse", "--abbrev-ref", "HEAD"]), "from-detached");
        let entries = list_branches(&r.repo).unwrap();
        assert_eq!(entry_for_worktree(&entries, &tree).unwrap().branch, "from-tree");
        assert!(entry_for_worktree(&entries, &r.dir.path().join("none")).is_none());
    }

    fn linked(r: &Rig) -> (WorktreeManager, Worktree) {
        let root = r.dir.path().join("linked");
        run(&r.repo, &["worktree", "add", "-q", "-b", "side", root.to_str().unwrap()]);
        let base = run(&root, &["rev-parse", "HEAD"]);
        (WorktreeManager::new(&r.dir.path().join("managed")), Worktree { root, branch: "side".into(), base_commit: base, repo: r.repo.clone() })
    }

    #[test]
    fn a_clean_worktree_is_removed_without_force_and_the_branch_stays() {
        let r = rig();
        let (manager, tree) = linked(&r);
        assert_eq!(manager.remove_checked(&tree, false).unwrap(), RemoveOutcome::Removed);
        assert!(!tree.root.exists());
        assert!(branches(&r.repo).contains(&"side".to_string()), "the branch stays");
    }

    #[test]
    fn a_dirty_worktree_needs_force_and_lists_its_files() {
        let r = rig();
        let (manager, tree) = linked(&r);
        std::fs::write(tree.root.join("a.txt"), "edited\n").unwrap();
        std::fs::write(tree.root.join("fresh.txt"), "new\n").unwrap();
        let RemoveOutcome::NeedsForce(files) = manager.remove_checked(&tree, false).unwrap() else { panic!("expected a refusal") };
        assert_eq!(files, ["a.txt", "fresh.txt"]);
        assert!(tree.root.exists(), "a refusal changes nothing");
        assert_eq!(manager.remove_checked(&tree, true).unwrap(), RemoveOutcome::Removed);
        assert!(!tree.root.exists());
        assert!(branches(&r.repo).contains(&"side".to_string()));
    }

    #[test]
    fn unmerged_commits_of_the_worktree_branch_do_not_block_the_removal() {
        let r = rig();
        let (manager, tree) = linked(&r);
        run(&tree.root, &["commit", "-q", "--allow-empty", "-m", "kept"]);
        assert_eq!(manager.remove_checked(&tree, false).unwrap(), RemoveOutcome::Removed);
        assert_eq!(unmerged_commits(&r.repo, "main", "side").unwrap(), 1, "the commit stays on the branch");
    }
}
