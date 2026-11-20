//! The read-only snapshot of a channel branch against the default branch, for the merge review tab.
//!
//! Every Git call is read-only and bounded. The source is read from `refs/heads/<branch>`, not from the
//! worktree. Every Git error becomes a `reason`.

use crate::project_context::default_branch;
use crate::workspace_diff::{ChangeFile, committed_files, toplevel};
use crate::worktree_access::{git, git_bounded};
use serde::Serialize;
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommit {
    pub id: String,
    pub subject: String,
    pub author: String,
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub source: Option<String>,
    pub target: Option<String>,
    pub head: Option<String>,
    pub head_short: Option<String>,
    pub base: Option<String>,
    pub base_short: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub nothing_to_review: bool,
    pub merged: bool,
    pub dirty: usize,
    pub commits: Vec<ReviewCommit>,
    pub files: Vec<ChangeFile>,
    pub truncated: bool,
    pub reason: Option<String>,
}

/// At most this many commits are listed.
const MAX_COMMITS: usize = 200;
const LIST_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

fn empty(source: Option<&str>, target: Option<String>, reason: impl Into<String>) -> ReviewSnapshot {
    ReviewSnapshot {
        source: source.map(str::to_owned),
        target,
        head: None,
        head_short: None,
        base: None,
        base_short: None,
        ahead: 0,
        behind: 0,
        nothing_to_review: false,
        merged: false,
        dirty: 0,
        commits: Vec::new(),
        files: Vec::new(),
        truncated: false,
        reason: Some(reason.into()),
    }
}

/// Count the changed paths of the folder: tracked changes and untracked files. A rename counts once.
fn dirty_count(folder: &Path) -> Result<usize, String> {
    let (text, _) = git_bounded(folder, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], LIST_BYTES, TIMEOUT)?;
    let mut fields = text.split('\0').filter(|field| !field.is_empty());
    let mut count = 0;
    while let Some(record) = fields.next() {
        count += 1;
        if record.starts_with(['R', 'C']) || record.get(1..2).is_some_and(|y| y == "R" || y == "C") {
            fields.next();
        }
    }
    Ok(count)
}

/// Read the `behind<TAB>ahead` line of `rev-list --left-right --count`.
fn parse_counts(text: &str) -> Result<(u64, u64), String> {
    let mut parts = text.split_whitespace();
    let (Some(behind), Some(ahead), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err("Git gave an unexpected commit count.".into());
    };
    match (behind.parse(), ahead.parse()) {
        (Ok(behind), Ok(ahead)) => Ok((behind, ahead)),
        _ => Err("Git gave an unexpected commit count.".into()),
    }
}

/// The commits of `target..head`, newest first. The flag is true when the list was cut.
fn commits(top: &Path, target: &str, head: &str) -> Result<(Vec<ReviewCommit>, bool), String> {
    let limit = format!("--max-count={}", MAX_COMMITS + 1);
    let range = format!("{target}..{head}");
    let (text, cut) = git_bounded(top, &["log", "-z", "--no-color", "--no-ext-diff", "--no-textconv", &limit, "--format=%H%x1f%s%x1f%an%x1f%ct", &range, "--"], LIST_BYTES, TIMEOUT)?;
    let mut records: Vec<&str> = text.split('\0').filter(|record| !record.is_empty()).collect();
    if cut {
        records.pop();
    }
    let mut commits: Vec<ReviewCommit> = records
        .into_iter()
        .filter_map(|record| {
            let mut parts = record.splitn(4, '\x1f');
            Some(ReviewCommit { id: parts.next()?.to_owned(), subject: parts.next()?.to_owned(), author: parts.next()?.to_owned(), time: parts.next()?.trim().parse().ok()? })
        })
        .collect();
    let over = commits.len() > MAX_COMMITS;
    commits.truncate(MAX_COMMITS);
    Ok((commits, cut || over))
}

/// True when `id` is a commit on `target`. Anything that is not a hex id is not.
fn on_target(top: &Path, id: &str, target: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_hexdigit()) && git(top, &["merge-base", "--is-ancestor", id, target]).is_ok()
}

/// The review of `branch` against the default branch.
///
/// `folder` is the channel folder, a worktree or the project path. `last_decided_head` is the `head` of the
/// newest stored decision. This never fails. Every Git error becomes a `reason`.
pub fn snapshot(folder: &Path, branch: Option<&str>, last_decided_head: Option<&str>) -> ReviewSnapshot {
    let Some(source) = branch else { return empty(None, None, "This channel has no branch.") };
    let top = match toplevel(folder) {
        Ok(top) => top,
        Err(reason) => return empty(Some(source), None, reason),
    };
    if git(&top, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        return empty(Some(source), None, "This repository has no commits yet.");
    }
    let target = default_branch(&top);
    if target.as_deref() == Some(source) {
        return empty(Some(source), target, "This channel is on the default branch.");
    }
    let source_ref = format!("refs/heads/{source}");
    let Ok(head) = git(&top, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{source_ref}^{{commit}}")]) else {
        return empty(Some(source), target, "The branch no longer exists.");
    };
    let Some(target_name) = target.clone() else {
        return empty(Some(source), None, "The target branch cannot be determined.");
    };
    let target_ref = format!("refs/heads/{target_name}");
    let Ok(target_id) = git(&top, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{target_ref}^{{commit}}")]) else {
        return empty(Some(source), target, "The target branch cannot be determined.");
    };
    let Ok(base) = git(&top, &["merge-base", &target_id, &head]) else {
        return empty(Some(source), target, format!("The branch has no commit in common with {target_name}."));
    };
    let short = |id: &str| git(&top, &["rev-parse", "--short", id]).ok();
    let counted = git(&top, &["rev-list", "--left-right", "--count", &format!("{target_id}...{head}")]).and_then(|text| parse_counts(&text));
    let (behind, ahead) = match counted {
        Ok(counts) => counts,
        Err(error) => return empty(Some(source), target, format!("Git could not read the branch: {error}")),
    };
    let dirty = match dirty_count(folder) {
        Ok(dirty) => dirty,
        Err(error) => return empty(Some(source), target, format!("Git could not read the branch: {error}")),
    };
    let mut snap = empty(Some(source), target, "");
    snap.reason = None;
    snap.head_short = short(&head);
    snap.base_short = short(&base);
    snap.ahead = ahead;
    snap.behind = behind;
    snap.nothing_to_review = ahead == 0;
    snap.merged = ahead == 0 && last_decided_head.is_some_and(|id| on_target(&top, id, &target_id));
    snap.dirty = dirty;
    if ahead > 0 {
        let read = commits(&top, &target_id, &head).and_then(|(commits, commits_cut)| {
            let (files, files_cut) = committed_files(&top, &base, &head)?;
            Ok((commits, files, commits_cut || files_cut))
        });
        match read {
            Ok((commits, files, truncated)) => (snap.commits, snap.files, snap.truncated) = (commits, files, truncated),
            Err(error) => return empty(Some(source), snap.target, format!("Git could not read the branch: {error}")),
        }
    }
    snap.head = Some(head);
    snap.base = Some(base);
    snap
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn run(root: &Path, args: &[&str]) -> String {
        let mut all = vec!["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];
        all.extend_from_slice(args);
        let output = crate::process::spawn_std(
            std::process::Command::new("git").args(&all).current_dir(root).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()),
        )
        .and_then(|child| child.wait_with_output())
        .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// A repository on `main` with one commit.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("first.txt"), "first\n").unwrap();
        run(&root, &["add", "."]);
        run(&root, &["commit", "-qm", "initial"]);
        (dir, root)
    }

    fn commit_file(root: &Path, name: &str, text: &str, message: &str) {
        std::fs::write(root.join(name), text).unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-qm", message]);
    }

    fn feature(root: &Path) {
        run(root, &["checkout", "-q", "-b", "feature"]);
        commit_file(root, "a.txt", "a\n", "add a");
        commit_file(root, "b.txt", "b\n", "add b");
    }

    #[test]
    fn a_branch_with_commits_lists_them_newest_first_with_files_and_counts() {
        let (_dir, root) = repository();
        feature(&root);
        let snap = snapshot(&root, Some("feature"), None);
        assert_eq!(snap.reason, None);
        assert_eq!((snap.ahead, snap.behind), (2, 0));
        assert_eq!(snap.commits.len(), 2);
        assert_eq!(snap.commits[0].subject, "add b");
        assert_eq!(snap.commits[1].subject, "add a");
        assert_eq!(snap.commits[0].author, "Test");
        assert!(snap.commits[0].time > 0);
        let paths: Vec<&str> = snap.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b.txt"]);
        assert_eq!(snap.head, Some(run(&root, &["rev-parse", "refs/heads/feature"])));
        assert_eq!(snap.base, Some(run(&root, &["rev-parse", "main"])));
        assert_eq!(snap.source.as_deref(), Some("feature"));
        assert_eq!(snap.target.as_deref(), Some("main"));
        assert!(!snap.nothing_to_review && !snap.merged && !snap.truncated);
        assert!(snap.head_short.is_some() && snap.base_short.is_some());
    }

    #[test]
    fn a_branch_with_no_commits_ahead_has_nothing_to_review_and_is_not_merged() {
        let (_dir, root) = repository();
        run(&root, &["branch", "feature"]);
        let snap = snapshot(&root, Some("feature"), None);
        assert_eq!(snap.reason, None);
        assert_eq!(snap.ahead, 0);
        assert!(snap.nothing_to_review && !snap.merged);
        assert!(snap.commits.is_empty() && snap.files.is_empty());
    }

    #[test]
    fn a_branch_is_merged_only_when_a_stored_decision_head_is_on_the_target() {
        let (_dir, root) = repository();
        feature(&root);
        let head = run(&root, &["rev-parse", "feature"]);
        run(&root, &["checkout", "-q", "main"]);
        run(&root, &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
        let merged = snapshot(&root, Some("feature"), Some(&head));
        assert!(merged.merged && merged.nothing_to_review, "{merged:?}");
        let unknown = snapshot(&root, Some("feature"), None);
        assert!(!unknown.merged && unknown.nothing_to_review);
        let other = snapshot(&root, Some("feature"), Some(&"0".repeat(40)));
        assert!(!other.merged && other.nothing_to_review);
    }

    #[test]
    fn a_branch_behind_the_target_reports_behind_and_keeps_its_own_commits_only() {
        let (_dir, root) = repository();
        feature(&root);
        run(&root, &["checkout", "-q", "main"]);
        commit_file(&root, "m.txt", "m\n", "main work");
        let snap = snapshot(&root, Some("feature"), None);
        assert_eq!((snap.ahead, snap.behind), (2, 1));
        assert_eq!(snap.commits.len(), 2);
        let paths: Vec<&str> = snap.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b.txt"]);
    }

    #[test]
    fn the_default_branch_gives_a_reason() {
        let (_dir, root) = repository();
        let snap = snapshot(&root, Some("main"), None);
        assert_eq!(snap.reason.as_deref(), Some("This channel is on the default branch."));
        assert!(snap.commits.is_empty() && snap.files.is_empty());
    }

    #[test]
    fn no_bound_branch_gives_a_reason() {
        let (_dir, root) = repository();
        assert_eq!(snapshot(&root, None, None).reason.as_deref(), Some("This channel has no branch."));
    }

    #[test]
    fn a_deleted_source_branch_gives_a_reason() {
        let (_dir, root) = repository();
        let snap = snapshot(&root, Some("gone"), None);
        assert_eq!(snap.reason.as_deref(), Some("The branch no longer exists."));
    }

    #[test]
    fn a_repository_without_commits_gives_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q", "-b", "main"]);
        let snap = snapshot(dir.path(), Some("feature"), None);
        assert_eq!(snap.reason.as_deref(), Some("This repository has no commits yet."));
        assert!(snap.commits.is_empty() && snap.files.is_empty());
    }

    #[test]
    fn a_missing_default_branch_gives_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q", "-b", "trunk"]);
        commit_file(root, "x.txt", "x\n", "initial");
        run(root, &["branch", "feature"]);
        run(root, &["checkout", "-q", "--detach"]);
        let snap = snapshot(root, Some("feature"), None);
        assert_eq!(snap.reason.as_deref(), Some("The target branch cannot be determined."));
    }

    #[test]
    fn unrelated_histories_give_a_no_merge_base_reason() {
        let (_dir, root) = repository();
        run(&root, &["checkout", "-q", "--orphan", "other"]);
        commit_file(&root, "o.txt", "o\n", "orphan");
        let snap = snapshot(&root, Some("other"), None);
        assert_eq!(snap.reason.as_deref(), Some("The branch has no commit in common with main."));
        assert!(snap.commits.is_empty() && snap.files.is_empty());
    }

    #[test]
    fn a_branch_name_that_starts_with_a_dash_is_not_read_as_an_option() {
        let (_dir, root) = repository();
        for name in ["--help", "-x", "--all"] {
            let snap = snapshot(&root, Some(name), None);
            assert_eq!(snap.reason.as_deref(), Some("The branch no longer exists."), "{name}");
        }
    }

    #[test]
    fn dirty_counts_uncommitted_and_untracked_paths_and_is_not_part_of_the_diff() {
        let (_dir, root) = repository();
        feature(&root);
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        let snap = snapshot(&root, Some("feature"), None);
        assert_eq!(snap.dirty, 2);
        let paths: Vec<&str> = snap.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b.txt"]);
        assert_eq!(snap.files[0].diff.matches("+a").count(), 1, "{}", snap.files[0].diff);
        assert!(!snap.files[0].diff.contains("changed"));
    }

    #[test]
    fn a_large_diff_is_cut_and_marked_truncated() {
        let (_dir, root) = repository();
        run(&root, &["checkout", "-q", "-b", "feature"]);
        let big = "line of text\n".repeat(crate::workspace_diff::FILE_BYTES / 4);
        commit_file(&root, "big.txt", &big, "big");
        let snap = snapshot(&root, Some("feature"), None);
        assert!(snap.truncated);
        assert!(snap.files[0].truncated && snap.files[0].diff.len() <= crate::workspace_diff::FILE_BYTES);
    }

    #[test]
    fn a_count_that_cannot_be_read_is_an_error_not_zero() {
        assert_eq!(parse_counts("3\t2"), Ok((3, 2)));
        for text in ["", "3", "x\t2", "3\ty", "-1\t2"] {
            assert!(parse_counts(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_status_failure_is_an_error_not_a_clean_tree() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dirty_count(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn the_folder_not_being_a_repository_gives_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let snap = snapshot(dir.path(), Some("feature"), None);
        assert_eq!(snap.reason.as_deref(), Some("This folder is not a Git repository."));
        let missing = snapshot(&dir.path().join("nope"), Some("feature"), None);
        assert!(missing.reason.is_some());
    }
}
