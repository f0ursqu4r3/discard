//! Paths that an autorun session must not read. The sandbox limits writes, not reads,
//! so the adapter denies reads of credential stores explicitly.

use std::path::{Path, PathBuf};

/// Entries in the user's home directory that hold credentials or provider logins.
pub const HOME_CREDENTIAL_PATHS: &[&str] =
    &[".ssh", ".aws", ".config/gh", ".gnupg", ".netrc", ".docker/config.json", ".claude", ".claude.json", ".codex"];

/// Entries in `HOME_CREDENTIAL_PATHS` that are files, not directories.
const HOME_CREDENTIAL_FILES: &[&str] = &[".netrc", ".docker/config.json", ".claude.json"];

/// Paths to deny reading: the credential entries in `home`, plus each direct child of
/// `discard_home` that is not an ancestor of (or equal to) `root`. Missing paths are kept.
pub fn denied_read_paths(home: &Path, discard_home: &Path, root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = HOME_CREDENTIAL_PATHS.iter().map(|p| home.join(p)).collect();
    if let Ok(entries) = std::fs::read_dir(discard_home) {
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| !root.starts_with(p)).collect();
        children.sort();
        paths.extend(children);
    }
    paths
}

/// Read denials for one autorun session: sandbox paths and matching `Read(...)` permission rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadDenials {
    pub paths: Vec<PathBuf>,
    pub rules: Vec<String>,
}

impl ReadDenials {
    pub fn new(home: &Path, discard_home: &Path, root: &Path) -> Self {
        let paths = denied_read_paths(home, discard_home, root);
        let rules = paths
            .iter()
            .map(|p| if is_file_path(home, p) { format!("Read({})", p.display()) } else { format!("Read({}/**)", p.display()) })
            .collect();
        Self { paths, rules }
    }
}

/// Whether `path` is a file, so a `Read` rule names it directly instead of adding `/**`.
/// Home entries use the fixed list because they may not exist. Others use the filesystem.
fn is_file_path(home: &Path, path: &Path) -> bool {
    if let Ok(rest) = path.strip_prefix(home) {
        if HOME_CREDENTIAL_FILES.iter().any(|f| rest == Path::new(f)) {
            return true;
        }
        if HOME_CREDENTIAL_PATHS.iter().any(|f| rest == Path::new(f)) {
            return false;
        }
    }
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let discard_home = tmp.path().join("discard");
        let root = discard_home.join("worktrees").join("w1");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(discard_home.join("worktrees").join("w2")).unwrap();
        std::fs::create_dir_all(discard_home.join("other")).unwrap();
        std::fs::write(discard_home.join("discard.db"), "").unwrap();
        (tmp, home, discard_home, root)
    }

    #[test]
    fn denied_reads_cover_credential_paths() {
        let (_tmp, home, discard_home, root) = layout();
        let paths = denied_read_paths(&home, &discard_home, &root);
        for p in [".ssh", ".aws", ".config/gh", ".gnupg", ".netrc", ".docker/config.json", ".claude", ".claude.json", ".codex"] {
            assert!(paths.contains(&home.join(p)), "{p}");
        }
    }

    #[test]
    fn read_denials_keep_the_session_root_readable() {
        let (_tmp, home, discard_home, root) = layout();
        let paths = denied_read_paths(&home, &discard_home, &root);
        assert!(paths.contains(&discard_home.join("discard.db")));
        assert!(paths.contains(&discard_home.join("other")));
        assert!(!paths.iter().any(|p| root.starts_with(p)));
    }

    #[test]
    fn read_rules_name_files_directly_and_directories_with_a_glob() {
        let (_tmp, home, discard_home, root) = layout();
        let denials = ReadDenials::new(&home, &discard_home, &root);
        assert_eq!(denials.rules.len(), denials.paths.len());
        for expected in [
            format!("Read({}/**)", home.join(".ssh").display()),
            format!("Read({})", home.join(".claude.json").display()),
            format!("Read({})", home.join(".docker/config.json").display()),
            format!("Read({})", discard_home.join("discard.db").display()),
            format!("Read({}/**)", discard_home.join("other").display()),
        ] {
            assert!(denials.rules.contains(&expected), "{expected}");
        }
    }

    #[test]
    fn file_paths_come_from_the_fixed_list_for_home_and_the_filesystem_otherwise() {
        let (_tmp, home, discard_home, _root) = layout();
        assert!(is_file_path(&home, &home.join(".claude.json")));
        assert!(is_file_path(&home, &home.join(".docker/config.json")));
        assert!(is_file_path(&home, &home.join(".netrc")));
        assert!(!is_file_path(&home, &home.join(".ssh")));
        assert!(is_file_path(&home, &discard_home.join("discard.db")));
        assert!(!is_file_path(&home, &discard_home.join("other")));
    }
}
