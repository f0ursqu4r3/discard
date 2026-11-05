//! What the host gives to plugins beyond the UI: scoped file access and agent memory. The host enforces every
//! scope. A plugin never gets a handle it could use outside its scope.

use crate::manifest::FsScope;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The most bytes one `fs.read` or `fs.write` moves.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The most entries one `fs.list` returns.
pub const MAX_LIST_ENTRIES: usize = 1000;
/// The most hits one `memory.search` returns.
pub const MAX_SEARCH_HITS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFile {
    pub path: String,
    pub version: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryText {
    pub text: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHit {
    pub scope: String,
    pub path: String,
    pub snippet: String,
}

/// Agent memory for plugins. `scope` is `agent-project`, `agent`, `project`, or `global`. The runtime has already checked it
/// against the plugin's manifest, and the implementation maps it to the ids of this app's agent and project.
pub trait MemoryHost: Send + Sync {
    fn list(&self, scope: &str) -> Result<Vec<MemoryFile>, String>;
    fn read(&self, scope: &str, path: &str) -> Result<MemoryText, String>;
    fn search(&self, scopes: &[String], query: &str, limit: usize) -> Result<Vec<MemoryHit>, String>;
    /// Returns the new version.
    fn append(&self, scope: &str, path: &str, text: &str) -> Result<String, String>;
    /// Returns the new version.
    fn edit(&self, scope: &str, path: &str, old: &str, new: &str, expected_version: &str) -> Result<String, String>;
}

/// The services a plugin manager hands to its plugins. The default grants nothing, so a manager that checks
/// an unreviewed draft cannot touch files or memory even if the draft asks for them.
#[derive(Clone, Default)]
pub struct Host {
    pub fs_enabled: bool,
    pub memory: Option<Arc<dyn MemoryHost>>,
}

impl Host {
    /// The host for plugins the user approved.
    pub fn live(memory: Option<Arc<dyn MemoryHost>>) -> Self {
        Self { fs_enabled: true, memory }
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").field("fs_enabled", &self.fs_enabled).field("memory", &self.memory.is_some()).finish()
    }
}

/// Check the paths in an `fs` scope. They must be absolute or start with `~/`, and may not contain `..`.
pub fn check_scope_path(path: &str) -> Result<(), String> {
    let rest = path.strip_prefix("~/").or(if path == "~" { Some("") } else { None });
    let absolute = rest.is_some() || path.starts_with('/');
    if !absolute {
        return Err(format!("`{path}` is not absolute: start it with `/` or `~/`"));
    }
    if Path::new(path).components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("`{path}` contains `..`"));
    }
    Ok(())
}

/// Scoped file access for one plugin.
#[derive(Debug, Clone)]
pub struct FsAccess {
    roots: Vec<PathBuf>,
    write: bool,
    home: Option<PathBuf>,
}

enum Resolved {
    /// The path exists (or can be created) at this canonical location, inside the scope.
    Found(PathBuf),
    /// The parent folder does not exist, so the path cannot exist.
    NoParent,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FsAccess {
    pub fn new(scope: &FsScope, home: Option<PathBuf>) -> Self {
        let mut access = Self { roots: Vec::new(), write: scope.write, home };
        access.roots = scope.paths.iter().filter_map(|p| access.expand(p).ok()).collect();
        access
    }

    fn expand(&self, raw: &str) -> Result<PathBuf, String> {
        check_scope_path(raw)?;
        if raw == "~" || raw.starts_with("~/") {
            let home = self.home.as_ref().ok_or("the home folder is not known")?;
            return Ok(home.join(raw.strip_prefix("~/").unwrap_or("")));
        }
        Ok(PathBuf::from(raw))
    }

    fn in_scope(&self, resolved: &Path) -> bool {
        // Roots are canonicalized on every call, so a root that appears later works and a link swapped in
        // later is seen. A root that does not exist grants nothing.
        self.roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).any(|root| resolved.starts_with(&root))
    }

    fn resolve(&self, raw: &str, needs_write: bool) -> Result<Resolved, String> {
        if needs_write && !self.write {
            return Err("this plugin has read-only file access".into());
        }
        let path = self.expand(raw)?;
        let resolved = match std::fs::canonicalize(&path) {
            Ok(real) => real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A link that points nowhere is not "missing". Refuse it, so a later write cannot follow it out.
                if std::fs::symlink_metadata(&path).is_ok() {
                    return Err(format!("`{raw}` is a link that points nowhere"));
                }
                let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                    return Err(format!("`{raw}` has no file name"));
                };
                match std::fs::canonicalize(parent) {
                    Ok(parent) => {
                        let target = parent.join(name);
                        if !self.in_scope(&target) {
                            return Err(format!("`{raw}` is outside the folders this plugin may use"));
                        }
                        return Ok(Resolved::Found(target));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Resolved::NoParent),
                    Err(e) => return Err(format!("cannot open `{raw}`: {e}")),
                }
            }
            Err(e) => return Err(format!("cannot open `{raw}`: {e}")),
        };
        if !self.in_scope(&resolved) {
            return Err(format!("`{raw}` is outside the folders this plugin may use"));
        }
        Ok(Resolved::Found(resolved))
    }

    fn existing(&self, raw: &str, needs_write: bool) -> Result<PathBuf, String> {
        match self.resolve(raw, needs_write)? {
            Resolved::Found(p) => Ok(p),
            Resolved::NoParent => Err(format!("`{raw}` does not exist")),
        }
    }

    pub fn read(&self, raw: &str) -> Result<String, String> {
        let path = self.existing(raw, false)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read `{raw}`: {e}"))?;
        if !meta.is_file() {
            return Err(format!("`{raw}` is not a file"));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("`{raw}` is larger than {MAX_FILE_BYTES} bytes"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read `{raw}`: {e}"))?;
        String::from_utf8(bytes).map_err(|_| format!("`{raw}` is not valid UTF-8 text"))
    }

    /// Replace the file in one step. A reader sees the old file or the new file, never half of one.
    pub fn write(&self, raw: &str, text: &str) -> Result<(), String> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("a write may hold at most {MAX_FILE_BYTES} bytes"));
        }
        let path = self.existing(raw, true)?;
        if path.is_dir() {
            return Err(format!("`{raw}` is a folder"));
        }
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(format!("`{raw}` has no file name"));
        };
        let temp = dir.join(format!(".{}.discard-tmp-{}", name.to_string_lossy(), TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            if let Ok(meta) = std::fs::metadata(&path) {
                let _ = std::fs::set_permissions(&temp, meta.permissions());
            }
            std::fs::rename(&temp, &path)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("cannot write `{raw}`: {e}"));
        }
        Ok(())
    }

    /// Entry names, sorted. A folder name ends in `/`.
    pub fn list(&self, raw: &str) -> Result<Vec<String>, String> {
        let path = self.existing(raw, false)?;
        let entries = std::fs::read_dir(&path).map_err(|e| format!("cannot list `{raw}`: {e}"))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot list `{raw}`: {e}"))?;
            let mut name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                name.push('/');
            }
            names.push(name);
            if names.len() > MAX_LIST_ENTRIES {
                return Err(format!("`{raw}` has more than {MAX_LIST_ENTRIES} entries"));
            }
        }
        names.sort();
        Ok(names)
    }

    /// False for a path that is missing. An error for a path outside the scope, so the answer never reveals
    /// what exists outside it.
    pub fn exists(&self, raw: &str) -> Result<bool, String> {
        match self.resolve(raw, false)? {
            Resolved::Found(p) => Ok(p.exists()),
            Resolved::NoParent => Ok(false),
        }
    }

    /// Create one folder. The parent must exist.
    pub fn mkdir(&self, raw: &str) -> Result<(), String> {
        match self.resolve(raw, true)? {
            Resolved::Found(p) if p.is_dir() => Ok(()),
            Resolved::Found(p) => std::fs::create_dir(&p).map_err(|e| format!("cannot create `{raw}`: {e}")),
            Resolved::NoParent => Err(format!("the parent folder of `{raw}` does not exist")),
        }
    }

    /// Delete one file. Folders are not deleted.
    pub fn remove(&self, raw: &str) -> Result<(), String> {
        let path = self.existing(raw, true)?;
        if path.is_dir() {
            return Err(format!("`{raw}` is a folder: only files can be removed"));
        }
        std::fs::remove_file(&path).map_err(|e| format!("cannot remove `{raw}`: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Rig {
        _dir: tempfile::TempDir,
        root: PathBuf,
        outside: PathBuf,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        Rig { _dir: dir, root, outside }
    }

    fn access(r: &Rig, write: bool) -> FsAccess {
        FsAccess::new(&FsScope { paths: vec![r.root.to_string_lossy().to_string()], write }, None)
    }

    fn p(r: &Rig, rel: &str) -> String {
        r.root.join(rel).to_string_lossy().to_string()
    }

    #[test]
    fn a_plugin_can_write_read_list_and_remove_inside_its_scope() {
        let r = rig();
        let a = access(&r, true);
        a.mkdir(&p(&r, "notes")).unwrap();
        a.write(&p(&r, "notes/a.txt"), "hello").unwrap();
        assert_eq!(a.read(&p(&r, "notes/a.txt")).unwrap(), "hello");
        a.write(&p(&r, "notes/a.txt"), "again").unwrap();
        assert_eq!(a.read(&p(&r, "notes/a.txt")).unwrap(), "again");
        a.write(&p(&r, "b.txt"), "b").unwrap();
        assert_eq!(a.list(&p(&r, "")).unwrap(), ["b.txt", "notes/"]);
        assert!(a.exists(&p(&r, "b.txt")).unwrap());
        a.remove(&p(&r, "b.txt")).unwrap();
        assert!(!a.exists(&p(&r, "b.txt")).unwrap());
    }

    #[test]
    fn read_only_access_refuses_every_change() {
        let r = rig();
        std::fs::write(r.root.join("f.txt"), "x").unwrap();
        let a = access(&r, false);
        assert_eq!(a.read(&p(&r, "f.txt")).unwrap(), "x");
        for result in [a.write(&p(&r, "f.txt"), "y"), a.mkdir(&p(&r, "d")), a.remove(&p(&r, "f.txt"))] {
            assert!(result.unwrap_err().contains("read-only"));
        }
        assert_eq!(std::fs::read_to_string(r.root.join("f.txt")).unwrap(), "x");
    }

    #[test]
    fn paths_outside_the_scope_are_refused() {
        let r = rig();
        let a = access(&r, true);
        let secret = r.outside.join("secret.txt").to_string_lossy().to_string();
        assert!(a.read(&secret).unwrap_err().contains("outside"));
        assert!(a.write(&r.outside.join("new.txt").to_string_lossy(), "x").unwrap_err().contains("outside"));
        assert!(a.exists(&secret).is_err(), "existence outside the scope must not be revealed");
        assert!(a.list(&r.outside.to_string_lossy()).is_err());
        assert!(!r.outside.join("new.txt").exists());
    }

    #[test]
    fn a_sibling_folder_with_the_same_prefix_is_outside_the_scope() {
        let r = rig();
        let sibling = r.root.parent().unwrap().join("root-evil");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("f.txt"), "x").unwrap();
        assert!(access(&r, true).read(&sibling.join("f.txt").to_string_lossy()).unwrap_err().contains("outside"));
    }

    #[test]
    fn dot_dot_and_relative_paths_are_refused() {
        let r = rig();
        let a = access(&r, true);
        assert!(a.read(&format!("{}/../outside/secret.txt", r.root.display())).unwrap_err().contains(".."));
        assert!(a.read("secret.txt").unwrap_err().contains("absolute"));
        assert!(a.read("").is_err());
    }

    #[test]
    fn a_link_that_leaves_the_scope_is_refused_for_read_write_list_and_remove() {
        let r = rig();
        symlink(r.outside.join("secret.txt"), r.root.join("link.txt")).unwrap();
        symlink(&r.outside, r.root.join("dirlink")).unwrap();
        let a = access(&r, true);
        assert!(a.read(&p(&r, "link.txt")).unwrap_err().contains("outside"));
        assert!(a.write(&p(&r, "link.txt"), "pwned").unwrap_err().contains("outside"));
        assert!(a.remove(&p(&r, "link.txt")).unwrap_err().contains("outside"));
        assert!(a.list(&p(&r, "dirlink")).unwrap_err().contains("outside"));
        assert!(a.write(&p(&r, "dirlink/new.txt"), "x").unwrap_err().contains("outside"));
        assert_eq!(std::fs::read_to_string(r.outside.join("secret.txt")).unwrap(), "secret");
        assert!(!r.outside.join("new.txt").exists());
    }

    #[test]
    fn a_link_inside_the_scope_that_stays_inside_works() {
        let r = rig();
        std::fs::write(r.root.join("real.txt"), "ok").unwrap();
        symlink(r.root.join("real.txt"), r.root.join("alias.txt")).unwrap();
        assert_eq!(access(&r, false).read(&p(&r, "alias.txt")).unwrap(), "ok");
    }

    #[test]
    fn a_dangling_link_cannot_be_written_through() {
        let r = rig();
        symlink(r.outside.join("not-yet.txt"), r.root.join("dangling")).unwrap();
        let a = access(&r, true);
        assert!(a.write(&p(&r, "dangling"), "x").unwrap_err().contains("points nowhere"));
        assert!(!r.outside.join("not-yet.txt").exists());
    }

    #[test]
    fn a_scope_root_that_is_itself_a_link_resolves_to_its_target() {
        let r = rig();
        let link = r.root.parent().unwrap().join("rootlink");
        symlink(&r.root, &link).unwrap();
        std::fs::write(r.root.join("f.txt"), "x").unwrap();
        let a = FsAccess::new(&FsScope { paths: vec![link.to_string_lossy().to_string()], write: false }, None);
        assert_eq!(a.read(&link.join("f.txt").to_string_lossy()).unwrap(), "x");
    }

    #[test]
    fn a_missing_root_grants_nothing_and_missing_paths_are_clear() {
        let r = rig();
        let gone = FsAccess::new(&FsScope { paths: vec![r.root.join("nope").to_string_lossy().to_string()], write: true }, None);
        assert!(gone.read(&p(&r, "nope/f.txt")).is_err());
        let a = access(&r, true);
        assert!(a.read(&p(&r, "missing.txt")).unwrap_err().contains("cannot read"));
        assert!(!a.exists(&p(&r, "missing/deeper.txt")).unwrap());
        assert!(a.write(&p(&r, "missing/deeper.txt"), "x").unwrap_err().contains("does not exist"));
    }

    #[test]
    fn the_home_shortcut_expands_in_scope_and_in_calls() {
        let r = rig();
        std::fs::write(r.root.join("f.txt"), "home").unwrap();
        let a = FsAccess::new(&FsScope { paths: vec!["~/".into()], write: false }, Some(r.root.clone()));
        assert_eq!(a.read("~/f.txt").unwrap(), "home");
        assert_eq!(a.read(&p(&r, "f.txt")).unwrap(), "home");
        let no_home = FsAccess::new(&FsScope { paths: vec!["~/".into()], write: false }, None);
        assert!(no_home.read("~/f.txt").is_err());
    }

    #[test]
    fn size_limits_apply_to_reads_and_writes() {
        let r = rig();
        let a = access(&r, true);
        let big = "x".repeat(MAX_FILE_BYTES as usize + 1);
        assert!(a.write(&p(&r, "big.txt"), &big).is_err());
        std::fs::write(r.root.join("big.txt"), &big).unwrap();
        assert!(a.read(&p(&r, "big.txt")).unwrap_err().contains("larger"));
        a.write(&p(&r, "ok.txt"), &"x".repeat(MAX_FILE_BYTES as usize)).unwrap();
    }

    #[test]
    fn binary_files_are_refused_as_text() {
        let r = rig();
        std::fs::write(r.root.join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(access(&r, false).read(&p(&r, "bin")).unwrap_err().contains("UTF-8"));
    }

    #[test]
    fn a_big_folder_is_refused_not_cut_short() {
        let r = rig();
        for i in 0..=MAX_LIST_ENTRIES {
            std::fs::write(r.root.join(format!("f{i}")), "").unwrap();
        }
        assert!(access(&r, false).list(&p(&r, "")).unwrap_err().contains("more than"));
    }

    #[test]
    fn writes_leave_no_temporary_files_and_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let r = rig();
        let a = access(&r, true);
        std::fs::write(r.root.join("run.sh"), "old").unwrap();
        std::fs::set_permissions(r.root.join("run.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
        a.write(&p(&r, "run.sh"), "new").unwrap();
        assert_eq!(std::fs::metadata(r.root.join("run.sh")).unwrap().permissions().mode() & 0o777, 0o750);
        let names: Vec<_> = std::fs::read_dir(&r.root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["run.sh"]);
    }

    #[test]
    fn only_files_can_be_removed_and_only_files_written() {
        let r = rig();
        let a = access(&r, true);
        a.mkdir(&p(&r, "d")).unwrap();
        assert!(a.remove(&p(&r, "d")).unwrap_err().contains("folder"));
        assert!(a.write(&p(&r, "d"), "x").unwrap_err().contains("folder"));
        a.mkdir(&p(&r, "d")).unwrap();
        assert!(a.mkdir(&p(&r, "x/y")).unwrap_err().contains("parent"));
    }

    #[test]
    fn scope_paths_must_be_absolute_without_dot_dot() {
        for ok in ["/tmp/x", "~/notes", "~"] {
            assert!(check_scope_path(ok).is_ok(), "{ok}");
        }
        for bad in ["notes", "./x", "../x", "/tmp/../etc", "~user/x", ""] {
            assert!(check_scope_path(bad).is_err(), "{bad}");
        }
    }
}
