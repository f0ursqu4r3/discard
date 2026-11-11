//! Paths that an autorun session must not read. The sandbox limits writes, not reads,
//! so the adapter denies reads of credential stores explicitly.

use std::path::{Path, PathBuf};

/// Entries in the user's home directory that hold credentials or provider logins.
/// `.claude` is also denied, except the session's own transcript directory (see `denied_read_paths`).
pub const HOME_CREDENTIAL_PATHS: &[&str] =
    &[".ssh", ".aws", ".config/gh", ".gnupg", ".netrc", ".docker/config.json", ".claude", ".claude.json", ".codex"];

/// Entries in `HOME_CREDENTIAL_PATHS` that are files, not directories.
const HOME_CREDENTIAL_FILES: &[&str] = &[".netrc", ".docker/config.json", ".claude.json"];

/// Claude shortens project directory names longer than this with a hash. Discard does not match those.
const CLAUDE_PROJECT_NAME_MAX: usize = 200;

/// Walk limits for symlinks inside a credential directory.
const WALK_MAX_DEPTH: usize = 4;
const WALK_MAX_ENTRIES: usize = 2000;

/// Paths to deny reading:
/// - the credential entries in `home`;
/// - the real target of each credential entry that is a symlink (dotfiles layouts such as
///   `~/.ssh -> ~/dotfiles/ssh`), because Seatbelt matches resolved paths;
/// - for each credential directory (except `.claude`), the real target of every symlink inside it
///   that points outside it. The walk goes at most 4 levels deep, visits at most 2000 entries, and
///   does not follow directory symlinks. Dangling symlinks are skipped;
/// - every entry of `claude_config` (the Claude CLI configuration directory, `~/.claude` or
///   `CLAUDE_CONFIG_DIR`) except `projects/<session root>`, where Claude saves large tool outputs
///   that the session reads back. If `claude_config` cannot be listed, it is denied whole;
/// - each directory in `extra_dirs` (relocated credential directories such as `CODEX_HOME`), with
///   the same symlink handling as home entries;
/// - every file under `discard_home` except the session root and its ancestors. At each level from
///   `discard_home` down to `root`, every entry not on the path to `root` is denied.
///
/// All inputs are canonicalized first, so a symlink does not deny the root or miss a credential
/// directory. A path that cannot be canonicalized (for example, a missing one) is kept as given.
/// A resolved symlink target is never denied if it equals or contains the session root, or contains
/// the kept `projects` directory of `claude_config`.
pub fn denied_read_paths(home: &Path, discard_home: &Path, root: &Path, claude_config: &Path, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let home = canonical(home);
    let claude_config = canonical(claude_config);
    let given_root = root;
    let root = canonical(root);
    let guard = Guard { root: &root, claude_projects: claude_config.join("projects") };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in HOME_CREDENTIAL_PATHS {
        let link = home.join(entry);
        if canonical(&link) == claude_config {
            continue;
        }
        // `.claude` is walked by `deny_claude_config`, so it is not walked here.
        deny_entry(link, *entry != ".claude", &guard, &mut paths);
    }
    deny_claude_config(&claude_config, &[given_root, &root], &mut paths);
    for dir in extra_dirs {
        deny_entry(dir.clone(), true, &guard, &mut paths);
    }
    deny_off_path(&canonical(discard_home), &root, &mut paths);
    paths
}

/// Paths that a resolved symlink target must not deny.
struct Guard<'a> {
    root: &'a Path,
    claude_projects: PathBuf,
}

impl Guard<'_> {
    /// Add `path` unless it is a duplicate, contains the session root, or contains the kept projects directory.
    fn deny(&self, path: PathBuf, paths: &mut Vec<PathBuf>) {
        if self.root.starts_with(&path) || self.claude_projects.starts_with(&path) || paths.contains(&path) {
            return;
        }
        paths.push(path);
    }
}

/// Deny a credential entry. A missing entry is kept as given. For an existing entry, also deny its
/// real path if that differs, and optionally the targets of symlinks inside it.
fn deny_entry(entry: PathBuf, walk: bool, guard: &Guard, paths: &mut Vec<PathBuf>) {
    let resolved = entry.canonicalize().ok();
    if !paths.contains(&entry) {
        paths.push(entry.clone());
    }
    let Some(resolved) = resolved else { return };
    if resolved != entry {
        guard.deny(resolved.clone(), paths);
    }
    if walk && resolved.is_dir() {
        let mut visited = 0;
        deny_symlink_targets(&resolved, &resolved, 1, &mut visited, guard, paths);
    }
}

/// Walk `dir` and deny the real target of each symlink that points outside `base`.
fn deny_symlink_targets(dir: &Path, base: &Path, depth: usize, visited: &mut usize, guard: &Guard, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in children {
        if *visited >= WALK_MAX_ENTRIES {
            return;
        }
        *visited += 1;
        let Ok(meta) = std::fs::symlink_metadata(&child) else { continue };
        if meta.file_type().is_symlink() {
            if let Ok(target) = child.canonicalize()
                && !target.starts_with(base)
            {
                guard.deny(target, paths);
            }
        } else if meta.is_dir() && depth < WALK_MAX_DEPTH {
            deny_symlink_targets(&child, base, depth + 1, visited, guard, paths);
        }
    }
}

/// Claude's directory name for a project: the absolute path with every character that is not an
/// ASCII letter or digit replaced by `-`. `None` when Claude would shorten the name.
pub fn claude_project_dir_name(root: &Path) -> Option<String> {
    let name: String = root.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    (name.len() <= CLAUDE_PROJECT_NAME_MAX).then_some(name)
}

/// Deny every entry of the Claude configuration directory except `projects/<root>`.
fn deny_claude_config(claude_config: &Path, roots: &[&Path], paths: &mut Vec<PathBuf>) {
    let projects = claude_config.join("projects");
    let kept: Vec<PathBuf> = roots.iter().filter_map(|r| claude_project_dir_name(r)).map(|name| projects.join(name)).collect();
    let Ok(entries) = std::fs::read_dir(claude_config) else {
        paths.push(claude_config.to_path_buf());
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in children {
        if child != projects || kept.is_empty() {
            paths.push(child);
            continue;
        }
        let Ok(projects_entries) = std::fs::read_dir(&projects) else {
            paths.push(projects.clone());
            continue;
        };
        let mut names: Vec<PathBuf> = projects_entries.flatten().map(|e| e.path()).collect();
        names.sort();
        paths.extend(names.into_iter().filter(|p| !kept.contains(p)));
    }
}

/// Deny each entry of `dir` that is not `root` or an ancestor of `root`. Descend into ancestors.
fn deny_off_path(dir: &Path, root: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in children {
        if child == root {
            continue;
        }
        if root.starts_with(&child) {
            deny_off_path(&child, root, paths);
        } else {
            paths.push(child);
        }
    }
}

/// Escape the glob characters of a `Read(...)` permission rule path with a backslash.
pub fn escape_glob(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Read denials for one autorun session: sandbox paths and matching `Read(...)` permission rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadDenials {
    pub paths: Vec<PathBuf>,
    pub rules: Vec<String>,
}

impl ReadDenials {
    pub fn new(home: &Path, discard_home: &Path, root: &Path, claude_config: &Path, extra_dirs: &[PathBuf]) -> Self {
        let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        let paths = denied_read_paths(&home, discard_home, root, claude_config, extra_dirs);
        let rules = paths
            .iter()
            // Claude permission rules read `//path` as an absolute filesystem path.
            .map(|p| if is_file_path(&home, p) { format!("Read(/{})", escape_glob(p)) } else { format!("Read(/{}/**)", escape_glob(p)) })
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
        let base = tmp.path().canonicalize().unwrap();
        let home = base.join("home");
        let discard_home = base.join("discard");
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
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        for p in [".ssh", ".aws", ".config/gh", ".gnupg", ".netrc", ".docker/config.json", ".claude", ".claude.json", ".codex"] {
            assert!(paths.contains(&home.join(p)), "{p}");
        }
    }

    #[test]
    fn read_denials_keep_the_session_root_readable() {
        let (_tmp, home, discard_home, root) = layout();
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(paths.contains(&discard_home.join("discard.db")));
        assert!(paths.contains(&discard_home.join("other")));
        assert!(paths.contains(&discard_home.join("worktrees").join("w2")));
        assert!(!paths.contains(&discard_home.join("worktrees")));
        assert!(!paths.iter().any(|p| root.starts_with(p)));
        assert!(root.is_dir());
    }

    #[test]
    fn a_symlinked_discard_home_does_not_deny_the_session_root() {
        let (tmp, home, discard_home, root) = layout();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&discard_home, &link).unwrap();
        for (given_home, given_root) in [(link.clone(), root.clone()), (discard_home.clone(), link.join("worktrees").join("w1"))] {
            let paths = denied_read_paths(&home, &given_home, &given_root, &home.join(".claude"), &[]);
            assert!(!paths.iter().any(|p| root.starts_with(p)), "{given_home:?} {given_root:?}");
            assert!(paths.contains(&discard_home.join("worktrees").join("w2")));
            assert!(paths.contains(&discard_home.join("other")));
        }
    }

    #[test]
    fn relocated_credential_directories_are_denied_too() {
        let (tmp, home, discard_home, root) = layout();
        let claude = tmp.path().join("claude-cfg");
        let codex = tmp.path().join("codex-cfg");
        let paths = denied_read_paths(&home, &discard_home, &root, &claude, std::slice::from_ref(&codex));
        assert!(paths.contains(&claude) && paths.contains(&codex));
        assert!(paths.contains(&home.join(".claude")), "defaults stay denied");
        let denials = ReadDenials::new(&home, &discard_home, &root, &claude, &[]);
        assert!(denials.rules.contains(&format!("Read(/{}/**)", claude.display())));
    }

    #[test]
    fn read_rules_name_files_directly_and_directories_with_a_glob() {
        let (_tmp, home, discard_home, root) = layout();
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert_eq!(denials.rules.len(), denials.paths.len());
        assert!(denials.rules.iter().all(|r| r.starts_with("Read(//")), "absolute rules need the // prefix");
        for expected in [
            format!("Read(/{}/**)", home.join(".ssh").display()),
            format!("Read(/{})", home.join(".claude.json").display()),
            format!("Read(/{})", home.join(".docker/config.json").display()),
            format!("Read(/{})", discard_home.join("discard.db").display()),
            format!("Read(/{}/**)", discard_home.join("other").display()),
            format!("Read(/{}/**)", discard_home.join("worktrees").join("w2").display()),
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

    #[test]
    fn the_session_transcript_directory_stays_readable() {
        let (_tmp, home, discard_home, root) = layout();
        let claude = home.join(".claude");
        let own = claude.join("projects").join(claude_project_dir_name(&root).unwrap());
        let other = claude.join("projects").join("-other-project");
        for dir in [&own, &other, &claude.join("todos")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(claude.join("history.jsonl"), "").unwrap();
        let paths = denied_read_paths(&home, &discard_home, &root, &claude, &[]);
        assert!(!paths.iter().any(|p| own.starts_with(p)), "{paths:#?}");
        for denied in [&other, &claude.join("todos"), &claude.join("history.jsonl")] {
            assert!(paths.contains(denied), "{denied:?}");
        }
    }

    #[test]
    fn a_missing_or_unlistable_claude_config_directory_is_denied_whole() {
        let (tmp, home, discard_home, root) = layout();
        let missing = tmp.path().join("no-such-config");
        assert!(denied_read_paths(&home, &discard_home, &root, &missing, &[]).contains(&missing));
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(paths.contains(&home.join(".claude")));
    }

    #[test]
    fn a_relocated_claude_config_directory_also_denies_the_default() {
        let (tmp, home, discard_home, root) = layout();
        let claude = tmp.path().canonicalize().unwrap().join("claude-cfg");
        std::fs::create_dir_all(claude.join("projects").join(claude_project_dir_name(&root).unwrap())).unwrap();
        std::fs::create_dir_all(claude.join("todos")).unwrap();
        let paths = denied_read_paths(&home, &discard_home, &root, &claude, &[]);
        assert!(paths.contains(&home.join(".claude")));
        assert!(paths.contains(&claude.join("todos")));
        assert!(!paths.iter().any(|p| claude.join("projects").starts_with(p)));
    }

    #[test]
    fn a_symlinked_default_claude_directory_is_walked_not_denied_whole() {
        let (tmp, home, discard_home, root) = layout();
        let claude = tmp.path().canonicalize().unwrap().join("claude-cfg");
        let own = claude.join("projects").join(claude_project_dir_name(&root).unwrap());
        std::fs::create_dir_all(&own).unwrap();
        std::os::unix::fs::symlink(&claude, home.join(".claude")).unwrap();
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(!paths.iter().any(|p| own.starts_with(p) || home.join(".claude") == *p), "{paths:#?}");
    }

    #[test]
    fn claude_project_names_replace_every_other_character_with_a_dash() {
        assert_eq!(claude_project_dir_name(Path::new("/Users/a.b/git/x_y")).as_deref(), Some("-Users-a-b-git-x-y"));
        assert_eq!(claude_project_dir_name(Path::new("/private/var/folders/T/.tmp1")).as_deref(), Some("-private-var-folders-T--tmp1"));
        // Claude shortens long names with a hash. Discard does not match them, so they stay denied.
        assert_eq!(claude_project_dir_name(&PathBuf::from(format!("/{}", "a".repeat(250)))), None);
    }

    #[test]
    fn home_and_extra_directories_are_canonicalized() {
        let (tmp, home, discard_home, root) = layout();
        let base = tmp.path().canonicalize().unwrap();
        let home_link = base.join("home-link");
        std::os::unix::fs::symlink(&home, &home_link).unwrap();
        let codex = base.join("codex-cfg");
        std::fs::create_dir_all(&codex).unwrap();
        let codex_link = base.join("codex-link");
        std::os::unix::fs::symlink(&codex, &codex_link).unwrap();
        let paths = denied_read_paths(&home_link, &discard_home, &root, &home_link.join(".claude"), std::slice::from_ref(&codex_link));
        assert!(paths.contains(&home.join(".ssh")), "{paths:#?}");
        assert!(paths.contains(&codex));
        let missing = base.join("missing-cfg");
        assert!(denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), std::slice::from_ref(&missing)).contains(&missing), "a missing path is kept as given");
    }

    #[test]
    fn read_rules_escape_glob_characters() {
        assert_eq!(escape_glob(Path::new("/a/b[1]/c*d?e{f}")), r"/a/b\[1\]/c\*d\?e\{f\}");
        let (tmp, _home, discard_home, root) = layout();
        let home = tmp.path().canonicalize().unwrap().join("home[x]");
        std::fs::create_dir_all(&home).unwrap();
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        let escaped = escape_glob(&home);
        assert!(denials.rules.contains(&format!("Read(/{escaped}/.ssh/**)")), "{:#?}", denials.rules);
        assert!(denials.paths.contains(&home.join(".ssh")), "sandbox paths stay literal");
    }

    fn symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn a_symlinked_credential_directory_denies_its_target() {
        let (tmp, home, discard_home, root) = layout();
        let base = tmp.path().canonicalize().unwrap();
        let target = base.join("dotfiles").join("ssh");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("id_ed25519"), "").unwrap();
        symlink(&target, &home.join(".ssh"));
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(denials.paths.contains(&home.join(".ssh")), "{:#?}", denials.paths);
        assert!(denials.paths.contains(&target), "{:#?}", denials.paths);
        assert!(denials.rules.contains(&format!("Read(/{}/**)", target.display())), "{:#?}", denials.rules);
    }

    #[test]
    fn a_symlinked_file_inside_a_credential_directory_denies_its_target() {
        let (tmp, home, discard_home, root) = layout();
        let base = tmp.path().canonicalize().unwrap();
        let target = base.join("keys").join("id_ed25519");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "key").unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        symlink(&target, &home.join(".ssh").join("id_ed25519"));
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(denials.paths.contains(&home.join(".ssh")));
        assert!(denials.paths.contains(&target), "{:#?}", denials.paths);
        assert!(denials.rules.contains(&format!("Read(/{})", target.display())), "{:#?}", denials.rules);
    }

    #[test]
    fn a_symlinked_credential_file_denies_its_target() {
        let (tmp, home, discard_home, root) = layout();
        let base = tmp.path().canonicalize().unwrap();
        let target = base.join("dotfiles").join("netrc");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "machine x").unwrap();
        symlink(&target, &home.join(".netrc"));
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(denials.paths.contains(&home.join(".netrc")));
        assert!(denials.paths.contains(&target), "{:#?}", denials.paths);
        assert!(denials.rules.contains(&format!("Read(/{})", target.display())), "{:#?}", denials.rules);
        assert!(!denials.rules.contains(&format!("Read(/{}/**)", target.display())));
    }

    #[test]
    fn a_credential_symlink_into_the_session_root_is_not_denied() {
        let (_tmp, home, discard_home, root) = layout();
        std::fs::create_dir_all(home.join(".gnupg")).unwrap();
        std::fs::create_dir_all(root.join("keys")).unwrap();
        symlink(&root, &home.join(".ssh"));
        symlink(&discard_home, &home.join(".aws"));
        symlink(&root.join("keys"), &home.join(".gnupg").join("inner"));
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(!paths.iter().any(|p| root.starts_with(p)), "{paths:#?}");
        assert!(paths.contains(&home.join(".ssh")), "the link itself stays denied");
    }

    #[test]
    fn dangling_credential_symlinks_are_ignored() {
        let (tmp, home, discard_home, root) = layout();
        let base = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        symlink(&base.join("missing-key"), &home.join(".ssh").join("id_rsa"));
        symlink(&base.join("missing-netrc"), &home.join(".netrc"));
        let paths = denied_read_paths(&home, &discard_home, &root, &home.join(".claude"), &[]);
        assert!(!paths.contains(&base.join("missing-key")), "{paths:#?}");
        assert!(!paths.contains(&base.join("missing-netrc")), "{paths:#?}");
        assert!(paths.contains(&home.join(".netrc")));
    }
}
