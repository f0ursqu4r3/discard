//! The live changes of a channel folder, for the Changes tab.
//!
//! Two scopes. `uncommitted` compares the working tree with `HEAD`. `branch` compares the working tree with the
//! commit where the branch left the default branch. Both include untracked files. Every Git call is read-only,
//! runs through the bounded helper, and passes `--no-ext-diff` and `--no-textconv`.

use crate::worktree_access::{git, git_bounded};
use serde::Serialize;
use std::{collections::HashMap, path::{Path, PathBuf}, time::Duration};

/// At most this many files. A larger change sets `truncated`.
pub const MAX_FILES: usize = 200;
/// At most this many bytes of diff text for one file.
pub const FILE_BYTES: usize = 64 * 1024;
/// At most this many bytes of diff text for all files together.
pub const TOTAL_BYTES: usize = 1024 * 1024;
/// At most this many bytes for one list of paths.
const LIST_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Uncommitted,
    Branch,
}

impl Scope {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "uncommitted" => Ok(Self::Uncommitted),
            "branch" => Ok(Self::Branch),
            other => Err(format!("`{other}` is not a changes scope.")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Uncommitted => "uncommitted",
            Self::Branch => "branch",
        }
    }
}

/// One changed file. `status` is one letter: `A`, `M`, `D`, `R`, or `?` for an untracked file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub binary: bool,
    pub diff: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelChanges {
    pub root: String,
    pub scope: String,
    /// The short id of the commit that the working tree is compared with. Null when the scope is unavailable.
    pub base: Option<String>,
    /// Why the scope has no result. Null when the scope is available.
    pub reason: Option<String>,
    pub files: Vec<ChangeFile>,
    pub truncated: bool,
}

struct Entry {
    path: String,
    old_path: Option<String>,
    status: String,
    additions: u64,
    deletions: u64,
    binary: bool,
}

fn bounded(root: &Path, args: &[&str], limit: usize) -> Result<(String, bool), String> {
    git_bounded(root, args, limit, TIMEOUT)
}

/// The top folder of the repository that holds `folder`.
pub(crate) fn toplevel(folder: &Path) -> Result<PathBuf, String> {
    if !folder.is_dir() {
        return Err(format!("The folder {} does not exist.", folder.display()));
    }
    let top = git(folder, &["rev-parse", "--show-toplevel"]).map_err(|_| "This folder is not a Git repository.".to_string())?;
    Ok(PathBuf::from(top))
}

/// The full id of the base commit, or the reason that the branch scope has none.
fn branch_base(top: &Path) -> Result<String, String> {
    let Some(default) = crate::project_context::default_branch(top) else {
        return Err("The default branch is unknown, so there is nothing to compare with.".into());
    };
    let Ok(current) = git(top, &["symbolic-ref", "--quiet", "--short", "HEAD"]) else {
        return Err("HEAD is detached. Check out a branch to see its changes.".into());
    };
    if current == default {
        return Err(format!("This is the default branch, {default}. Use Uncommitted to see its changes."));
    }
    git(top, &["merge-base", &format!("refs/heads/{default}"), "HEAD"])
        .map_err(|_| format!("This branch has no commit in common with {default}."))
}

/// Read the paths of a `--name-status -z` list. An incomplete last record is dropped.
fn parse_status(text: &str, cut: bool) -> Vec<Entry> {
    let mut fields = text.split('\0');
    let mut entries = Vec::new();
    while let Some(code) = fields.next() {
        let Some(first) = fields.next() else { break };
        let letter = code.chars().next().unwrap_or('M');
        let (old_path, path) = if matches!(letter, 'R' | 'C') {
            let Some(second) = fields.next() else { break };
            (Some(first.to_owned()), second.to_owned())
        } else {
            (None, first.to_owned())
        };
        let status = match letter {
            'T' => 'M',
            other => other,
        };
        entries.push(Entry { path, old_path, status: status.to_string(), additions: 0, deletions: 0, binary: false });
    }
    if cut {
        entries.pop();
    }
    entries
}

/// Read a `--numstat -z` list into `(additions, deletions, binary)` by path. Binary files report `-`.
fn parse_numstat(text: &str) -> HashMap<String, (u64, u64, bool)> {
    let mut fields = text.split('\0');
    let mut stats = HashMap::new();
    while let Some(record) = fields.next() {
        let mut parts = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(rest)) = (parts.next(), parts.next(), parts.next()) else { break };
        let path = if rest.is_empty() {
            let _old = fields.next();
            let Some(new) = fields.next() else { break };
            new
        } else {
            rest
        };
        let binary = added == "-" && removed == "-";
        stats.insert(path.to_owned(), (added.parse().unwrap_or(0), removed.parse().unwrap_or(0), binary));
    }
    stats
}

/// The changed tracked files for `range`: `[base]` compares the working tree, `[base, head]` compares two commits.
/// The flag is true when a list was cut.
fn tracked(top: &Path, range: &[&str]) -> Result<(Vec<Entry>, bool), String> {
    let common = ["diff", "--no-ext-diff", "--no-textconv", "--no-color", "-M", "-z"];
    let (names, names_cut) = bounded(top, &[&common[..], &["--name-status"], range, &["--"]].concat(), LIST_BYTES)?;
    let (numbers, numbers_cut) = bounded(top, &[&common[..], &["--numstat"], range, &["--"]].concat(), LIST_BYTES)?;
    let stats = parse_numstat(&numbers);
    let mut entries = parse_status(&names, names_cut);
    for entry in &mut entries {
        if let Some(&(added, removed, binary)) = stats.get(&entry.path) {
            (entry.additions, entry.deletions, entry.binary) = (added, removed, binary);
        }
    }
    Ok((entries, names_cut || numbers_cut))
}

/// The untracked files that Git does not ignore. A nested repository shows as a folder and is skipped.
fn untracked(top: &Path) -> Result<(Vec<Entry>, bool), String> {
    let (text, cut) = bounded(top, &["ls-files", "--others", "--exclude-standard", "-z"], LIST_BYTES)?;
    let mut paths: Vec<&str> = text.split('\0').filter(|path| !path.is_empty()).collect();
    if cut {
        paths.pop();
    }
    let entries = paths
        .into_iter()
        .filter(|path| !path.ends_with('/'))
        .map(|path| Entry { path: path.to_owned(), old_path: None, status: "?".into(), additions: 0, deletions: 0, binary: false })
        .collect();
    Ok((entries, cut))
}

/// Drop a partial last line from a cut diff, so the view never shows half a line.
fn trim_to_line(diff: &mut String) {
    if let Some(end) = diff.rfind('\n') {
        diff.truncate(end + 1);
    }
}

/// The diff text of one file, at most `limit` bytes. The flag is true when the diff was cut.
fn file_diff(top: &Path, range: &[&str], entry: &Entry, limit: usize) -> Result<(String, bool), String> {
    let flags = ["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--src-prefix=a/", "--dst-prefix=b/"];
    let (mut diff, cut) = if entry.status == "?" {
        bounded(top, &[&flags[..], &["--no-index", "--", "/dev/null", &entry.path]].concat(), limit)?
    } else {
        let mut args = [&flags[..], &["-M"], range, &["--"]].concat();
        if let Some(old) = &entry.old_path {
            args.push(old);
        }
        args.push(&entry.path);
        bounded(top, &args, limit)?
    };
    if cut {
        trim_to_line(&mut diff);
    }
    Ok((diff, cut))
}

/// Count the lines of an untracked file. A binary file reports `-`.
fn untracked_stat(top: &Path, path: &str) -> Result<(u64, bool), String> {
    let (text, _) = bounded(top, &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-index", "--numstat", "--", "/dev/null", path], 4096)?;
    let mut parts = text.split('\t');
    let (added, removed) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if added == "-" && removed == "-" {
        return Ok((0, true));
    }
    Ok((added.trim().parse().unwrap_or(0), false))
}

/// Sort and bound `entries`, then read each file's counts and diff against `range`. The flag is true when a
/// limit cut the result.
fn diff_files(top: &Path, mut entries: Vec<Entry>, range: &[&str]) -> Result<(Vec<ChangeFile>, bool), String> {
    let mut truncated = false;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    truncated |= entries.len() > MAX_FILES;
    entries.truncate(MAX_FILES);

    let mut remaining = TOTAL_BYTES;
    let mut files = Vec::with_capacity(entries.len());
    for mut entry in entries {
        if entry.status == "?" {
            let (added, binary) = untracked_stat(top, &entry.path)?;
            (entry.additions, entry.binary) = (added, binary);
        }
        let limit = remaining.min(FILE_BYTES);
        let (diff, cut) = if entry.binary {
            (String::new(), false)
        } else if limit == 0 {
            (String::new(), true)
        } else {
            file_diff(top, range, &entry, limit)?
        };
        remaining -= diff.len();
        truncated |= cut;
        files.push(ChangeFile {
            path: entry.path,
            old_path: entry.old_path,
            status: entry.status,
            additions: entry.additions,
            deletions: entry.deletions,
            binary: entry.binary,
            diff,
            truncated: cut,
        });
    }
    Ok((files, truncated))
}

/// The files that `head` changed since `base`, with bounded diffs. Used by the merge review.
pub(crate) fn committed_files(top: &Path, base: &str, head: &str) -> Result<(Vec<ChangeFile>, bool), String> {
    let range = [base, head];
    let (entries, cut) = tracked(top, &range)?;
    let (files, files_cut) = diff_files(top, entries, &range)?;
    Ok((files, cut || files_cut))
}

/// The changes of the folder of a channel.
///
/// Errors: the folder is missing, or it is not a Git repository, or it has no commit yet. A scope with no base is
/// not an error. It returns no files, `base: None`, and a `reason`.
pub fn channel_changes(folder: &Path, scope: Scope) -> Result<ChannelChanges, String> {
    let top = toplevel(folder)?;
    let head = git(&top, &["rev-parse", "--verify", "--quiet", "HEAD"]).map_err(|_| "This repository has no commits yet.".to_string())?;
    let result = |base: Option<String>, reason: Option<String>, files: Vec<ChangeFile>, truncated: bool| ChannelChanges {
        root: top.to_string_lossy().into_owned(),
        scope: scope.name().into(),
        base,
        reason,
        files,
        truncated,
    };
    let base = match scope {
        Scope::Uncommitted => head,
        Scope::Branch => match branch_base(&top) {
            Ok(base) => base,
            Err(reason) => return Ok(result(None, Some(reason), Vec::new(), false)),
        },
    };
    let short = git(&top, &["rev-parse", "--short", &base])?;

    let (mut entries, mut truncated) = tracked(&top, &[&base])?;
    let (others, others_cut) = untracked(&top)?;
    entries.extend(others);
    truncated |= others_cut;
    let (files, cut) = diff_files(&top, entries, &[&base])?;
    truncated |= cut;
    Ok(result(Some(short), None, files, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_commit::{CommitRequest, commit, tests::repository};

    fn run(root: &Path, args: &[&str]) {
        git(root, args).unwrap();
    }

    fn changes(root: &Path, scope: Scope) -> ChannelChanges {
        channel_changes(root, scope).unwrap()
    }

    fn file<'a>(result: &'a ChannelChanges, path: &str) -> &'a ChangeFile {
        result.files.iter().find(|file| file.path == path).unwrap_or_else(|| panic!("no file {path}"))
    }

    fn commit_all(root: &Path, message: &str) {
        commit(root, &CommitRequest { message: message.into(), paths: None }, false).unwrap();
    }

    #[test]
    fn an_uncommitted_edit_has_a_diff_and_counts() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        commit_all(&root, "Add a");
        std::fs::write(root.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        let result = changes(&root, Scope::Uncommitted);
        assert_eq!(result.scope, "uncommitted");
        assert!(result.reason.is_none() && !result.truncated);
        assert_eq!(result.base.as_deref(), Some(git(&root, &["rev-parse", "--short", "HEAD"]).unwrap().as_str()));
        let a = file(&result, "a.txt");
        assert_eq!((a.status.as_str(), a.additions, a.deletions, a.binary), ("M", 2, 1, false));
        assert!(a.diff.contains("-two") && a.diff.contains("+TWO") && a.diff.contains("+three"), "{}", a.diff);
        assert!(a.old_path.is_none() && !a.truncated);
    }

    #[test]
    fn a_clean_tree_has_no_files() {
        let (_dir, _repo, root) = repository();
        assert!(changes(&root, Scope::Uncommitted).files.is_empty());
    }

    #[test]
    fn an_untracked_file_diffs_against_nothing_and_counts_its_lines() {
        let (_dir, _repo, root) = repository();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/new.txt"), "x\ny\nz\n").unwrap();
        let result = changes(&root, Scope::Uncommitted);
        let new = file(&result, "src/new.txt");
        assert_eq!((new.status.as_str(), new.additions, new.deletions), ("?", 3, 0));
        assert!(new.diff.contains("+x") && new.diff.contains("new file mode"), "{}", new.diff);
    }

    #[test]
    fn an_ignored_file_is_not_listed() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join(".gitignore"), "skip.txt\n").unwrap();
        std::fs::write(root.join("skip.txt"), "x\n").unwrap();
        let paths: Vec<_> = changes(&root, Scope::Uncommitted).files.into_iter().map(|file| file.path).collect();
        assert_eq!(paths, [".gitignore"]);
    }

    #[test]
    fn a_deleted_file_shows_removed_lines() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("gone.txt"), "a\nb\n").unwrap();
        commit_all(&root, "Add gone");
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        let result = changes(&root, Scope::Uncommitted);
        let gone = file(&result, "gone.txt");
        assert_eq!((gone.status.as_str(), gone.additions, gone.deletions), ("D", 0, 2));
        assert!(gone.diff.contains("-a") && gone.diff.contains("deleted file mode"), "{}", gone.diff);
    }

    #[test]
    fn a_staged_rename_reports_the_old_path() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("old.txt"), "1\n2\n3\n4\n5\n6\n7\n8\n").unwrap();
        commit_all(&root, "Add old");
        run(&root, &["mv", "old.txt", "new.txt"]);
        std::fs::write(root.join("new.txt"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n").unwrap();
        let result = changes(&root, Scope::Uncommitted);
        assert_eq!(result.files.len(), 1, "{:?}", result.files);
        let renamed = &result.files[0];
        assert_eq!((renamed.path.as_str(), renamed.old_path.as_deref(), renamed.status.as_str()), ("new.txt", Some("old.txt"), "R"));
        assert_eq!((renamed.additions, renamed.deletions), (1, 0));
        assert!(renamed.diff.contains("rename from old.txt") && renamed.diff.contains("+9"), "{}", renamed.diff);
    }

    #[test]
    fn a_binary_file_has_no_diff_text() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 0, 255]).unwrap();
        commit_all(&root, "Add blob");
        std::fs::write(root.join("blob.bin"), [0u8, 9, 9, 0, 1, 1]).unwrap();
        std::fs::write(root.join("fresh.bin"), [0u8, 7, 0, 7]).unwrap();
        let result = changes(&root, Scope::Uncommitted);
        for name in ["blob.bin", "fresh.bin"] {
            let entry = file(&result, name);
            assert!(entry.binary, "{name}");
            assert_eq!((entry.diff.as_str(), entry.additions, entry.deletions), ("", 0, 0), "{name}");
        }
    }

    #[test]
    fn files_are_sorted_by_path() {
        let (_dir, _repo, root) = repository();
        for name in ["b.txt", "a.txt", "c/d.txt"] {
            if let Some(parent) = Path::new(name).parent() {
                std::fs::create_dir_all(root.join(parent)).unwrap();
            }
            std::fs::write(root.join(name), "x\n").unwrap();
        }
        let paths: Vec<_> = changes(&root, Scope::Uncommitted).files.into_iter().map(|file| file.path).collect();
        assert_eq!(paths, ["a.txt", "b.txt", "c/d.txt"]);
    }

    #[test]
    fn the_branch_scope_has_committed_and_uncommitted_work() {
        let (_dir, repo, root) = repository();
        std::fs::write(root.join("committed.txt"), "c\n").unwrap();
        commit_all(&root, "Add committed");
        std::fs::write(root.join("committed.txt"), "c\nmore\n").unwrap();
        std::fs::write(root.join("loose.txt"), "l\n").unwrap();
        let branch = changes(&root, Scope::Branch);
        assert!(branch.reason.is_none());
        let base = git(&repo, &["rev-parse", "--short", "main"]).unwrap();
        assert_eq!(branch.base.as_deref(), Some(base.as_str()));
        let paths: Vec<_> = branch.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["committed.txt", "loose.txt"]);
        let committed = file(&branch, "committed.txt");
        assert_eq!((committed.status.as_str(), committed.additions), ("A", 2));
        // The uncommitted scope sees the edit and the untracked file, but compares with HEAD.
        let uncommitted = changes(&root, Scope::Uncommitted);
        assert_eq!(file(&uncommitted, "committed.txt").additions, 1);
        assert_eq!(file(&uncommitted, "committed.txt").status, "M");
    }

    #[test]
    fn the_branch_scope_uses_the_merge_base_after_the_default_branch_moves() {
        let (_dir, repo, root) = repository();
        std::fs::write(repo.join("main-only.txt"), "m\n").unwrap();
        commit(&repo, &CommitRequest { message: "Main moves".into(), paths: None }, true).unwrap();
        std::fs::write(root.join("mine.txt"), "x\n").unwrap();
        commit_all(&root, "Mine");
        let branch = changes(&root, Scope::Branch);
        let paths: Vec<_> = branch.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["mine.txt"]);
    }

    #[test]
    fn the_default_branch_has_no_base_and_gives_a_reason() {
        let (_dir, repo, _root) = repository();
        std::fs::write(repo.join("x.txt"), "x\n").unwrap();
        let branch = changes(&repo, Scope::Branch);
        assert!(branch.base.is_none() && branch.files.is_empty() && !branch.truncated);
        assert!(branch.reason.as_deref().unwrap().contains("main"), "{:?}", branch.reason);
        assert_eq!(changes(&repo, Scope::Uncommitted).files.len(), 1);
    }

    #[test]
    fn a_detached_head_has_no_base_and_gives_a_reason() {
        let (_dir, _repo, root) = repository();
        run(&root, &["checkout", "-q", "--detach"]);
        let branch = changes(&root, Scope::Branch);
        assert!(branch.base.is_none() && branch.files.is_empty());
        assert!(branch.reason.as_deref().unwrap().contains("detached"), "{:?}", branch.reason);
    }

    #[test]
    fn a_folder_that_is_not_a_repository_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = channel_changes(dir.path(), Scope::Uncommitted).unwrap_err();
        assert!(error.contains("not a Git repository"), "{error}");
        let missing = channel_changes(&dir.path().join("none"), Scope::Branch).unwrap_err();
        assert!(missing.contains("does not exist"), "{missing}");
    }

    #[test]
    fn a_repository_with_no_commit_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q", "-b", "main"]);
        let error = channel_changes(dir.path(), Scope::Uncommitted).unwrap_err();
        assert!(error.contains("no commits"), "{error}");
    }

    #[test]
    fn a_subfolder_reports_the_repository_top() {
        let (_dir, _repo, root) = repository();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/x.txt"), "x\n").unwrap();
        let result = changes(&root.join("sub"), Scope::Uncommitted);
        assert_eq!(result.files[0].path, "sub/x.txt");
        assert_eq!(PathBuf::from(&result.root).canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn an_unknown_scope_is_an_error() {
        assert_eq!(Scope::parse("branch"), Ok(Scope::Branch));
        assert!(Scope::parse("everything").is_err());
    }

    #[test]
    fn a_long_diff_is_cut_per_file_on_a_line_boundary() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("long.txt"), "long line of text\n".repeat(12000)).unwrap();
        std::fs::write(root.join("short.txt"), "short\n").unwrap();
        let result = changes(&root, Scope::Uncommitted);
        let long = file(&result, "long.txt");
        assert!(long.truncated && result.truncated);
        assert!(long.diff.len() <= FILE_BYTES && long.diff.ends_with('\n'));
        assert_eq!(long.additions, 12000);
        assert!(!file(&result, "short.txt").truncated);
    }

    #[test]
    fn the_file_count_and_total_size_are_capped() {
        let (_dir, _repo, root) = repository();
        for i in 0..205 {
            std::fs::write(root.join(format!("file{i:03}")), "long line\n".repeat(3000)).unwrap();
        }
        let result = changes(&root, Scope::Uncommitted);
        assert!(result.truncated);
        assert_eq!(result.files.len(), MAX_FILES);
        assert!(result.files.iter().map(|file| file.diff.len()).sum::<usize>() <= TOTAL_BYTES);
        assert!(result.files.last().unwrap().truncated);
        assert!(result.files.last().unwrap().diff.is_empty());
    }

    #[test]
    fn the_result_serializes_with_camel_case_names() {
        let (_dir, _repo, root) = repository();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        let value = serde_json::to_value(changes(&root, Scope::Uncommitted)).unwrap();
        assert!(value["files"][0].get("oldPath").is_some() && value["files"][0].get("additions").is_some());
        assert!(value.get("reason").is_some() && value.get("base").is_some());
    }
}
