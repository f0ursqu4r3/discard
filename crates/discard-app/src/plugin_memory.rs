//! Gives plugins access to agent memory. A plugin names a scope (`agent`, `project`, `global`). This maps it to
//! the ids of the agent and project this app runs, so a plugin never passes an id or a path for a scope.

use discard_core::memory::{MemoryStore, Scope};
use discard_core::model::{AgentId, ProjectId};
use discard_lua::{MemoryFile, MemoryHit, MemoryHost, MemoryText};
use std::sync::Arc;

pub struct PluginMemory {
    store: Arc<MemoryStore>,
    agent: AgentId,
    project: ProjectId,
}

impl PluginMemory {
    pub fn new(store: Arc<MemoryStore>, agent: AgentId, project: ProjectId) -> Self {
        Self { store, agent, project }
    }

    fn scope(&self, name: &str) -> Result<Scope, String> {
        match name {
            "agent" => Ok(Scope::Agent(self.agent.clone())),
            "project" => Ok(Scope::Project(self.project.clone())),
            "global" => Ok(Scope::Global),
            other => Err(format!("unknown memory scope `{other}`")),
        }
    }
}

impl MemoryHost for PluginMemory {
    fn list(&self, scope: &str) -> Result<Vec<MemoryFile>, String> {
        let files = self.store.list(&self.scope(scope)?).map_err(|e| e.to_string())?;
        Ok(files.into_iter().map(|f| MemoryFile { path: f.path, version: f.version, bytes: f.bytes }).collect())
    }

    fn read(&self, scope: &str, path: &str) -> Result<MemoryText, String> {
        let c = self.store.read(&self.scope(scope)?, path).map_err(|e| e.to_string())?;
        Ok(MemoryText { text: c.text, version: c.version })
    }

    fn search(&self, scopes: &[String], query: &str, limit: usize) -> Result<Vec<MemoryHit>, String> {
        let scopes = scopes.iter().map(|s| self.scope(s)).collect::<Result<Vec<_>, _>>()?;
        let hits = self.store.search(&scopes, query, limit).map_err(|e| e.to_string())?;
        Ok(hits.into_iter().map(|h| MemoryHit { scope: h.scope, path: h.path, snippet: h.snippet }).collect())
    }

    fn append(&self, scope: &str, path: &str, text: &str) -> Result<String, String> {
        self.store.append(&self.scope(scope)?, path, text).map_err(|e| e.to_string())
    }

    fn edit(&self, scope: &str, path: &str, old: &str, new: &str, expected_version: &str) -> Result<String, String> {
        self.store.edit(&self.scope(scope)?, path, old, new, expected_version).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rig {
        _dir: tempfile::TempDir,
        store: Arc<MemoryStore>,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open_in_memory_index(dir.path()).unwrap());
        Rig { _dir: dir, store }
    }

    fn host(r: &Rig, agent: &str) -> PluginMemory {
        PluginMemory::new(r.store.clone(), AgentId(agent.into()), ProjectId("p1".into()))
    }

    #[test]
    fn scope_names_map_to_this_apps_agent_and_project() {
        let r = rig();
        let h = host(&r, "a1");
        h.append("agent", "n.md", "mine").unwrap();
        h.append("project", "n.md", "ours").unwrap();
        h.append("global", "n.md", "everyone").unwrap();
        assert_eq!(r.store.read(&Scope::Agent(AgentId("a1".into())), "n.md").unwrap().text, "mine");
        assert_eq!(r.store.read(&Scope::Project(ProjectId("p1".into())), "n.md").unwrap().text, "ours");
        assert_eq!(r.store.read(&Scope::Global, "n.md").unwrap().text, "everyone");
    }

    #[test]
    fn another_agents_private_memory_is_out_of_reach() {
        let r = rig();
        host(&r, "other").append("agent", "secret.md", "private").unwrap();
        let mine = host(&r, "a1");
        assert!(mine.read("agent", "secret.md").is_err());
        assert!(mine.list("agent").unwrap().is_empty());
    }

    #[test]
    fn unknown_scopes_and_bad_paths_are_errors() {
        let h = host(&rig(), "a1");
        assert!(h.read("galaxy", "a.md").unwrap_err().contains("unknown memory scope"));
        assert!(h.append("global", "../escape.md", "x").is_err());
        assert!(h.append("global", "/abs.md", "x").is_err());
        assert!(h.search(&["galaxy".into()], "x", 5).is_err());
    }

    #[test]
    fn edit_needs_the_current_version_and_search_finds_text() {
        let h = host(&rig(), "a1");
        let v1 = h.append("global", "n.md", "alpha beta").unwrap();
        let v2 = h.edit("global", "n.md", "beta", "gamma", &v1).unwrap();
        assert!(h.edit("global", "n.md", "gamma", "delta", &v1).unwrap_err().contains("changed since"));
        assert_eq!(h.read("global", "n.md").unwrap(), MemoryText { text: "alpha gamma".into(), version: v2 });
        let hits = h.search(&["global".into()], "gamma", 5).unwrap();
        assert_eq!((hits[0].scope.as_str(), hits[0].path.as_str()), ("global", "n.md"));
        assert!(h.search(&["project".into()], "gamma", 5).unwrap().is_empty(), "search stays inside the named scopes");
    }
}
