//! An immutable, in-memory copy of a plugin directory, with a content hash.
//! Plugins load from a snapshot, never from the live directory.

use crate::manifest::{Manifest, ManifestError};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const MAX_FILES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("io error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("`{0}` is a symbolic link; plugins may not contain links")]
    Symlink(String),
    #[error("plugin has more than {MAX_FILES} files")]
    TooManyFiles,
    #[error("`{0}` is larger than {MAX_FILE_BYTES} bytes")]
    FileTooLarge(String),
    #[error("plugin is larger than {MAX_TOTAL_BYTES} bytes in total")]
    TooLarge,
    #[error("missing required file `{0}`")]
    MissingFile(&'static str),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    files: BTreeMap<String, Vec<u8>>,
}

impl Snapshot {
    /// Capture a directory. Reads every file once into memory.
    pub fn capture(dir: &Path) -> Result<Snapshot, SnapshotError> {
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        walk(dir, "", &mut files, &mut total)?;
        Ok(Snapshot { files })
    }

    /// Build a snapshot from in-memory files (used by tests and by drafts).
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Snapshot {
        Snapshot { files }
    }

    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// SHA-256 over the sorted paths and contents, as lowercase hex. Changes if any byte changes.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        for (path, body) in &self.files {
            // Length prefixes keep file boundaries unambiguous.
            h.update((path.len() as u64).to_le_bytes());
            h.update(path.as_bytes());
            h.update((body.len() as u64).to_le_bytes());
            h.update(body);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn manifest(&self) -> Result<Manifest, SnapshotError> {
        let bytes = self.get("plugin.toml").ok_or(SnapshotError::MissingFile("plugin.toml"))?;
        let text = std::str::from_utf8(bytes).map_err(|e| ManifestError::Parse(e.to_string()))?;
        Ok(Manifest::parse(text)?)
    }

    pub fn entry_source(&self) -> Result<&str, SnapshotError> {
        let bytes = self.get("init.lua").ok_or(SnapshotError::MissingFile("init.lua"))?;
        std::str::from_utf8(bytes).map_err(|_| SnapshotError::MissingFile("init.lua"))
    }
}

fn io_err(path: &Path, e: std::io::Error) -> SnapshotError {
    SnapshotError::Io { path: path.display().to_string(), message: e.to_string() }
}

fn walk(
    dir: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut u64,
) -> Result<(), SnapshotError> {
    let mut entries: Vec<_> = fs::read_dir(dir).map_err(|e| io_err(dir, e))?.collect::<Result<_, _>>().map_err(|e| io_err(dir, e))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let path = entry.path();
        // `symlink_metadata` does not follow links, so a link is seen as a link.
        let meta = fs::symlink_metadata(&path).map_err(|e| io_err(&path, e))?;
        if meta.file_type().is_symlink() {
            return Err(SnapshotError::Symlink(rel));
        }
        if meta.is_dir() {
            walk(&path, &rel, files, total)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(SnapshotError::FileTooLarge(rel));
        }
        if files.len() >= MAX_FILES {
            return Err(SnapshotError::TooManyFiles);
        }
        *total += meta.len();
        if *total > MAX_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        files.insert(rel, fs::read(&path).map_err(|e| io_err(&path, e))?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    const MANIFEST: &str = "name = \"p\"\nversion = \"1\"\napi = \"1\"\n";

    fn plugin_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "plugin.toml", MANIFEST);
        write(d.path(), "init.lua", "return 1");
        d
    }

    #[test]
    fn captures_files_recursively_with_relative_paths() {
        let d = plugin_dir();
        write(d.path(), "lib/util.lua", "x");
        let s = Snapshot::capture(d.path()).unwrap();
        let paths: Vec<_> = s.files().keys().cloned().collect();
        assert_eq!(paths, ["init.lua", "lib/util.lua", "plugin.toml"]);
        assert_eq!(s.get("lib/util.lua"), Some(&b"x"[..]));
    }

    #[test]
    fn the_hash_is_stable_and_changes_with_content_and_paths() {
        let d = plugin_dir();
        let a = Snapshot::capture(d.path()).unwrap();
        assert_eq!(a.hash(), Snapshot::capture(d.path()).unwrap().hash());
        assert_eq!(a.hash().len(), 64);
        write(d.path(), "init.lua", "return 2");
        assert_ne!(a.hash(), Snapshot::capture(d.path()).unwrap().hash());
    }

    #[test]
    fn the_hash_does_not_confuse_file_boundaries() {
        let one = Snapshot::from_files([("a".to_string(), b"bc".to_vec())].into());
        let two = Snapshot::from_files([("ab".to_string(), b"c".to_vec())].into());
        assert_ne!(one.hash(), two.hash());
    }

    #[test]
    fn symlinks_are_rejected() {
        let d = plugin_dir();
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("evil.lua")).unwrap();
        assert!(matches!(Snapshot::capture(d.path()), Err(SnapshotError::Symlink(p)) if p == "evil.lua"));
    }

    #[test]
    fn symlinked_directories_are_rejected_too() {
        let d = plugin_dir();
        std::os::unix::fs::symlink("/etc", d.path().join("lib")).unwrap();
        assert!(matches!(Snapshot::capture(d.path()), Err(SnapshotError::Symlink(_))));
    }

    #[test]
    fn size_and_count_limits_apply() {
        let d = plugin_dir();
        fs::write(d.path().join("big.lua"), vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(Snapshot::capture(d.path()), Err(SnapshotError::FileTooLarge(_))));
        let d = plugin_dir();
        for i in 0..MAX_FILES {
            write(d.path(), &format!("f{i}.lua"), "x");
        }
        assert!(matches!(Snapshot::capture(d.path()), Err(SnapshotError::TooManyFiles)));
    }

    #[test]
    fn manifest_and_entry_come_from_the_snapshot() {
        let s = Snapshot::capture(plugin_dir().path()).unwrap();
        assert_eq!(s.manifest().unwrap().name, "p");
        assert_eq!(s.entry_source().unwrap(), "return 1");
    }

    #[test]
    fn missing_manifest_or_entry_is_an_error() {
        let s = Snapshot::from_files([("init.lua".to_string(), b"x".to_vec())].into());
        assert!(matches!(s.manifest(), Err(SnapshotError::MissingFile("plugin.toml"))));
        let s = Snapshot::from_files([("plugin.toml".to_string(), MANIFEST.as_bytes().to_vec())].into());
        assert!(matches!(s.entry_source(), Err(SnapshotError::MissingFile("init.lua"))));
    }

    #[test]
    fn a_snapshot_is_unaffected_by_later_disk_changes() {
        let d = plugin_dir();
        let s = Snapshot::capture(d.path()).unwrap();
        write(d.path(), "init.lua", "evil()");
        assert_eq!(s.entry_source().unwrap(), "return 1");
    }
}
