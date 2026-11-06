//! Gives plugins access to agent memory. A plugin names a scope (`agent`, `project`, `global`). This maps it to
//! the active agent and project. Each engine event and UI callback pins its agent for the whole callback.

use discard_core::memory::{MemoryStore, Scope};
use discard_core::model::{AgentId, ProjectId};
use discard_lua::{MemoryFile, MemoryHit, MemoryHost, MemoryText};
use std::sync::{Arc, Mutex};
use std::cell::RefCell;
thread_local! { static EVENT_AGENT: RefCell<Option<AgentId>> = const { RefCell::new(None) }; }
thread_local! { static EVENT_PROJECT: RefCell<Option<Option<ProjectId>>> = const { RefCell::new(None) }; }
type ActiveProject = Arc<dyn Fn() -> Option<ProjectId> + Send + Sync>;
type ActiveAgent = Arc<dyn Fn() -> AgentId + Send + Sync>;

pub struct PluginMemory {
    store: Arc<MemoryStore>,
    agent: AgentId,
    project: Option<ProjectId>,
    active: Mutex<Option<ActiveAgent>>,
    active_project: Mutex<Option<ActiveProject>>,
}

impl PluginMemory {
    pub fn new(store: Arc<MemoryStore>, agent: AgentId, project: impl Into<Option<ProjectId>>) -> Self {
        Self { store, agent, project: project.into(), active: Mutex::new(None), active_project: Mutex::new(None) }
    }

    pub fn set_active_source(&self, source: ActiveAgent) { *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(source); }
    pub fn for_agent<T>(agent: AgentId, callback: impl FnOnce() -> T) -> T {
        struct Restore(Option<AgentId>);
        impl Drop for Restore { fn drop(&mut self) { EVENT_AGENT.with(|scope| { scope.replace(self.0.take()); }); } }
        let previous = EVENT_AGENT.with(|scope| scope.replace(Some(agent)));
        let _restore = Restore(previous);
        callback()
    }
    pub fn set_project_source(&self, source: ActiveProject) { *self.active_project.lock().unwrap_or_else(|e| e.into_inner()) = Some(source); }
    pub fn for_context<T>(agent: AgentId, project: impl Into<Option<ProjectId>>, callback: impl FnOnce() -> T) -> T {
        struct Restore(Option<Option<ProjectId>>);
        impl Drop for Restore { fn drop(&mut self) { EVENT_PROJECT.with(|scope| { scope.replace(self.0.take()); }); } }
        let previous = EVENT_PROJECT.with(|scope| scope.replace(Some(project.into())));
        let _restore = Restore(previous);
        Self::for_agent(agent, callback)
    }
    fn project(&self) -> Option<ProjectId> {
        EVENT_PROJECT.with(|scope| scope.borrow().clone()).unwrap_or_else(|| {
            self.active_project.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or_else(|| self.project.clone(), |source| source())
        })
    }
    fn agent(&self) -> AgentId {
        EVENT_AGENT.with(|scope| scope.borrow().clone()).unwrap_or_else(|| {
            self.active.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or_else(|| self.agent.clone(), |source| source())
        })
    }
    fn scope(&self, name: &str) -> Result<Scope, String> {
        match name {
            "agent" => Ok(Scope::Agent(self.agent())),
            "project" => Ok(Scope::Project(self.project().ok_or("no project in this conversation")?)),
            "agent-project" => Ok(Scope::AgentProject(self.agent(), self.project().ok_or("no project in this conversation")?)),
            "global" => Ok(Scope::Global),
            other => Err(format!("unknown memory scope `{other}`")),
        }
    }
}

impl MemoryHost for PluginMemory {
    fn list(&self, scope: &str) -> Result<Vec<MemoryFile>, String> {
        let files = self.store.plugin_list(&self.scope(scope)?).map_err(|e| e.to_string())?;
        Ok(files.into_iter().map(|f| MemoryFile { path: f.path, version: f.version, bytes: f.bytes }).collect())
    }

    fn read(&self, scope: &str, path: &str) -> Result<MemoryText, String> {
        let c = self.store.plugin_read(&self.scope(scope)?, path).map_err(|e| e.to_string())?;
        Ok(MemoryText { text: c.text, version: c.version })
    }

    fn search(&self, scopes: &[String], query: &str, limit: usize) -> Result<Vec<MemoryHit>, String> {
        let scopes = scopes.iter().map(|s| self.scope(s)).collect::<Result<Vec<_>, _>>()?;
        let hits = self.store.plugin_search(&scopes, query, limit).map_err(|e| e.to_string())?;
        Ok(hits.into_iter().map(|h| MemoryHit { scope: h.scope, path: h.path, snippet: h.snippet }).collect())
    }

    fn append(&self, scope: &str, path: &str, text: &str) -> Result<String, String> {
        self.store.plugin_append(&self.scope(scope)?, path, text).map_err(|e| e.to_string())
    }

    fn edit(&self, scope: &str, path: &str, old: &str, new: &str, expected_version: &str) -> Result<String, String> {
        self.store.plugin_edit(&self.scope(scope)?, path, old, new, expected_version).map_err(|e| e.to_string())
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
    fn project_scope_follows_selection_but_callback_keeps_its_origin() {
        let r = rig();
        let h = host(&r, "agent");
        h.set_project_source(Arc::new(|| Some(ProjectId("selected".into()))));
        PluginMemory::for_context(AgentId("agent".into()), ProjectId("origin".into()), || h.append("project", "notes.md", "origin").unwrap());
        assert!(h.list("project").unwrap().is_empty());
        h.append("project", "notes.md", "selected").unwrap();
        assert_eq!(r.store.plugin_read(&Scope::Project(ProjectId("origin".into())), "notes.md").unwrap().text, "origin");
        assert_eq!(h.read("project", "notes.md").unwrap().text, "selected");
    }
    #[test]
    fn scope_names_map_to_this_apps_agent_and_project() {
        let r = rig();
        let h = host(&r, "a1");
        h.append("agent", "n.md", "mine").unwrap();
        h.append("project", "n.md", "ours").unwrap();
        h.append("global", "n.md", "everyone").unwrap();
        assert_eq!(r.store.plugin_read(&Scope::Agent(AgentId("a1".into())), "n.md").unwrap().text, "mine");
        assert_eq!(r.store.plugin_read(&Scope::Project(ProjectId("p1".into())), "n.md").unwrap().text, "ours");
        assert_eq!(r.store.plugin_read(&Scope::Global, "n.md").unwrap().text, "everyone");
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

    #[test]
    fn background_plugin_event_uses_its_agent_then_restores_the_active_agent() {
        let r = rig();
        let h = host(&r, "initial");
        h.set_active_source(Arc::new(|| AgentId("active".into())));
        PluginMemory::for_agent(AgentId("background".into()), || h.append("agent", "notes.md", "background").unwrap());
        h.append("agent", "notes.md", "active").unwrap();
        assert_eq!(r.store.plugin_read(&Scope::Agent(AgentId("background".into())), "notes.md").unwrap().text, "background");
        assert_eq!(r.store.plugin_read(&Scope::Agent(AgentId("active".into())), "notes.md").unwrap().text, "active");
        assert!(r.store.list(&Scope::Agent(AgentId("initial".into()))).unwrap().is_empty());
    }

    #[test]
    fn one_callback_keeps_its_agent_when_active_channel_changes() {
        let r = rig();
        let h = host(&r, "initial");
        let active = Arc::new(Mutex::new(AgentId("first".into())));
        let source = active.clone();
        h.set_active_source(Arc::new(move || source.lock().unwrap().clone()));
        PluginMemory::for_agent(AgentId("first".into()), || {
            h.append("agent", "notes.md", "before").unwrap();
            *active.lock().unwrap() = AgentId("second".into());
            h.append("agent", "notes.md", "after").unwrap();
            assert!(h.read("agent", "notes.md").unwrap().text.contains("before"));
        });
        assert!(r.store.plugin_read(&Scope::Agent(AgentId("first".into())), "notes.md").unwrap().text.contains("after"));
        assert!(h.list("agent").unwrap().is_empty(), "next callback sees the newly selected agent");
    }
}

#[cfg(test)]
mod optional_project_tests {
    use super::*;
    #[test]
    fn projectless_callback_rejects_project_scopes_and_restores_selected_project() {
        let d=tempfile::tempdir().unwrap();
        let store=Arc::new(MemoryStore::open_in_memory_index(d.path()).unwrap());
        let agent=AgentId("a".into());
        let host=PluginMemory::new(store,agent.clone(),Some(ProjectId("selected".into())));
        PluginMemory::for_context(agent,None,|| {
            assert!(host.list("project").is_err());
            assert!(host.list("agent-project").is_err());
            host.append("agent","private.md","private").unwrap();
        });
        host.append("agent-project","project.md","scoped").unwrap();
        assert_eq!(host.read("agent-project","project.md").unwrap().text,"scoped");
    }
}
