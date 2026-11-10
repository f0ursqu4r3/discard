//! Bounded changes recorded during one workspace response.

use crate::{model::Block, worktree_access::{git, git_bounded}};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path, time::Duration};

pub struct StartSnapshot { pub head: Option<String>, pub dirty: BTreeMap<String, Option<String>> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangesState { Completed, Stopped, Failed }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedCommit { pub id: String, pub subject: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile { pub path: String, pub status: String, pub diff: String, pub truncated: bool, pub preexisting: bool }

fn dirty_paths(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = git(root, &["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    let mut entries = text.split('\0'); let mut result = BTreeMap::new();
    while let Some(entry) = entries.next() {
        let kind = entry.as_bytes().first().copied();
        let (path, status) = match kind {
            Some(b'?') => (entry.get(2..), "?"),
            Some(b'1') => (entry.splitn(9, ' ').nth(8), entry.get(2..4).unwrap_or("M")),
            Some(b'2') => { let _ = entries.next(); (entry.splitn(10, ' ').nth(9), entry.get(2..4).unwrap_or("R")) }
            Some(b'u') => (entry.splitn(11, ' ').nth(10), "U"),
            _ => continue,
        };
        if let Some(path) = path { result.insert(path.to_owned(), status.to_owned()); }
    }
    Ok(result)
}
fn content_hash(root: &Path, path: &str) -> Result<Option<String>, String> {
    let path = root.join(path);
    let metadata = match std::fs::symlink_metadata(&path) { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None), Err(e) => return Err(e.to_string()) };
    let mut hash = Sha256::new();
    if metadata.file_type().is_symlink() {
        use std::os::unix::ffi::OsStrExt;
        hash.update(std::fs::read_link(path).map_err(|e| e.to_string())?.as_os_str().as_bytes());
    } else if metadata.is_file() {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?; let mut bytes = [0; 8192];
        loop { let n = file.read(&mut bytes).map_err(|e| e.to_string())?; if n == 0 { break; } hash.update(&bytes[..n]); }
    } else {
        hash.update(git(&path, &["rev-parse", "HEAD"])?); hash.update(git(&path, &["status", "--porcelain=v2", "-z"])?);
    }
    Ok(Some(format!("{:x}", hash.finalize())))
}
pub fn snapshot(root: &Path) -> Result<StartSnapshot, String> {
    let head = git(root, &["rev-parse", "--verify", "HEAD"]).ok();
    let dirty = dirty_paths(root)?.keys().map(|path| Ok((path.clone(), content_hash(root, path)?))).collect::<Result<_, String>>()?;
    Ok(StartSnapshot { head, dirty })
}
pub fn changes(root: &Path, start: &StartSnapshot, state: ChangesState) -> Result<Block, String> {
    let mut paths = BTreeMap::new(); let mut commits = Vec::new(); let mut truncated = false;
    if let Some(head) = &start.head {
        let range = format!("{head}..HEAD");
        let names = git(root, &["diff", "--no-renames", "--name-status", "-z", &range, "--"])?;
        let mut fields = names.split('\0');
        while let Some(status) = fields.next() { if let Some(path) = fields.next() && !path.is_empty() { paths.insert(path.to_owned(), status.to_owned()); } }
        let (log, cut) = git_bounded(root, &["log", "--format=%H%x00%s", &range], 32768, Duration::from_secs(10))?; truncated |= cut;
        commits = log.lines().filter_map(|line| line.split_once('\0').map(|(id, subject)| ChangedCommit { id: id.into(), subject: subject.into() })).collect();
    }
    let dirty = dirty_paths(root)?;
    for (path, status) in &dirty {
        let current = content_hash(root, path)?;
        if start.dirty.get(path) != Some(&current) { paths.insert(path.clone(), status.clone()); }
    }
    for (path, original) in &start.dirty {
        if !dirty.contains_key(path) && content_hash(root, path)? != *original { paths.entry(path.clone()).or_insert_with(|| "M".into()); }
    }
    truncated |= paths.len() > 200;
    let mut remaining = 128 * 1024; let mut files = Vec::new();
    for (path, status) in paths.into_iter().take(200) {
        let limit = remaining.min(16 * 1024);
        let (mut diff, cut) = if limit == 0 { (String::new(), true) }
            else if dirty.get(&path).is_some_and(|s| s == "?") {
                git_bounded(root, &["diff", "--no-ext-diff", "--no-textconv", "--no-index", "--", "/dev/null", &path], limit, Duration::from_secs(10))?
            } else { git_bounded(root, &["diff", "--no-ext-diff", "--no-textconv", start.head.as_deref().unwrap_or("HEAD"), "--", &path], limit, Duration::from_secs(10))? };
        if diff.len() > limit { let mut end = limit; while !diff.is_char_boundary(end) { end -= 1; } diff.truncate(end); }
        remaining -= diff.len(); truncated |= cut;
        files.push(ChangedFile { preexisting: start.dirty.contains_key(&path), path, status, diff, truncated: cut });
    }
    Ok(Block::Changes { root: root.to_string_lossy().into(), state, commits, files, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{workspace_commit::{commit, CommitRequest, tests::repository}, model::Block};
    fn files(block: Block) -> Vec<ChangedFile> { let Block::Changes { files, .. } = block else { panic!() }; files }
    #[test]
    fn reports_new_commits_and_uncommitted_edits() {
        let (_dir, _repo, root) = repository(); let start = snapshot(&root).unwrap();
        std::fs::write(root.join("a"), "committed").unwrap(); commit(&root, &CommitRequest { message: "Change".into(), paths: None }, false).unwrap();
        std::fs::write(root.join("b"), "uncommitted").unwrap();
        let Block::Changes { commits, files, .. } = changes(&root, &start, ChangesState::Completed).unwrap() else { panic!() };
        assert_eq!(commits.len(), 1); assert_eq!(commits[0].subject, "Change"); assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }
    #[test]
    fn ignores_files_dirty_before_the_response_when_unchanged() {
        let (_dir, _repo, root) = repository(); std::fs::write(root.join("a"), "user work").unwrap(); let start = snapshot(&root).unwrap();
        assert!(files(changes(&root, &start, ChangesState::Completed).unwrap()).is_empty());
    }
    #[test]
    fn marks_preexisting_dirty_files_that_changed() {
        let (_dir, _repo, root) = repository(); std::fs::write(root.join("a"), "user work").unwrap(); let start = snapshot(&root).unwrap();
        std::fs::write(root.join("a"), "agent change").unwrap();
        assert!(files(changes(&root, &start, ChangesState::Stopped).unwrap())[0].preexisting);
    }
    #[test]
    fn limits_diff_size() {
        let (_dir, _repo, root) = repository(); let start = snapshot(&root).unwrap();
        for i in 0..205 { std::fs::write(root.join(format!("file{i:03}")), "long line\n".repeat(3000)).unwrap(); }
        let Block::Changes { files, truncated, .. } = changes(&root, &start, ChangesState::Completed).unwrap() else { panic!() };
        assert!(truncated); assert_eq!(files.len(), 200); assert!(files.iter().all(|f| f.diff.len() <= 16384));
        assert!(files.iter().map(|f| f.diff.len()).sum::<usize>() <= 131072); assert!(files[0].truncated);
    }
    #[test]
    fn reports_untracked_new_files() {
        let (_dir, _repo, root) = repository(); let start = snapshot(&root).unwrap(); std::fs::write(root.join("new.txt"), "new\n").unwrap();
        let result = files(changes(&root, &start, ChangesState::Failed).unwrap());
        assert_eq!(result[0].path, "new.txt"); assert!(result[0].diff.contains("+new")); assert!(!result[0].preexisting);
    }
}
