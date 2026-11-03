//! The plugin repository: `plugins/` is a git repository with one commit per approved change.
//! Installing swaps a plugin's directory atomically and verifies the installed bytes match what was reviewed.

use crate::drafts::validate_name;
use crate::snapshot::Snapshot;
use similar::TextDiff;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("git failed: {0}")]
    Git(String),
    #[error("storage error: {0}")]
    Io(String),
    #[error("the installed files do not match the reviewed files")]
    VerifyFailed,
    #[error("plugin `{0}` is not installed")]
    NotInstalled(String),
    #[error("invalid plugin name `{0}`")]
    BadName(String),
    #[error(transparent)]
    Snapshot(#[from] crate::snapshot::SnapshotError),
}

pub type Result<T> = std::result::Result<T, RepoError>;

pub struct PluginRepo {
    root: PathBuf,
}

fn io_err(e: std::io::Error) -> RepoError {
    RepoError::Io(e.to_string())
}

fn check_name(name: &str) -> Result<&str> {
    validate_name(name).map_err(|_| RepoError::BadName(name.to_string()))
}

fn is_commit_id(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

impl PluginRepo {
    /// Open the repository at `root`, creating and initializing it if needed.
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(io_err)?;
        let repo = Self { root: root.to_path_buf() };
        if !root.join(".git").exists() {
            repo.git(&["init", "-q"])?;
        }
        // A private identity, and no signing: plugin history must not depend on the user's git setup.
        for (key, value) in [("user.name", "Discard"), ("user.email", "discard@localhost"), ("commit.gpgsign", "false")] {
            repo.git(&["config", key, value])?;
        }
        repo.recover()?;
        Ok(repo)
    }

    /// Finish or undo an install that was interrupted by a crash.
    fn recover(&self) -> Result<()> {
        for entry in fs::read_dir(&self.root).map_err(io_err)?.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name.starts_with(".tmp-") {
                let _ = fs::remove_dir_all(entry.path());
            } else if let Some(rest) = file_name.strip_prefix(".old-") {
                // `.old-<name>-<uuid>`: put the old version back if the new one never arrived.
                let name = rest.rsplit_once('-').map(|(n, _)| n).unwrap_or(rest);
                if self.root.join(name).exists() {
                    let _ = fs::remove_dir_all(entry.path());
                } else {
                    let _ = fs::rename(entry.path(), self.root.join(name));
                }
            }
        }
        Ok(())
    }

    fn git_output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
            .args(args)
            .current_dir(&self.root)
            // The user's global and system git config must not change how plugin history is written.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|e| RepoError::Git(format!("could not run git: {e}")))?;
        if !out.status.success() {
            return Err(RepoError::Git(String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        Ok(out.stdout)
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.git_output(args)?).into_owned())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Names of installed plugins, sorted.
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

    /// The installed files of a plugin, copied into memory.
    pub fn capture(&self, name: &str) -> Result<Snapshot> {
        let dir = self.root.join(check_name(name)?);
        if !dir.is_dir() {
            return Err(RepoError::NotInstalled(name.to_string()));
        }
        Ok(Snapshot::capture(&dir)?)
    }

    /// Replace (or add) a plugin with the reviewed bytes. Returns the commit ID.
    /// The swap is atomic. If the installed bytes do not hash to `snapshot.hash()`, the change is undone.
    pub fn install(&self, name: &str, snapshot: &Snapshot, message: &str) -> Result<String> {
        check_name(name)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let staging = self.root.join(format!(".tmp-{nonce}"));
        let backup = self.root.join(format!(".old-{name}-{nonce}"));
        let live = self.root.join(name);

        let stage = || -> std::io::Result<()> {
            for (path, body) in snapshot.files() {
                let target = staging.join(path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, body)?;
            }
            if snapshot.files().is_empty() {
                fs::create_dir_all(&staging)?;
            }
            Ok(())
        };
        if let Err(e) = stage() {
            let _ = fs::remove_dir_all(&staging);
            return Err(io_err(e));
        }

        let had_old = live.exists();
        if had_old && let Err(e) = fs::rename(&live, &backup) {
            let _ = fs::remove_dir_all(&staging);
            return Err(io_err(e));
        }
        if let Err(e) = fs::rename(&staging, &live) {
            if had_old {
                let _ = fs::rename(&backup, &live);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(io_err(e));
        }

        // The installed bytes must be exactly the reviewed bytes.
        let verified = Snapshot::capture(&live).map(|s| s.hash() == snapshot.hash()).unwrap_or(false);
        if !verified {
            let _ = fs::remove_dir_all(&live);
            if had_old {
                let _ = fs::rename(&backup, &live);
            }
            return Err(RepoError::VerifyFailed);
        }

        let committed = self
            .git(&["add", "-A", "--", name])
            .and_then(|_| self.git(&["commit", "-q", "--allow-empty", "-m", message]))
            .and_then(|_| self.git(&["rev-parse", "HEAD"]));
        match committed {
            Ok(commit) => {
                if had_old {
                    let _ = fs::remove_dir_all(&backup);
                }
                Ok(commit.trim().to_string())
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&live);
                if had_old {
                    let _ = fs::rename(&backup, &live);
                }
                Err(e)
            }
        }
    }

    /// The files a plugin had at `commit`.
    pub fn snapshot_at(&self, name: &str, commit: &str) -> Result<Snapshot> {
        check_name(name)?;
        if !is_commit_id(commit) {
            return Err(RepoError::Git(format!("`{commit}` is not a commit id")));
        }
        let prefix = format!("{name}/");
        let listing = self.git_output(&["ls-tree", "-r", "--name-only", "-z", commit, "--", &prefix])?;
        let mut files = BTreeMap::new();
        for path in listing.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8_lossy(path).into_owned();
            let relative = path.strip_prefix(&prefix).unwrap_or(&path).to_string();
            let body = self.git_output(&["show", &format!("{commit}:{path}")])?;
            files.insert(relative, body);
        }
        if files.is_empty() {
            return Err(RepoError::NotInstalled(name.to_string()));
        }
        Ok(Snapshot::from_files(files))
    }

    /// Remove a plugin and commit the removal.
    pub fn remove(&self, name: &str, message: &str) -> Result<()> {
        let live = self.root.join(check_name(name)?);
        if !live.is_dir() {
            return Err(RepoError::NotInstalled(name.to_string()));
        }
        fs::remove_dir_all(&live).map_err(io_err)?;
        self.git(&["add", "-A", "--", name])?;
        self.git(&["commit", "-q", "--allow-empty", "-m", message])?;
        Ok(())
    }

    /// (commit, message), newest first, for one plugin.
    pub fn log(&self, name: &str) -> Result<Vec<(String, String)>> {
        check_name(name)?;
        let out = self.git(&["log", "--format=%H%x1f%s", "--", name])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\u{1f}'))
            .map(|(c, m)| (c.to_string(), m.to_string()))
            .collect())
    }
}

/// A unified diff between two versions of a plugin. `old` is `None` for a new plugin.
pub fn diff(old: Option<&Snapshot>, new: &Snapshot) -> String {
    let empty = BTreeMap::new();
    let old_files = old.map(Snapshot::files).unwrap_or(&empty);
    let mut paths: Vec<&String> = old_files.keys().chain(new.files().keys()).collect();
    paths.sort();
    paths.dedup();
    let mut out = String::new();
    for path in paths {
        let (before, after) = (old_files.get(path), new.files().get(path));
        if before == after {
            continue;
        }
        let looks_binary = |b: &Option<&Vec<u8>>| b.is_some_and(|b| b.contains(&0) || std::str::from_utf8(b).is_err());
        if looks_binary(&before) || looks_binary(&after) {
            out.push_str(&format!("Binary file {path} changed\n"));
            continue;
        }
        let text = |b: Option<&Vec<u8>>| b.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        let (old_name, new_name) = match (before, after) {
            (None, _) => ("/dev/null", path.as_str()),
            (_, None) => (path.as_str(), "/dev/null"),
            _ => (path.as_str(), path.as_str()),
        };
        let (a, b) = (text(before), text(after));
        let diff = TextDiff::from_lines(&a, &b);
        out.push_str(&diff.unified_diff().context_radius(3).header(old_name, new_name).to_string());
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(files: &[(&str, &str)]) -> Snapshot {
        Snapshot::from_files(files.iter().map(|(k, v)| (k.to_string(), v.as_bytes().to_vec())).collect())
    }

    fn repo() -> (PluginRepo, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (PluginRepo::open(&d.path().join("plugins")).unwrap(), d)
    }

    fn git(d: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").args(args).current_dir(d).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn open_creates_a_git_repository_with_a_private_identity() {
        let (r, _d) = repo();
        assert!(r.root().join(".git").is_dir());
        assert_eq!(git(r.root(), &["config", "user.name"]).trim(), "Discard");
        assert_eq!(git(r.root(), &["config", "commit.gpgsign"]).trim(), "false");
        PluginRepo::open(r.root()).unwrap();
    }

    #[test]
    fn install_writes_files_commits_them_and_returns_the_commit() {
        let (r, _d) = repo();
        let s = snap(&[("plugin.toml", "t"), ("init.lua", "one"), ("lib/u.lua", "u")]);
        let commit = r.install("p", &s, "install p").unwrap();
        assert_eq!(commit.len(), 40);
        assert_eq!(r.capture("p").unwrap(), s);
        assert_eq!(r.list().unwrap(), ["p"]);
        assert_eq!(git(r.root(), &["rev-parse", "HEAD"]).trim(), commit);
        assert!(git(r.root(), &["status", "--porcelain"]).trim().is_empty(), "the tree must be clean after an install");
    }

    #[test]
    fn an_update_replaces_the_directory_and_removes_files_that_were_dropped() {
        let (r, _d) = repo();
        r.install("p", &snap(&[("init.lua", "one"), ("old.lua", "old")]), "v1").unwrap();
        r.install("p", &snap(&[("init.lua", "two")]), "v2").unwrap();
        assert_eq!(r.capture("p").unwrap(), snap(&[("init.lua", "two")]));
        assert_eq!(r.log("p").unwrap().len(), 2);
    }

    #[test]
    fn snapshot_at_returns_an_earlier_version() {
        let (r, _d) = repo();
        let c1 = r.install("p", &snap(&[("init.lua", "one")]), "v1").unwrap();
        r.install("p", &snap(&[("init.lua", "two"), ("extra.lua", "e")]), "v2").unwrap();
        assert_eq!(r.snapshot_at("p", &c1).unwrap(), snap(&[("init.lua", "one")]));
    }

    #[test]
    fn plugins_are_independent_and_names_are_validated() {
        let (r, _d) = repo();
        r.install("a", &snap(&[("init.lua", "a")]), "a").unwrap();
        r.install("b", &snap(&[("init.lua", "b")]), "b").unwrap();
        assert_eq!(r.list().unwrap(), ["a", "b"]);
        for bad in ["", "../x", "a/b", ".git", "A"] {
            assert!(matches!(r.install(bad, &snap(&[("init.lua", "x")]), "m"), Err(RepoError::BadName(_))), "{bad:?}");
        }
        assert!(!r.root().join("x").exists());
    }

    #[test]
    fn remove_deletes_the_plugin_and_commits() {
        let (r, _d) = repo();
        r.install("p", &snap(&[("init.lua", "x")]), "v1").unwrap();
        r.remove("p", "remove p").unwrap();
        assert!(r.list().unwrap().is_empty());
        assert!(matches!(r.capture("p"), Err(RepoError::NotInstalled(_))));
        assert!(git(r.root(), &["status", "--porcelain"]).trim().is_empty());
        assert!(matches!(r.remove("p", "again"), Err(RepoError::NotInstalled(_))));
    }

    #[test]
    fn installing_identical_bytes_again_still_succeeds() {
        let (r, _d) = repo();
        let s = snap(&[("init.lua", "same")]);
        r.install("p", &s, "v1").unwrap();
        let c2 = r.install("p", &s, "again").unwrap();
        assert_eq!(c2.len(), 40);
        assert_eq!(r.capture("p").unwrap(), s);
    }

    #[test]
    fn user_git_hooks_and_config_do_not_run() {
        let (r, d) = repo();
        let hooks = d.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let marker = d.path().join("hook-ran");
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        git(r.root(), &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        r.install("p", &snap(&[("init.lua", "x")]), "v1").unwrap();
        assert!(!marker.exists(), "a pre-commit hook must not run for plugin commits");
    }

    #[test]
    fn diff_of_a_new_plugin_shows_every_file_as_added() {
        let d = diff(None, &snap(&[("init.lua", "line1\nline2\n"), ("plugin.toml", "name = \"p\"\n")]));
        assert!(d.contains("+++ init.lua") && d.contains("+line1") && d.contains("+line2"), "{d}");
        assert!(d.contains("+++ plugin.toml"));
        assert!(!d.contains("\n-line"), "nothing is removed in a new plugin: {d}");
    }

    #[test]
    fn diff_of_an_update_shows_changes_additions_and_deletions() {
        let old = snap(&[("init.lua", "keep\nold\n"), ("gone.lua", "bye\n")]);
        let new = snap(&[("init.lua", "keep\nnew\n"), ("added.lua", "hi\n")]);
        let d = diff(Some(&old), &new);
        assert!(d.contains("-old") && d.contains("+new"), "{d}");
        assert!(d.contains("+++ added.lua") && d.contains("+hi"), "{d}");
        assert!(d.contains("--- gone.lua") && d.contains("-bye"), "{d}");
        assert!(!d.contains("+keep") && !d.contains("-keep"), "unchanged lines are context only: {d}");
    }

    #[test]
    fn diff_of_identical_versions_is_empty() {
        let s = snap(&[("init.lua", "same\n")]);
        assert!(diff(Some(&s), &s).trim().is_empty());
    }

    #[test]
    fn diff_handles_binary_looking_content_without_panicking() {
        let bin = Snapshot::from_files([("data.lua".to_string(), vec![0xff, 0xfe, 0x00, 0x01])].into());
        assert!(diff(None, &bin).contains("data.lua"));
    }
}
