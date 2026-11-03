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

/// Providers Discard can run.
pub const PROVIDERS: &[&str] = &["claude", "codex"];

/// Find or create the workspace for `project_dir` and `provider`.
/// A project has one channel per provider, each with its own agent and session.
pub fn ensure_workspace(store: &Store, project_dir: &Path, provider: &str) -> Result<Workspace> {
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

    let existing = store.channels_for_project(&project.id)?;
    let mut found = None;
    for channel in &existing {
        let agent = store.load_agent(&channel.agent_id)?;
        if agent.provider == provider {
            found = Some((channel.clone(), agent));
            break;
        }
    }
    let (channel, agent) = match found {
        Some(pair) => pair,
        None => {
            // The default agent asks before it acts.
            let agent = AgentConfig {
                name: display_name(provider),
                avatar: None,
                provider: provider.to_string(),
                model: None,
                system_prompt: String::new(),
                policy: ApprovalPolicy::Ask,
            };
            let agent_id = AgentId::new();
            store.insert_agent(&agent_id, &agent)?;
            let channel = Channel {
                id: ChannelId::new(),
                project_id: project.id.clone(),
                // The first channel of a project is `general`. Later ones are named after their provider.
                name: if existing.is_empty() { "general".into() } else { provider.to_string() },
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

fn display_name(provider: &str) -> String {
    let mut chars = provider.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
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
        let ws = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude").unwrap();
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
        let first = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude").unwrap();
        let second = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn different_directories_get_separate_projects() {
        let store = Store::open_in_memory().unwrap();
        let a = ensure_workspace(&store, &PathBuf::from("/work/a"), "claude").unwrap();
        let b = ensure_workspace(&store, &PathBuf::from("/work/b"), "claude").unwrap();
        assert_ne!(a.project.id, b.project.id);
        assert_ne!(a.session_id, b.session_id);
    }

    #[test]
    fn each_provider_gets_its_own_channel_agent_and_session_in_the_same_project() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let claude = ensure_workspace(&store, &dir, "claude").unwrap();
        let codex = ensure_workspace(&store, &dir, "codex").unwrap();
        assert_eq!(claude.project, codex.project, "the same project");
        assert_ne!(claude.channel.id, codex.channel.id);
        assert_ne!(claude.session_id, codex.session_id);
        assert_ne!(claude.channel.agent_id, codex.channel.agent_id);
        assert_eq!((claude.agent.provider.as_str(), codex.agent.provider.as_str()), ("claude", "codex"));
        assert_eq!((claude.channel.name.as_str(), codex.channel.name.as_str()), ("general", "codex"));
        assert_eq!((claude.agent.name.as_str(), codex.agent.name.as_str()), ("Claude", "Codex"));
    }

    #[test]
    fn switching_back_to_a_provider_reuses_its_channel() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let first = ensure_workspace(&store, &dir, "codex").unwrap();
        ensure_workspace(&store, &dir, "claude").unwrap();
        assert_eq!(ensure_workspace(&store, &dir, "codex").unwrap(), first);
        assert_eq!(first.channel.name, "general", "whichever provider comes first owns `general`");
    }

    #[test]
    fn a_root_directory_still_gets_a_usable_name() {
        let store = Store::open_in_memory().unwrap();
        let ws = ensure_workspace(&store, &PathBuf::from("/"), "claude").unwrap();
        assert!(!ws.project.name.is_empty());
    }
}
