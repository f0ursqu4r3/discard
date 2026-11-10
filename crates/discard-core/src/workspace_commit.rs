//! Scoped commits through the app, outside the provider sandbox.

use std::{path::{Path, PathBuf}, time::Duration};
use crate::{workspace_session::paths_inside, worktree_access::{git, git_bounded}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitRequest { pub message: String, pub paths: Option<Vec<String>> }
#[derive(Debug, Clone, Serialize)]
pub struct CommitResult { pub commit: String, pub branch: String, pub files: Vec<String>, #[serde(skip_serializing_if = "Option::is_none")] pub warning: Option<String> }

pub fn commit(root: &Path, request: &CommitRequest, main_checkout_granted: bool) -> Result<CommitResult, String> {
    commit_with_file_limit(root, request, 1024 * 1024, main_checkout_granted)
}
fn commit_with_file_limit(root: &Path, request: &CommitRequest, file_limit: usize, main_checkout_granted: bool) -> Result<CommitResult, String> {
    if request.message.trim().is_empty() || request.message.len() > 4096 || request.message.contains('\0') { return Err("Enter a commit message of 1 to 4096 bytes.".into()); }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if PathBuf::from(git(&root, &["rev-parse", "--show-toplevel"])?).canonicalize().map_err(|e| e.to_string())? != root { return Err("The commit root is not a worktree root.".into()); }
    let git_dir = PathBuf::from(git(&root, &["rev-parse", "--absolute-git-dir"])?).canonicalize().map_err(|e| e.to_string())?;
    let common = PathBuf::from(git(&root, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?).canonicalize().map_err(|e| e.to_string())?;
    if (git_dir == common || common.starts_with(&root)) && !main_checkout_granted { return Err("Agents do not write in the main checkout.".into()); }
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" { return Err("Create a branch before committing. This worktree has a detached HEAD.".into()); }
    let all = vec![".".to_owned()]; let paths = request.paths.as_ref().unwrap_or(&all);
    if !paths_inside(&root, paths) || paths.iter().any(|p| p.starts_with(':') || p.contains('\0') || Path::new(p).components().any(|c| c.as_os_str() == ".git")) { return Err("Commit paths must stay inside this worktree.".into()); }
    let run = |args: &[&str]| git_bounded(&root, args, 1024 * 1024, Duration::from_secs(30));
    let mut add = vec!["add", "-A", "--"]; add.extend(paths.iter().map(String::as_str)); run(&add)?;
    let mut args = vec!["commit", "-m", request.message.as_str()];
    if request.paths.is_some() { args.extend(["--only", "--"]); args.extend(paths.iter().map(String::as_str)); }
    run(&args)?;
    let commit = git(&root, &["rev-parse", "--verify", "HEAD"])?;
    // File inspection is optional after a successful mutation. Never lose its audit identity.
    let (files, warning) = match git_bounded(&root, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-z", "HEAD"], file_limit, Duration::from_secs(30)) {
        Ok((files, truncated)) => {
            let complete = if truncated { files.rsplit_once('\0').map(|(prefix, _)| prefix).unwrap_or("") } else { &files };
            (complete.split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect(), truncated.then(|| "The commit succeeded. Its file list was truncated.".into()))
        }
        Err(error) => (Vec::new(), Some(format!("The commit succeeded, but Discard could not list its files: {error}"))),
    };
    Ok(CommitResult { commit, branch, files, warning })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::worktree_access::git;
    use std::path::PathBuf;
    pub(crate) fn repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap(); let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]).unwrap();
        for (key, value) in [("user.name", "Workspace User"), ("user.email", "workspace@example.invalid"), ("commit.gpgsign", "false")] { git(&repo, &["config", key, value]).unwrap(); }
        git(&repo, &["commit", "--allow-empty", "-qm", "initial"]).unwrap();
        let root = dir.path().join("worktree"); git(&repo, &["worktree", "add", "-b", "feature", root.to_str().unwrap()]).unwrap();
        (dir, repo, root)
    }
    fn request() -> CommitRequest { CommitRequest { message: "Change files".into(), paths: None } }
    #[test]
    fn main_checkout_commit_needs_a_grant() {
        let (_dir, repo, _root) = repository(); std::fs::write(repo.join("new"), "new").unwrap();
        assert!(commit(&repo, &request(), false).is_err());
        assert_eq!(commit(&repo, &request(), true).unwrap().branch, "main");
    }
    #[test]
    fn main_checkout_commit_still_runs_no_hooks() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, repo, _root) = repository();
        let hook = repo.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(repo.join("new"), "new").unwrap();
        assert!(commit(&repo, &request(), true).is_ok()); assert!(!repo.join("hook-ran").exists());
    }

    #[test]
    fn successful_commit_survives_file_list_truncation() {
        let (_dir, _repo, root) = repository(); std::fs::write(root.join("long-filename"), "new").unwrap();
        let result = commit_with_file_limit(&root, &request(), 2, false).unwrap();
        assert_eq!(result.commit, git(&root, &["rev-parse", "HEAD"]).unwrap());
        assert!(result.warning.as_deref().unwrap().contains("file list"));
        assert!(result.files.is_empty());
    }
    #[test]
    fn commits_all_changes_with_the_users_identity() {
        let (_dir, _repo, root) = repository(); std::fs::write(root.join("a"), "new").unwrap();
        let result = commit(&root, &request(), false).unwrap();
        assert_eq!(result.branch, "feature"); assert_eq!(result.files, ["a"]);
        assert_eq!(git(&root, &["log", "-1", "--format=%an <%ae>%n%B"]).unwrap(), "Workspace User <workspace@example.invalid>\nChange files");
    }
    #[test]
    fn commits_only_listed_paths() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("a"), "a").unwrap(); std::fs::write(root.join("b"), "b").unwrap(); git(&root, &["add", "b"]).unwrap();
        let result = commit(&root, &CommitRequest { paths: Some(vec!["a".into()]), ..request() }, false).unwrap();
        assert_eq!(result.files, ["a"]); assert_eq!(git(&root, &["diff", "--cached", "--name-only"]).unwrap(), "b");
    }
    #[test]
    fn refuses_detached_head() {
        let (_dir, _repo, root) = repository(); git(&root, &["checkout", "--detach"]).unwrap();
        assert_eq!(commit(&root, &request(), false).unwrap_err(), "Create a branch before committing. This worktree has a detached HEAD.");
    }
    #[test]
    fn refuses_empty_and_oversized_messages() {
        let (_dir, _repo, root) = repository();
        for message in [" ".into(), "x".repeat(4097)] { assert!(commit(&root, &CommitRequest { message, paths: None }, false).is_err()); }
    }
    #[test]
    fn refuses_paths_outside_the_root() {
        let (_dir, repo, root) = repository();
        assert!(commit(&repo, &request(), false).is_err());
        for path in ["../outside", "/outside", ":(top)*"] { assert!(commit(&root, &CommitRequest { paths: Some(vec![path.into()]), ..request() }, false).is_err()); }
    }
    #[test]
    fn nothing_to_commit_is_an_error() {
        let (_dir, _repo, root) = repository(); assert!(commit(&root, &request(), false).is_err());
    }
    #[test]
    fn does_not_run_hooks() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, _repo, root) = repository(); std::fs::create_dir(root.join(".husky")).unwrap();
        let hook = root.join(".husky/pre-commit"); std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(&root, &["config", "core.hooksPath", ".husky"]).unwrap(); commit(&root, &request(), false).unwrap();
        assert!(!root.join("hook-ran").exists());
    }
}
