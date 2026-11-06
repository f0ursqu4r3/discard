//! Read-only memory scope summaries.
use crate::{
    agent::AgentConfig,
    memory::{MemoryStore, Scope},
    model::{AgentId, Project},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub bytes: usize,
    pub core: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSummary {
    pub scope: Scope,
    pub title: String,
    pub files: Vec<FileRow>,
}
pub fn summaries(
    memory: &MemoryStore,
    agents: &[(AgentId, AgentConfig)],
    projects: &[Project],
) -> Vec<ScopeSummary> {
    let mut scopes = vec![(Scope::Global, "Global".to_string())];
    scopes.extend(
        projects
            .iter()
            .map(|p| (Scope::Project(p.id.clone()), p.name.clone())),
    );
    scopes.extend(
        agents
            .iter()
            .map(|(id, a)| (Scope::Agent(id.clone()), a.name.clone())),
    );
    for (id, agent) in agents {
        for project in projects {
            scopes.push((Scope::AgentProject(id.clone(), project.id.clone()), format!("{} · {}", agent.name, project.name)));
        }
    }
    for scope in memory.scopes_on_disk() {
        if !scopes.iter().any(|(s, _)| s == &scope) {
            let title = scope.label();
            scopes.push((scope, title));
        }
    }
    scopes
        .into_iter()
        .map(|(scope, title)| {
            let files = memory
                .list(&scope)
                .unwrap_or_default()
                .into_iter()
                .map(|f| FileRow {
                    core: f.path.starts_with("core/"),
                    path: f.path,
                    bytes: f.bytes,
                })
                .collect();
            ScopeSummary {
                scope,
                title,
                files,
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectId;
    #[test]
    fn summaries_list_global_project_and_agent_scopes_with_names_as_titles() {
        use crate::{capability::ApprovalPolicy, model::AgentId};
        let d = tempfile::tempdir().unwrap();
        let m = MemoryStore::open_in_memory_index(d.path()).unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "Project".into(),
            path: "/p".into(),
        };
        let id = AgentId::new();
        let cfg = AgentConfig {
            reasoning_effort: None,
            name: "Agent".into(),
            avatar: None,
            provider: "claude".into(),
            model: None,
            system_prompt: String::new(),
            policy: ApprovalPolicy::Ask,
            plugin_write: false,
        };
        let rows = summaries(&m, &[(id, cfg)], &[project]);
        assert_eq!(
            rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Global", "Project", "Agent", "Agent · Project"]
        );
    }
    #[test]
    fn an_empty_store_gives_a_global_scope_with_no_files() {
        let d = tempfile::tempdir().unwrap();
        let m = MemoryStore::open_in_memory_index(d.path()).unwrap();
        let rows = summaries(&m, &[], &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scope, Scope::Global);
        assert!(rows[0].files.is_empty());
    }
    #[test]
    fn a_file_under_core_is_marked_core() {
        let d = tempfile::tempdir().unwrap();
        let m = MemoryStore::open_in_memory_index(d.path()).unwrap();
        m.create(&Scope::Global, "core/note.md", "text").unwrap();
        assert!(summaries(&m, &[], &[])[0].files[0].core);
    }
    #[test]
    fn a_scope_on_disk_with_no_known_owner_is_listed_by_its_label() {
        let d = tempfile::tempdir().unwrap();
        let m = MemoryStore::open_in_memory_index(d.path()).unwrap();
        let scope = Scope::Project(ProjectId("orphan".into()));
        m.create(&scope, "note.md", "text").unwrap();
        let rows = summaries(&m, &[], &[]);
        assert_eq!(rows[1].title, "project:orphan");
    }
    #[cfg(unix)]
    #[test]
    fn a_symlinked_scope_folder_gives_no_files_and_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let m = MemoryStore::open_in_memory_index(d.path()).unwrap();
        std::os::unix::fs::symlink(d.path(), d.path().join("global")).unwrap();
        assert!(summaries(&m, &[], &[])[0].files.is_empty());
    }
}
