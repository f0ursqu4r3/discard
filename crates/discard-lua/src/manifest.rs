//! `plugin.toml`: what a plugin is, and which capabilities it asks for.

use serde::{Deserialize, Serialize};

/// The plugin API version this build of Discard implements.
pub const API_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub ui: bool,
    #[serde(default)]
    pub approvals_decide: bool,
    #[serde(default)]
    pub fs: Option<FsScope>,
    #[serde(default)]
    pub net: Option<NetScope>,
    #[serde(default)]
    pub exec: Option<ExecScope>,
    #[serde(default)]
    pub memory: Option<MemoryScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsScope {
    pub paths: Vec<String>,
    #[serde(default)]
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetScope {
    /// `host` or `host:port`.
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecScope {
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryScope {
    /// Any of `agent-project`, `agent`, `project`, `global`.
    pub scopes: Vec<String>,
    /// Without this, the plugin can list, read, and search, but not append or edit.
    #[serde(default)]
    pub write: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("invalid plugin.toml: {0}")]
    Parse(String),
    #[error("invalid plugin name `{0}`: use 1 to 64 characters from a-z, 0-9, and `-`")]
    BadName(String),
    #[error("plugin needs API version {wanted}, but this Discard implements {have}")]
    ApiMismatch { wanted: String, have: &'static str },
    #[error("unknown memory scope `{0}`")]
    BadMemoryScope(String),
    #[error("bad fs path: {0}")]
    BadFsPath(String),
    #[error("capability `{0}` has an empty scope list")]
    EmptyScope(&'static str),
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let m: Manifest = toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        let valid_name = !m.name.is_empty()
            && m.name.len() <= 64
            && m.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !m.name.starts_with('-');
        if !valid_name {
            return Err(ManifestError::BadName(m.name));
        }
        if m.api != API_VERSION {
            return Err(ManifestError::ApiMismatch { wanted: m.api, have: API_VERSION });
        }
        let c = &m.capabilities;
        if c.fs.as_ref().is_some_and(|f| f.paths.is_empty()) {
            return Err(ManifestError::EmptyScope("fs"));
        }
        if let Some(fs) = &c.fs {
            for path in &fs.paths {
                crate::host::check_scope_path(path).map_err(ManifestError::BadFsPath)?;
            }
        }
        if c.net.as_ref().is_some_and(|n| n.hosts.is_empty()) {
            return Err(ManifestError::EmptyScope("net"));
        }
        if c.exec.as_ref().is_some_and(|e| e.commands.is_empty()) {
            return Err(ManifestError::EmptyScope("exec"));
        }
        if let Some(mem) = &c.memory {
            if mem.scopes.is_empty() {
                return Err(ManifestError::EmptyScope("memory"));
            }
            if let Some(bad) = mem.scopes.iter().find(|s| !["agent-project", "agent", "project", "global"].contains(&s.as_str())) {
                return Err(ManifestError::BadMemoryScope(bad.clone()));
            }
        }
        Ok(m)
    }
}

impl Capabilities {
    /// True when the plugin asks for nothing except `ui`.
    pub fn is_ui_only(&self) -> bool {
        self.ui && !self.approvals_decide && self.fs.is_none() && self.net.is_none() && self.exec.is_none() && self.memory.is_none()
    }

    /// Human-readable lines for the approval screen.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ui {
            out.push("Change how Discard looks (ui)".to_string());
        }
        if self.approvals_decide {
            out.push("Approve or deny tool calls automatically (approvals.decide)".to_string());
        }
        if let Some(fs) = &self.fs {
            let mode = if fs.write { "Read and write" } else { "Read" };
            out.push(format!("{mode} files under: {}", fs.paths.join(", ")));
        }
        if let Some(net) = &self.net {
            out.push(format!("Connect to: {}", net.hosts.join(", ")));
        }
        if let Some(exec) = &self.exec {
            out.push(format!(
                "Run programs ({}). This is full local access: a program can read files, use the network, and start other programs.",
                exec.commands.join(", ")
            ));
        }
        if let Some(mem) = &self.memory {
            if mem.write {
                out.push(format!(
                    "Read and write agent memory ({}). Memory under `core/` is added to the agent's instructions, so a write can change how the agent behaves.",
                    mem.scopes.join(", ")
                ));
            } else {
                out.push(format!("Read agent memory ({})", mem.scopes.join(", ")));
            }
        }
        out
    }

    /// True if `exec` is requested. It always means full local access.
    pub fn grants_full_local_access(&self) -> bool {
        self.exec.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "name = \"hello\"\nversion = \"0.1.0\"\napi = \"1\"\n";

    #[test]
    fn minimal_manifest_has_no_capabilities() {
        let m = Manifest::parse(MINIMAL).unwrap();
        assert_eq!(m.name, "hello");
        assert_eq!(m.capabilities, Capabilities::default());
        assert!(m.capabilities.describe().is_empty());
    }

    #[test]
    fn full_manifest_parses_every_scope() {
        let text = format!(
            "{MINIMAL}[capabilities]\nui = true\napprovals_decide = true\n\
             [capabilities.fs]\npaths = [\"/tmp/x\"]\nwrite = true\n\
             [capabilities.net]\nhosts = [\"example.com:443\"]\n\
             [capabilities.exec]\ncommands = [\"git\"]\n\
             [capabilities.memory]\nscopes = [\"agent\", \"project\"]\n"
        );
        let c = Manifest::parse(&text).unwrap().capabilities;
        assert!(c.ui && c.approvals_decide);
        assert_eq!(c.fs.unwrap().paths, ["/tmp/x"]);
        assert_eq!(c.net.unwrap().hosts, ["example.com:443"]);
        assert_eq!(c.exec.unwrap().commands, ["git"]);
        assert_eq!(c.memory.unwrap().scopes, ["agent", "project"]);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(matches!(Manifest::parse(&format!("{MINIMAL}sudo = true\n")), Err(ManifestError::Parse(_))));
        assert!(matches!(
            Manifest::parse(&format!("{MINIMAL}[capabilities]\nroot = true\n")),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn names_are_restricted() {
        for bad in ["", "Hello", "has space", "a/b", "..", &"x".repeat(65)] {
            let text = format!("name = \"{bad}\"\nversion = \"1\"\napi = \"1\"\n");
            assert_eq!(Manifest::parse(&text), Err(ManifestError::BadName(bad.to_string())), "{bad}");
        }
    }

    #[test]
    fn api_version_must_match() {
        let text = "name = \"a\"\nversion = \"1\"\napi = \"2\"\n";
        assert_eq!(Manifest::parse(text), Err(ManifestError::ApiMismatch { wanted: "2".into(), have: API_VERSION }));
    }

    #[test]
    fn memory_scopes_are_validated_and_empty_scope_lists_rejected() {
        let bad = format!("{MINIMAL}[capabilities.memory]\nscopes = [\"galaxy\"]\n");
        assert_eq!(Manifest::parse(&bad), Err(ManifestError::BadMemoryScope("galaxy".into())));
        let empty = format!("{MINIMAL}[capabilities.net]\nhosts = []\n");
        assert_eq!(Manifest::parse(&empty), Err(ManifestError::EmptyScope("net")));
    }

    #[test]
    fn fs_paths_must_be_absolute_and_free_of_dot_dot() {
        for bad in ["notes", "../x", "/tmp/../etc"] {
            let text = format!("{MINIMAL}[capabilities.fs]\npaths = [\"{bad}\"]\n");
            assert!(matches!(Manifest::parse(&text), Err(ManifestError::BadFsPath(_))), "{bad}");
        }
        assert!(Manifest::parse(&format!("{MINIMAL}[capabilities.fs]\npaths = [\"~/notes\", \"/tmp/x\"]\n")).is_ok());
    }

    #[test]
    fn memory_is_read_only_unless_write_is_set_and_the_review_says_which() {
        let read = Manifest::parse(&format!("{MINIMAL}[capabilities.memory]\nscopes = [\"global\"]\n")).unwrap().capabilities;
        assert!(!read.memory.as_ref().unwrap().write);
        assert!(read.describe()[0].starts_with("Read agent memory"));
        let write = Manifest::parse(&format!("{MINIMAL}[capabilities.memory]\nscopes = [\"global\"]\nwrite = true\n")).unwrap().capabilities;
        assert!(write.describe()[0].contains("change how the agent behaves"), "{:?}", write.describe());
    }

    #[test]
    fn ui_only_means_nothing_else_is_requested() {
        let ui = Manifest::parse(&format!("{MINIMAL}[capabilities]\nui = true\n")).unwrap().capabilities;
        assert!(ui.is_ui_only());
        let more = Manifest::parse(&format!("{MINIMAL}[capabilities]\nui = true\napprovals_decide = true\n")).unwrap().capabilities;
        assert!(!more.is_ui_only());
        assert!(!Capabilities::default().is_ui_only(), "no capabilities at all is not a ui plugin");
    }

    #[test]
    fn exec_is_described_as_full_local_access() {
        let c = Manifest::parse(&format!("{MINIMAL}[capabilities.exec]\ncommands = [\"ls\"]\n")).unwrap().capabilities;
        assert!(c.grants_full_local_access());
        assert!(c.describe().iter().any(|l| l.contains("full local access")), "{:?}", c.describe());
        assert!(!Capabilities::default().grants_full_local_access());
    }
}
