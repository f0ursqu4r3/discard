//! Read-only workspace summaries.
use crate::{
    model::{ChannelId, WorkspaceMode},
    store::{Store, StoreError},
};
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub exists: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRow {
    pub name: String,
    pub mode: WorkspaceMode,
    pub archived: bool,
    pub current: bool,
    pub worktree: Option<WorktreeInfo>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub name: String,
    pub path: String,
    pub exists: bool,
    pub channels: Vec<ChannelRow>,
}
pub fn rows(store: &Store, current: &ChannelId) -> Result<Vec<ProjectRow>, StoreError> {
    let mut out = Vec::new();
    for p in store.list_projects()? {
        let mut channels = Vec::new();
        for (c, archived) in store.all_channels_for_project(&p.id)? {
            let worktree = store.worktree_for(&c.id)?.map(|w| WorktreeInfo {
                exists: Path::new(&w.path).exists(),
                path: w.path,
                branch: w.branch,
            });
            channels.push(ChannelRow {
                name: c.name,
                mode: c.workspace_mode,
                archived,
                current: &c.id == current,
                worktree,
            });
        }
        out.push(ProjectRow {
            exists: Path::new(&p.path).exists(),
            name: p.name,
            path: p.path,
            channels,
        });
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ProjectId};
    fn fixture() -> (Store, ChannelId, tempfile::TempDir) {
        use crate::{
            agent::AgentConfig,
            capability::ApprovalPolicy,
            model::{AgentId, Channel, WorkspaceMode},
        };
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = Project {
            id: ProjectId::new(),
            name: "project".into(),
            path: dir.path().to_string_lossy().into(),
        };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store
            .insert_agent(
                &agent,
                &AgentConfig {
                    workspace_write: false,
                    reasoning_effort: None,
                    name: "agent".into(),
                    avatar: None,
                    provider: "claude".into(),
                    model: None,
                    system_prompt: String::new(),
                    policy: ApprovalPolicy::Ask,
                    plugin_write: false,
                },
            )
            .unwrap();
        let id = ChannelId::new();
        store
            .insert_channel(&Channel {
                id: id.clone(),
                project_id: Some(project.id),
                name: "channel".into(),
                agent_id: agent,
                workspace_mode: WorkspaceMode::Worktree,
            })
            .unwrap();
        (store, id, dir)
    }
    #[test]
    fn rows_list_each_project_with_its_channels_and_mark_the_current_one() {
        let (s, id, _) = fixture();
        let r = rows(&s, &id).unwrap();
        assert_eq!(r[0].name, "project");
        assert_eq!(r[0].channels[0].name, "channel");
        assert!(r[0].channels[0].current);
    }
    #[test]
    fn an_archived_channel_is_tagged_and_keeps_no_worktree() {
        let (s, id, _) = fixture();
        s.archive_channel(&id).unwrap();
        let r = rows(&s, &id).unwrap();
        assert!(r[0].channels[0].archived);
        assert!(r[0].channels[0].worktree.is_none());
    }
    #[test]
    fn a_worktree_record_shows_branch_and_path() {
        let (s, id, d) = fixture();
        let path = d.path().join("missing").to_string_lossy().to_string();
        s.save_worktree(&crate::store::WorktreeRecord {
            channel_id: id.clone(),
            path: path.clone(),
            branch: "branch".into(),
            base_commit: "base".into(),
            repo: "repo".into(), owned: true })
        .unwrap();
        let r = rows(&s, &id).unwrap();
        let w = r[0].channels[0].worktree.as_ref().unwrap();
        assert_eq!(w.branch, "branch");
        assert_eq!(w.path, path);
        assert!(!w.exists);
    }
    #[test]
    fn an_empty_store_gives_no_rows() {
        assert!(
            rows(&Store::open_in_memory().unwrap(), &ChannelId::new())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn a_missing_project_or_worktree_folder_is_marked_not_existing() {
        let s = Store::open_in_memory().unwrap();
        let p = Project {
            id: ProjectId::new(),
            name: "p".into(),
            path: "/no/such/path".into(),
        };
        s.insert_project(&p).unwrap();
        assert!(!rows(&s, &ChannelId::new()).unwrap()[0].exists);
    }
}
