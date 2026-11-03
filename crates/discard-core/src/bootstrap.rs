//! First-run setup: a project, a default agent, a channel, and a session. Later runs reuse them.

use crate::agent::AgentConfig;
use crate::capability::ApprovalPolicy;
use crate::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use crate::event::SessionId;
use crate::store::{Result, Store};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub project: Project,
    pub channel: Channel,
    pub agent: AgentConfig,
    pub session_id: SessionId,
}

/// Find or create the workspace for `project_dir`.
pub fn ensure_workspace(store: &Store, project_dir: &Path) -> Result<Workspace> {
    let path = project_dir.to_string_lossy().into_owned();
    let project = match store.project_by_path(&path)? {
        Some(project) => project,
        None => {
            let name = project_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "root".to_string());
            let project = Project { id: ProjectId::new(), name, path };
            store.insert_project(&project)?;
            project
        }
    };

    let (channel, agent) = match store.first_channel(&project.id)? {
        Some(channel) => {
            let agent = store.load_agent(&channel.agent_id)?;
            (channel, agent)
        }
        None => {
            // The default agent asks before it acts.
            let agent = AgentConfig {
                name: "Claude".into(),
                avatar: None,
                provider: "claude".into(),
                model: None,
                system_prompt: String::new(),
                policy: ApprovalPolicy::Ask,
            };
            let agent_id = AgentId::new();
            store.insert_agent(&agent_id, &agent)?;
            let channel = Channel {
                id: ChannelId::new(),
                project_id: project.id.clone(),
                name: "general".into(),
                agent_id,
                workspace_mode: WorkspaceMode::MainCheckout,
            };
            store.insert_channel(&channel)?;
            (channel, agent)
        }
    };

    let session_id = match store.latest_session(&channel.id)? {
        Some(id) => id,
        None => {
            let id = SessionId::new();
            store.create_session(&id, &channel.id)?;
            id
        }
    };
    Ok(Workspace { project, channel, agent, session_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::ApprovalPolicy;
    use crate::model::WorkspaceMode;
    use std::path::PathBuf;

    #[test]
    fn first_run_creates_a_project_agent_channel_and_session() {
        let store = Store::open_in_memory().unwrap();
        let ws = ensure_workspace(&store, &PathBuf::from("/work/my-app")).unwrap();
        assert_eq!(ws.project.name, "my-app");
        assert_eq!(ws.project.path, "/work/my-app");
        assert_eq!(ws.channel.name, "general");
        assert_eq!(ws.channel.workspace_mode, WorkspaceMode::MainCheckout);
        assert_eq!(ws.agent.provider, "claude");
        assert_eq!(ws.agent.policy, ApprovalPolicy::Ask, "the default agent asks before acting");
        assert_eq!(store.latest_session(&ws.channel.id).unwrap(), Some(ws.session_id.clone()));
    }

    #[test]
    fn a_second_run_reuses_everything() {
        let store = Store::open_in_memory().unwrap();
        let first = ensure_workspace(&store, &PathBuf::from("/work/my-app")).unwrap();
        let second = ensure_workspace(&store, &PathBuf::from("/work/my-app")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn different_directories_get_separate_projects() {
        let store = Store::open_in_memory().unwrap();
        let a = ensure_workspace(&store, &PathBuf::from("/work/a")).unwrap();
        let b = ensure_workspace(&store, &PathBuf::from("/work/b")).unwrap();
        assert_ne!(a.project.id, b.project.id);
        assert_ne!(a.session_id, b.session_id);
    }

    #[test]
    fn a_root_directory_still_gets_a_usable_name() {
        let store = Store::open_in_memory().unwrap();
        let ws = ensure_workspace(&store, &PathBuf::from("/")).unwrap();
        assert!(!ws.project.name.is_empty());
    }
}
