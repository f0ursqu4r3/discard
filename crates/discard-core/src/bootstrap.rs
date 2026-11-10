//! First-run setup: a project, a default agent, a channel, and a session. Later runs reuse them.

use crate::agent::AgentConfig;
use crate::capability::ApprovalPolicy;
use crate::model::{AgentId, Channel, ChannelId, Project, ProjectId, WorkspaceMode};
use crate::event::SessionId;
use crate::store::{Result, Store};
use crate::workspace::change_workspace_mode;
use crate::worktree::WorktreeManager;
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

/// Which agent to use in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The everyday agent. It cannot write plugins.
    Default,
    /// The bundled Builder agent. It has the `plugin.write` permission.
    Builder,
}

/// Where a channel's agent works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceChoice {
    /// New channels in a git repository get their own worktree. Existing channels are left alone.
    Auto,
    /// Use a git worktree for this channel. Changing an existing channel starts a new session.
    Worktree,
    /// Work in the project's main checkout. Changing an existing channel starts a new session.
    Main,
}

/// Find or create the workspace for `project_dir`, `provider`, and `role`.
/// A project has one channel per provider and role, each with its own agent and session.
pub fn ensure_workspace(store: &Store, project_dir: &Path, provider: &str, role: Role, choice: WorkspaceChoice) -> Result<Workspace> {
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
        match store.agent_archived(&channel.agent_id) {
            Ok(false) => (),
            Ok(true) | Err(crate::store::StoreError::NotFound(_)) => continue,
            Err(error) => return Err(error),
        }
        let agent = store.load_agent(&channel.agent_id)?;
        if agent.provider == provider && agent.plugin_write == (role == Role::Builder) {
            found = Some((channel.clone(), agent));
            break;
        }
    }
    let (channel, agent) = match found {
        Some(pair) => pair,
        None => {
            let (agent_id, agent) = if let Some(existing) = store.find_project_agent(&project.id, provider, role == Role::Builder)? {
                existing
            } else {
            let agent = AgentConfig {
                workspace_write: false, sandbox_autorun: false,
                reasoning_effort: None,
                name: if role == Role::Builder { "Builder".to_string() } else { crate::agent::generated_name() },
                avatar: None,
                provider: provider.to_string(),
                model: None,
                system_prompt: String::new(),
                policy: ApprovalPolicy::Ask,
                plugin_write: role == Role::Builder,
            };
            let agent_id = AgentId::new();
            store.insert_agent(&agent_id, &agent)?;
            store.ensure_default_conversation(&agent_id)?;
            (agent_id, agent)
            };
            let channel = Channel {
                id: ChannelId::new(),
                project_id: Some(project.id.clone()),
                // The first channel of a project is `general`. Later ones are named after their provider.
                name: match role {
                    Role::Builder if provider == "claude" => "builder".into(),
                    Role::Builder => format!("builder-{provider}"),
                    Role::Default if existing.is_empty() => "general".into(),
                    Role::Default => provider.to_string(),
                },
                agent_id,
                workspace_mode: match choice {
                    WorkspaceChoice::Worktree => WorkspaceMode::Worktree,
                    WorkspaceChoice::Main => WorkspaceMode::MainCheckout,
                    WorkspaceChoice::Auto if WorktreeManager::is_repo(project_dir) => WorkspaceMode::Worktree,
                    WorkspaceChoice::Auto => WorkspaceMode::MainCheckout,
                },
            };
            store.insert_channel(&channel)?;
            (channel, agent)
        }
    };

    store.register_bootstrap_project_channel(&channel.id)?;

    // An explicit choice that differs from the channel's mode moves it, with a new session.
    let wanted = match choice {
        WorkspaceChoice::Worktree => Some(WorkspaceMode::Worktree),
        WorkspaceChoice::Main => Some(WorkspaceMode::MainCheckout),
        WorkspaceChoice::Auto => None,
    };
    let mut channel = channel;
    if let Some(mode) = wanted.filter(|m| *m != channel.workspace_mode) {
        change_workspace_mode(store, &channel.id, mode)?;
        channel.workspace_mode = mode;
    }

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
        let ws = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(ws.project.name, "my-app");
        assert_eq!(ws.project.path, "/work/my-app");
        assert_eq!(ws.channel.name, "general");
        assert_eq!(ws.channel.workspace_mode, WorkspaceMode::MainCheckout);
        assert_eq!(ws.agent.provider, "claude");
        assert_eq!(ws.agent.policy, ApprovalPolicy::Ask, "the default agent asks before acting");
        assert_eq!(store.latest_session(&ws.channel.id).unwrap(), Some(ws.session_id.clone()));
    }

    #[test]
    fn after_archiving_the_only_channel_a_new_run_reuses_the_edited_agent() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let first = ensure_workspace(&store, &dir, "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let mut config = first.agent.clone();
        config.model = Some("edited".into());
        store.update_agent(&first.channel.agent_id, &config).unwrap();
        store.archive_channel(&first.channel.id).unwrap();
        let second = ensure_workspace(&store, &dir, "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_ne!(first.channel.id, second.channel.id);
        assert_eq!(first.channel.agent_id, second.channel.agent_id);
        assert_eq!(second.agent.model.as_deref(), Some("edited"));
        assert_eq!(store.list_agents().unwrap().len(), 1);
    }

    #[test]
    fn a_new_project_never_reuses_another_projects_agent() {
        let store = Store::open_in_memory().unwrap();
        let first = ensure_workspace(&store, &PathBuf::from("/work/one"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let second = ensure_workspace(&store, &PathBuf::from("/work/two"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_ne!(first.channel.agent_id, second.channel.agent_id, "agent-private memory is keyed by agent");
        assert_eq!(store.list_agents().unwrap().len(), 2);
    }

    #[test]
    fn archiving_in_one_project_does_not_hand_its_agent_to_another() {
        let store = Store::open_in_memory().unwrap();
        let one = ensure_workspace(&store, &PathBuf::from("/work/one"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        store.archive_channel(&one.channel.id).unwrap();
        let two = ensure_workspace(&store, &PathBuf::from("/work/two"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_ne!(one.channel.agent_id, two.channel.agent_id);
        let again = ensure_workspace(&store, &PathBuf::from("/work/one"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(again.channel.agent_id, one.channel.agent_id, "the project's own agent comes back");
    }

    #[test]
    fn a_second_run_reuses_everything() {
        let store = Store::open_in_memory().unwrap();
        let first = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let second = ensure_workspace(&store, &PathBuf::from("/work/my-app"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn different_directories_get_separate_projects() {
        let store = Store::open_in_memory().unwrap();
        let a = ensure_workspace(&store, &PathBuf::from("/work/a"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let b = ensure_workspace(&store, &PathBuf::from("/work/b"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_ne!(a.project.id, b.project.id);
        assert_ne!(a.session_id, b.session_id);
    }

    #[test]
    fn each_provider_gets_its_own_channel_agent_and_session_in_the_same_project() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let claude = ensure_workspace(&store, &dir, "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let codex = ensure_workspace(&store, &dir, "codex", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(claude.project, codex.project, "the same project");
        assert_ne!(claude.channel.id, codex.channel.id);
        assert_ne!(claude.session_id, codex.session_id);
        assert_ne!(claude.channel.agent_id, codex.channel.agent_id);
        assert_eq!((claude.agent.provider.as_str(), codex.agent.provider.as_str()), ("claude", "codex"));
        assert_eq!((claude.channel.name.as_str(), codex.channel.name.as_str()), ("general", "codex"));
        assert!(!claude.agent.name.is_empty());
        assert!(!codex.agent.name.is_empty());
    }

    #[test]
    fn switching_back_to_a_provider_reuses_its_channel() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let first = ensure_workspace(&store, &dir, "codex", Role::Default, WorkspaceChoice::Auto).unwrap();
        ensure_workspace(&store, &dir, "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(ensure_workspace(&store, &dir, "codex", Role::Default, WorkspaceChoice::Auto).unwrap(), first);
        assert_eq!(first.channel.name, "general", "whichever provider comes first owns `general`");
    }

    #[test]
    fn the_builder_is_a_separate_agent_with_plugin_write_on() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let normal = ensure_workspace(&store, &dir, "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        let builder = ensure_workspace(&store, &dir, "claude", Role::Builder, WorkspaceChoice::Auto).unwrap();
        assert!(!normal.agent.plugin_write, "the everyday agent must not be able to write plugins");
        assert!(builder.agent.plugin_write);
        assert_eq!((builder.agent.name.as_str(), builder.channel.name.as_str()), ("Builder", "builder"));
        assert_ne!(normal.channel.id, builder.channel.id);
        assert_ne!(normal.session_id, builder.session_id);
        assert_eq!(builder.agent.policy, ApprovalPolicy::Ask, "the builder still asks before running commands");
    }

    #[test]
    fn the_builder_is_reused_per_provider() {
        let store = Store::open_in_memory().unwrap();
        let dir = PathBuf::from("/work/my-app");
        let first = ensure_workspace(&store, &dir, "codex", Role::Builder, WorkspaceChoice::Auto).unwrap();
        assert_eq!(first.channel.name, "builder-codex");
        assert_eq!(ensure_workspace(&store, &dir, "codex", Role::Builder, WorkspaceChoice::Auto).unwrap(), first);
        let claude = ensure_workspace(&store, &dir, "claude", Role::Builder, WorkspaceChoice::Auto).unwrap();
        assert_ne!(claude.channel.id, first.channel.id);
    }

    #[test]
    fn builder_bootstrap_survives_v4_backfill_without_changing_channel_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("builder.db");
        let project_dir = PathBuf::from("/work/builder-compat");
        let (channel_id, session_id, mode) = {
            let store = Store::open(&path).unwrap();
            let builder = ensure_workspace(&store, &project_dir, "claude", Role::Builder, WorkspaceChoice::Main).unwrap();
            assert_eq!(store.project_channel_metadata(&builder.channel.id).unwrap().unwrap().kind, crate::model::ProjectChannelKind::Legacy);
            (builder.channel.id, builder.session_id, builder.channel.workspace_mode)
        };
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute("DELETE FROM project_channels WHERE channel_id=?1", [&channel_id.0]).unwrap();
            conn.execute("DELETE FROM channel_agents WHERE channel_id=?1", [&channel_id.0]).unwrap();
            conn.execute("DELETE FROM project_agents WHERE project_id=(SELECT project_id FROM channels WHERE id=?1)", [&channel_id.0]).unwrap();
            conn.execute_batch("ALTER TABLE projects DROP COLUMN avatar; ALTER TABLE messages DROP COLUMN author_json;").unwrap();
            conn.execute_batch("DROP TABLE always_allow_rules;").unwrap();
            conn.pragma_update(None, "user_version", 4).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 10);
        let metadata = store.project_channel_metadata(&channel_id).unwrap().unwrap();
        assert_eq!(metadata.kind, crate::model::ProjectChannelKind::Legacy);
        assert_eq!(store.load_channel(&channel_id).unwrap().workspace_mode, mode);
        assert_eq!(store.latest_session(&channel_id).unwrap(), Some(session_id.clone()));
        let builder = ensure_workspace(&store, &project_dir, "claude", Role::Builder, WorkspaceChoice::Main).unwrap();
        assert_eq!(builder.channel.id, channel_id);
        assert_eq!(builder.session_id, session_id);
        assert_eq!(store.project_channel_metadata(&channel_id).unwrap().unwrap().kind, crate::model::ProjectChannelKind::Legacy);
        assert_eq!(store.channel_agents(&channel_id).unwrap(), vec![builder.channel.agent_id.clone()]);
        assert!(store.project_agents(&builder.project.id).unwrap().contains(&builder.channel.agent_id));
    }

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let ok = crate::process::spawn_std(std::process::Command::new("git")
                .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(dir.path())).and_then(|mut child| child.wait())
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q"]);
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn new_channels_in_a_git_repository_get_a_worktree_and_others_use_the_main_checkout() {
        let store = Store::open_in_memory().unwrap();
        let repo = git_repo();
        assert_eq!(ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Auto).unwrap().channel.workspace_mode, WorkspaceMode::Worktree);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(ensure_workspace(&store, plain.path(), "claude", Role::Default, WorkspaceChoice::Auto).unwrap().channel.workspace_mode, WorkspaceMode::MainCheckout);
    }

    #[test]
    fn an_explicit_choice_sets_the_mode_of_a_new_channel() {
        let store = Store::open_in_memory().unwrap();
        let repo = git_repo();
        assert_eq!(ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Main).unwrap().channel.workspace_mode, WorkspaceMode::MainCheckout);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(ensure_workspace(&store, plain.path(), "claude", Role::Default, WorkspaceChoice::Worktree).unwrap().channel.workspace_mode, WorkspaceMode::Worktree);
    }

    #[test]
    fn auto_never_moves_an_existing_channel() {
        let store = Store::open_in_memory().unwrap();
        let repo = git_repo();
        let first = ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Main).unwrap();
        let again = ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert_eq!(again.channel.workspace_mode, WorkspaceMode::MainCheckout, "a channel from before worktrees stays where it is");
        assert_eq!(again.session_id, first.session_id);
    }

    #[test]
    fn an_explicit_different_choice_moves_the_channel_with_a_new_session() {
        let store = Store::open_in_memory().unwrap();
        let repo = git_repo();
        let first = ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Main).unwrap();
        let moved = ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Worktree).unwrap();
        assert_eq!(moved.channel.id, first.channel.id);
        assert_eq!(moved.channel.workspace_mode, WorkspaceMode::Worktree);
        assert_ne!(moved.session_id, first.session_id);
        let same = ensure_workspace(&store, repo.path(), "claude", Role::Default, WorkspaceChoice::Worktree).unwrap();
        assert_eq!(same.session_id, moved.session_id, "asking for the mode the channel already has changes nothing");
    }

    #[test]
    fn a_root_directory_still_gets_a_usable_name() {
        let store = Store::open_in_memory().unwrap();
        let ws = ensure_workspace(&store, &PathBuf::from("/"), "claude", Role::Default, WorkspaceChoice::Auto).unwrap();
        assert!(!ws.project.name.is_empty());
    }
    #[test]
    fn an_archived_agent_is_not_selected_from_an_existing_project_channel() {
        let store=Store::open_in_memory().unwrap(); let dir=PathBuf::from("/work/archived");
        let first=ensure_workspace(&store,&dir,"claude",Role::Default,WorkspaceChoice::Main).unwrap();
        store.set_agent_archived(&first.channel.agent_id,true).unwrap();
        let next=ensure_workspace(&store,&dir,"claude",Role::Default,WorkspaceChoice::Main).unwrap();
        assert_ne!(first.channel.agent_id,next.channel.agent_id);
        assert_ne!(first.channel.id,next.channel.id);
        assert!(store.agent_archived(&first.channel.agent_id).unwrap());
        assert_eq!(store.conversations_for_agent(&first.channel.agent_id).unwrap().len(),2);
    }

}
