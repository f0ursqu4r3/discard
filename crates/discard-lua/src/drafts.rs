//! Drafts: where an agent writes plugin code before the user reviews it. A draft never runs.

use crate::snapshot::{MAX_FILE_BYTES, MAX_FILES, Snapshot, SnapshotError};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DraftError {
    #[error("invalid plugin name `{0}`: use 1 to 64 characters from a-z, 0-9, and `-`")]
    BadName(String),
    #[error("invalid file path `{0}`: {1}")]
    BadPath(String, &'static str),
    #[error("draft `{0}` does not exist")]
    NotFound(String),
    #[error("draft `{0}` already exists")]
    AlreadyExists(String),
    #[error("file `{0}` does not exist in the draft")]
    NoSuchFile(String),
    #[error("the text to replace was not found in `{0}`")]
    TextNotFound(String),
    #[error("the text to replace appears {0} times; include more context")]
    Ambiguous(usize),
    #[error("a draft file may hold at most {MAX_FILE_BYTES} bytes")]
    TooLarge,
    #[error("a draft may hold at most {MAX_FILES} files")]
    TooManyFiles,
    #[error("`{0}` is a symbolic link; drafts may not contain links")]
    Symlink(String),
    #[error("storage error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, DraftError>;

/// Plugin names: lowercase letters, digits, `-`.
pub fn validate_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok { Ok(name) } else { Err(DraftError::BadName(name.to_string())) }
}

/// Draft file paths: relative, `.lua`, `.toml`, or `.md`, safe characters only, at most 4 levels deep.
pub fn validate_file_path(path: &str) -> Result<&str> {
    let bad = |why: &'static str| Err(DraftError::BadPath(path.to_string(), why));
    if path.is_empty() || path.len() > 120 {
        return bad("must be 1 to 120 characters");
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 4 {
        return bad("is nested too deeply");
    }
    for part in &parts {
        if part.is_empty() {
            return bad("has an empty segment");
        }
        if part.starts_with('.') {
            return bad("segments may not start with a dot");
        }
        if !part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return bad("use only a-z, 0-9, `.`, `_`, `-`, and `/`");
        }
    }
    if ![".lua", ".toml", ".md"].iter().any(|ext| path.ends_with(ext)) {
        return bad("must end in .lua, .toml, or .md");
    }
    Ok(path)
}

fn io_err(e: std::io::Error) -> DraftError {
    DraftError::Io(e.to_string())
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => count_files(&e.path()),
            Ok(t) if t.is_file() => 1,
            _ => 0,
        })
        .sum()
}

pub struct DraftStore {
    root: PathBuf,
}

impl DraftStore {
    pub fn new(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(io_err)?;
        Ok(Self { root: root.to_path_buf() })
    }

    fn dir(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self.root.join(name))
    }

    /// The draft directory, which must exist and must not be a link.
    fn existing_dir(&self, name: &str) -> Result<PathBuf> {
        let dir = self.dir(name)?;
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_symlink() => Err(DraftError::Symlink(name.to_string())),
            Ok(m) if m.is_dir() => Ok(dir),
            _ => Err(DraftError::NotFound(name.to_string())),
        }
    }

    /// A file path inside a draft. Refuses links at every existing component.
    fn resolve(&self, name: &str, path: &str) -> Result<PathBuf> {
        let dir = self.existing_dir(name)?;
        validate_file_path(path)?;
        let mut current = dir;
        for component in Path::new(path).components() {
            current.push(component);
            if let Ok(meta) = fs::symlink_metadata(&current)
                && meta.file_type().is_symlink()
            {
                return Err(DraftError::Symlink(path.to_string()));
            }
        }
        Ok(current)
    }

    fn write_all(&self, dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        if files.len() > MAX_FILES {
            return Err(DraftError::TooManyFiles);
        }
        for (path, body) in files {
            validate_file_path(path)?;
            if body.len() as u64 > MAX_FILE_BYTES {
                return Err(DraftError::TooLarge);
            }
        }
        for (path, body) in files {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io_err)?;
            }
            fs::write(&target, body).map_err(io_err)?;
        }
        Ok(())
    }

    /// Create a draft from files. Fails if the draft already exists.
    pub fn create(&self, name: &str, files: &BTreeMap<String, String>) -> Result<()> {
        let dir = self.dir(name)?;
        if fs::symlink_metadata(&dir).is_ok() {
            return Err(DraftError::AlreadyExists(name.to_string()));
        }
        let bytes: BTreeMap<String, Vec<u8>> = files.iter().map(|(k, v)| (k.clone(), v.clone().into_bytes())).collect();
        fs::create_dir_all(&dir).map_err(io_err)?;
        if let Err(e) = self.write_all(&dir, &bytes) {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
        Ok(())
    }

    /// Start a draft from the bytes of an installed plugin. Replaces an existing draft of that name.
    pub fn fork(&self, name: &str, from: &Snapshot) -> Result<()> {
        let dir = self.dir(name)?;
        if fs::symlink_metadata(&dir).is_ok() {
            fs::remove_dir_all(&dir).map_err(io_err)?;
        }
        fs::create_dir_all(&dir).map_err(io_err)?;
        if let Err(e) = self.write_all(&dir, from.files()) {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
        Ok(())
    }

    pub fn write_file(&self, name: &str, path: &str, content: &str) -> Result<()> {
        let file = self.resolve(name, path)?;
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err(DraftError::TooLarge);
        }
        if !file.exists() && count_files(&self.existing_dir(name)?) >= MAX_FILES {
            return Err(DraftError::TooManyFiles);
        }
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        fs::write(&file, content).map_err(io_err)
    }

    pub fn edit_file(&self, name: &str, path: &str, old: &str, new: &str) -> Result<()> {
        let current = self.read_file(name, path)?;
        let count = if old.is_empty() { 0 } else { current.matches(old).count() };
        match count {
            0 => Err(DraftError::TextNotFound(path.to_string())),
            1 => self.write_file(name, path, &current.replacen(old, new, 1)),
            n => Err(DraftError::Ambiguous(n)),
        }
    }

    pub fn delete_file(&self, name: &str, path: &str) -> Result<()> {
        let file = self.resolve(name, path)?;
        fs::remove_file(&file).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { DraftError::NoSuchFile(path.to_string()) } else { io_err(e) })
    }

    pub fn read_file(&self, name: &str, path: &str) -> Result<String> {
        let file = self.resolve(name, path)?;
        fs::read_to_string(&file).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { DraftError::NoSuchFile(path.to_string()) } else { io_err(e) })
    }

    /// Copy the draft into memory. Review and install use this copy, never the directory.
    pub fn snapshot(&self, name: &str) -> Result<Snapshot> {
        let dir = self.existing_dir(name)?;
        Snapshot::capture(&dir).map_err(|e| match e {
            SnapshotError::Symlink(p) => DraftError::Symlink(p),
            SnapshotError::FileTooLarge(_) => DraftError::TooLarge,
            SnapshotError::TooManyFiles => DraftError::TooManyFiles,
            other => DraftError::Io(other.to_string()),
        })
    }

    pub fn list(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(&self.root)
            .map_err(io_err)?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| validate_name(n).is_ok())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn discard(&self, name: &str) -> Result<()> {
        let dir = self.existing_dir(name)?;
        fs::remove_dir_all(dir).map_err(io_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (DraftStore, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (DraftStore::new(&d.path().join("drafts")).unwrap(), d)
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    const TOML: &str = "name = \"p\"\nversion = \"1\"\napi = \"1\"\n";

    #[test]
    fn plugin_names_are_restricted() {
        for ok in ["a", "my-plugin", "x1", &"a".repeat(64)] {
            assert_eq!(validate_name(ok), Ok(ok));
        }
        for bad in ["", "A", "a b", "a/b", "..", ".", "-a", "a_b", &"a".repeat(65), "é"] {
            assert_eq!(validate_name(bad), Err(DraftError::BadName(bad.into())), "{bad:?}");
        }
    }

    #[test]
    fn file_paths_are_restricted() {
        for ok in ["init.lua", "plugin.toml", "lib/util.lua", "docs/notes.md", "a/b/c/d.lua"] {
            assert_eq!(validate_file_path(ok), Ok(ok));
        }
        for bad in ["", "/etc/x.lua", "../x.lua", "a/../b.lua", "a//b.lua", ".hidden.lua", "a/.x.lua", "x.sh", "x", "a\\b.lua", "a\0b.lua", "a/b/c/d/e.lua", "a b.lua", "x.lua/", "é.lua"] {
            assert!(validate_file_path(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn create_writes_the_files_and_snapshot_reads_them_back() {
        let (s, _d) = store();
        s.create("p", &files(&[("plugin.toml", TOML), ("init.lua", "return 1"), ("lib/u.lua", "x")])).unwrap();
        let snap = s.snapshot("p").unwrap();
        assert_eq!(snap.get("init.lua"), Some(&b"return 1"[..]));
        assert_eq!(snap.files().keys().cloned().collect::<Vec<_>>(), ["init.lua", "lib/u.lua", "plugin.toml"]);
        assert_eq!(s.list().unwrap(), ["p"]);
    }

    #[test]
    fn create_refuses_an_existing_draft_and_bad_input() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "x")])).unwrap();
        assert_eq!(s.create("p", &files(&[("init.lua", "y")])), Err(DraftError::AlreadyExists("p".into())));
        assert!(matches!(s.create("Bad Name", &files(&[("init.lua", "x")])), Err(DraftError::BadName(_))));
        assert!(matches!(s.create("q", &files(&[("../evil.lua", "x")])), Err(DraftError::BadPath(..))));
        assert!(s.list().unwrap() == ["p"], "a rejected create must leave nothing behind");
    }

    #[test]
    fn a_create_with_any_bad_file_creates_nothing() {
        let (s, d) = store();
        let r = s.create("q", &files(&[("init.lua", "ok"), ("sub/../../out.lua", "bad")]));
        assert!(r.is_err());
        assert!(!d.path().join("drafts/q").exists());
        assert!(!d.path().join("out.lua").exists());
    }

    #[test]
    fn write_and_edit_and_delete_change_files() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "alpha beta")])).unwrap();
        s.write_file("p", "lib/new.lua", "n").unwrap();
        s.edit_file("p", "init.lua", "beta", "gamma").unwrap();
        assert_eq!(s.read_file("p", "init.lua").unwrap(), "alpha gamma");
        assert_eq!(s.read_file("p", "lib/new.lua").unwrap(), "n");
        s.delete_file("p", "lib/new.lua").unwrap();
        assert_eq!(s.read_file("p", "lib/new.lua"), Err(DraftError::NoSuchFile("lib/new.lua".into())));
    }

    #[test]
    fn edit_reports_missing_and_ambiguous_text() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "dup dup")])).unwrap();
        assert_eq!(s.edit_file("p", "init.lua", "zzz", "y"), Err(DraftError::TextNotFound("init.lua".into())));
        assert_eq!(s.edit_file("p", "init.lua", "dup", "y"), Err(DraftError::Ambiguous(2)));
        assert_eq!(s.edit_file("p", "init.lua", "", "y"), Err(DraftError::TextNotFound("init.lua".into())));
        assert_eq!(s.read_file("p", "init.lua").unwrap(), "dup dup");
    }

    #[test]
    fn operations_on_a_missing_draft_fail_cleanly() {
        let (s, _d) = store();
        assert_eq!(s.snapshot("nope").unwrap_err(), DraftError::NotFound("nope".into()));
        assert_eq!(s.write_file("nope", "init.lua", "x"), Err(DraftError::NotFound("nope".into())));
        assert_eq!(s.discard("nope"), Err(DraftError::NotFound("nope".into())));
    }

    #[test]
    fn size_and_count_limits_apply() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "x")])).unwrap();
        assert_eq!(s.write_file("p", "big.lua", &"x".repeat(MAX_FILE_BYTES as usize + 1)), Err(DraftError::TooLarge));
        for i in 0..MAX_FILES - 1 {
            s.write_file("p", &format!("f{i}.lua"), "x").unwrap();
        }
        assert_eq!(s.write_file("p", "one-too-many.lua", "x"), Err(DraftError::TooManyFiles));
        s.write_file("p", "f0.lua", "overwrite is fine").unwrap();
    }

    #[test]
    fn symlinks_in_a_draft_are_rejected_and_never_followed() {
        let (s, d) = store();
        s.create("p", &files(&[("init.lua", "x")])).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.lua"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("drafts/p/link")).unwrap();
        assert!(matches!(s.read_file("p", "link/secret.lua"), Err(DraftError::Symlink(_))));
        assert!(matches!(s.write_file("p", "link/new.lua", "x"), Err(DraftError::Symlink(_))));
        assert!(!outside.path().join("new.lua").exists());
        assert!(matches!(s.snapshot("p"), Err(DraftError::Symlink(_))), "a snapshot must refuse links too");
    }

    #[test]
    fn fork_copies_installed_bytes_into_a_fresh_draft() {
        let (s, _d) = store();
        let live = Snapshot::from_files([("plugin.toml".to_string(), TOML.as_bytes().to_vec()), ("init.lua".to_string(), b"live".to_vec())].into());
        s.fork("p", &live).unwrap();
        assert_eq!(s.snapshot("p").unwrap(), live);
        s.write_file("p", "init.lua", "edited").unwrap();
        s.fork("p", &live).unwrap();
        assert_eq!(s.read_file("p", "init.lua").unwrap(), "live", "fork resets the draft");
    }

    #[test]
    fn discard_removes_the_draft() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "x")])).unwrap();
        s.discard("p").unwrap();
        assert!(s.list().unwrap().is_empty());
    }

    #[test]
    fn a_snapshot_is_unaffected_by_later_draft_edits() {
        let (s, _d) = store();
        s.create("p", &files(&[("init.lua", "v1")])).unwrap();
        let snap = s.snapshot("p").unwrap();
        s.write_file("p", "init.lua", "v2").unwrap();
        assert_eq!(snap.get("init.lua"), Some(&b"v1"[..]));
        assert_ne!(snap.hash(), s.snapshot("p").unwrap().hash());
    }
}
