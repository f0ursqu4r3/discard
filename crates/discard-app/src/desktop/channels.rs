//! Real project channels. Git and storage work run off the webview thread.
use super::core::{AppCore, CoreInit};
use discard_core::{model::{Channel, ChannelId, ProjectId, WorkspaceMode}, store::{Store, WorktreeRecord}, ui_state::Update, worktree::{list_branch_tree, WorktreeManager}};
use std::{path::PathBuf, sync::{Arc, Mutex}};
use tokio::{runtime::Handle, sync::mpsc::UnboundedReceiver};

pub type Launcher = Arc<dyn Fn(&Channel) -> Result<(CoreInit, UnboundedReceiver<Update>), String> + Send + Sync>;
pub type OnReady = Arc<dyn Fn(&Arc<AppCore>) + Send + Sync>;

pub struct Channels {
    pub core: Arc<AppCore>,
    pub store: Arc<Mutex<Store>>,
    pub runtime: Handle,
    pub launch: Launcher,
    operation: Mutex<()>,
}
impl Channels {
    pub fn new(core: Arc<AppCore>, store: Arc<Mutex<Store>>, runtime: Handle, launch: Launcher) -> Self {
        Self { core, store, runtime, launch, operation: Mutex::new(()) }
    }
    fn project_dir(&self) -> PathBuf { self.core.current_project().path.into() }
    /// Caller holds the project-change guard. Prepare runtimes before replacing visible state.
    pub fn activate_project(&self, id: &ProjectId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let project = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let project = store.visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id == *id).ok_or("Project unavailable")?;
            store.select_project(id).map_err(|e| e.to_string())?;
            project
        };
        self.core.select_sidebar_project(Some(&project));
        Ok(())
    }
    pub fn restore(&self) -> Result<(), String> {
        if let Some(id) = self.core.saved_conversation() {
            if self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&ChannelId(id.clone())).is_ok() {
                self.select(&id)?;
            }
        }
        Ok(())
    }
    fn start(&self, channel: &Channel) -> Result<(), String> { self.start_mode(channel, false) }
    fn start_mode(&self, channel: &Channel, restart: bool) -> Result<(), String> {
        let (init, mut updates) = (self.launch)(channel)?;
        if restart { if !self.core.restart_closed_channel(init, channel)? { return Ok(()); } }
        else { self.core.add_channel(init, channel); }
        let (archived, unavailable) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            (store.channel_archived(&channel.id).map_err(|e| e.to_string())?, store.agent_archived(&channel.agent_id).unwrap_or(true))
        };
        self.core.channel_read_only(&channel.id.0, unavailable || archived, archived);
        let core = self.core.clone();
        let id = channel.id.0.clone();
        let generation = core.channel_runtime_generation(&id).ok_or("Conversation unavailable")?;
        self.runtime.spawn(async move {
            while let Some(update) = updates.recv().await {
                if core.apply_runtime(&id, Some(generation), update) {
                    let generation = core.request_branch_refresh();
                    let core = core.clone();
                    tokio::task::spawn_blocking(move || core.refresh_branches(generation));
                }
            }
        });
        Ok(())
    }
    pub fn select(&self, id: &str) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.select_inner(id)
    }
    fn select_inner(&self, id: &str) -> Result<(), String> { self.select_inner_mode(id, false) }
    fn select_inner_mode(&self, id: &str, project_change: bool) -> Result<(), String> {
        if project_change { self.core.can_switch_for_project()?; } else { self.core.can_switch()?; }
        let (channel, project) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let channel = store.load_channel(&ChannelId(id.to_owned())).map_err(|e| e.to_string())?;
            let project = channel.project_id.as_ref().and_then(|id| store.list_projects().ok()?.into_iter().find(|p| p.id == *id));
            (channel, project)
        };
        let restart = self.core.has_channel(id) && self.core.channel_session_closed(id);
        if !self.core.has_channel(id) { self.start(&channel)?; }
        if project_change { self.core.switch_channel_for_project(id)?; } else { self.core.switch_channel(id)?; }
        if let Some(project) = project { self.core.configure_branch_source(self.store.clone(), channel.id.clone(), channel.workspace_mode, project.path.into()); }
        else { self.core.clear_branch_source(); }
        self.core.clear_slots();
        self.core.refresh_branches(self.core.request_branch_refresh());
        if channel.project_id.is_none() {
            if let Err(error) = self.store.lock().unwrap_or_else(|e| e.into_inner()).add_direct_message(&channel.agent_id) {
                self.core.append_channel_notice(id, format!("Chat opened, but could not save it to Direct Messages: {error}"));
            }
        }
        // Selection succeeded. Report a restart failure in the retained transcript so
        // callers still open that history through the shared conversation-opened event.
        if restart && let Err(error) = self.start_mode(&channel, true) {
            self.core.append_channel_notice(id, format!("Conversation history is open, but its session could not restart: {error}. Select this conversation again to retry."));
        }
        Ok(())
    }
    pub fn reload_project(&self, id: &ProjectId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channels: Vec<_> = self.store.lock().unwrap_or_else(|e| e.into_inner()).all_conversations().map_err(|e| e.to_string())?.into_iter().filter(|c| c.project_id.as_ref() == Some(id) && self.core.has_channel(&c.id.0)).collect();
        self.core.invalidate_project_channels(id)?;
        for channel in channels { self.start_mode(&channel, true)?; }
        if self.core.active_context().1.as_ref() == Some(id) {
            self.select_inner_mode(&self.core.active_channel_id(), true)?;
        }
        Ok(())
    }
    pub fn restore_conversation(&self, id: &str) -> Result<(), String> {
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&ChannelId(id.to_owned())).map_err(|e| e.to_string())?;
        self.core.begin_agent_change(&channel.agent_id.0)?;
        let result = (|| {
            self.store.lock().unwrap_or_else(|e| e.into_inner()).restore_channel(&channel.id).map_err(|e| e.to_string())?;
            if self.core.agent_sessions_closed(&channel.agent_id.0) { self.reload_agent(&channel.agent_id.0) }
            else { self.core.channel_read_only(id, false, false); Ok(()) }
        })();
        self.core.end_agent_change(&channel.agent_id.0); result
    }
    pub fn reload_agent(&self, id: &str) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channels = self.core.agent_channel_ids(id);
        let (cfg, unavailable) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            (store.load_agent(&discard_core::model::AgentId(id.to_owned())).map_err(|e| e.to_string())?, store.agent_archived(&discard_core::model::AgentId(id.to_owned())).unwrap_or(true))
        };
        self.core.refresh_agent(id, &cfg, unavailable);
        for id in channels {
            let (channel, archived) = {
                let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                let channel = store.load_channel(&ChannelId(id.clone())).map_err(|e| e.to_string())?;
                let archived = store.channel_archived(&channel.id).map_err(|e| e.to_string())?;
                (channel, archived)
            };
            if self.core.channel_session_closed(&id) { self.start_mode(&channel, true)?; }
            self.core.channel_read_only(&id, unavailable || archived, archived);
        }
        Ok(())
    }
    pub fn open_agent(&self, agent: &discard_core::model::AgentId) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).default_conversation(agent).map_err(|e| e.to_string())?;
        self.select_inner(&channel.id.0)?; Ok(channel.id.0)
    }
    pub fn create_conversation(&self, agent: &discard_core::model::AgentId, project: Option<&ProjectId>, name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).create_conversation(agent, project, name).map_err(|e| e.to_string())?;
        self.select_inner(&channel.id.0)?; Ok(channel.id.0)
    }
    pub fn create(&self, name: &str, mode: &str, source_channel: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        if self.core.active_channel_id() != source_channel { return Err("The channel changed. Open New channel again.".into()); }
        if self.core.active_context().1.as_ref() != Some(&self.core.current_project().id) { return Err("Open a project conversation first.".into()); }
        let name = validate_name(name)?;
        let mode = WorkspaceMode::parse(mode).ok_or("Choose a workspace for this channel.")?;
        if mode == WorkspaceMode::Worktree && !WorktreeManager::is_repo(&self.project_dir()) { return Err("This project is not a Git repository. Use the project folder.".into()); }
        self.create_inner(name, mode, None, None)
    }
    fn create_inner(&self, name: String, mode: WorkspaceMode, branch: Option<String>, worktree: Option<PathBuf>) -> Result<String, String> {
        let channel = Channel { id: ChannelId::new(), project_id: Some(self.core.current_project().id), name, agent_id: discard_core::model::AgentId(self.core.active_agent_id()), workspace_mode: mode };
        let record = worktree.map(|path| -> Result<WorktreeRecord, String> {
            let manager = WorktreeManager::new(&self.project_dir());
            let prepared = manager.recorded(&self.project_dir(), &path, branch.as_deref().ok_or("Missing branch")?).map_err(|e| e.to_string())?;
            Ok(WorktreeRecord { channel_id: channel.id.clone(), path: path.to_string_lossy().into_owned(), branch: prepared.worktree.branch,
                base_commit: prepared.worktree.base_commit, repo: self.project_dir().to_string_lossy().into_owned(), owned: false })
        }).transpose()?;
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let existing = store.channels_for_project(&self.core.current_project().id).map_err(|e| e.to_string())?;
            if existing.iter().any(|c| c.name.eq_ignore_ascii_case(&channel.name)) { return Err("A channel with this name already exists.".into()); }
            store.create_named_channel(&channel, branch.as_deref(), record.as_ref()).map_err(|e| e.to_string())?;
        }
        self.start(&channel)?;
        self.select_inner(&channel.id.0)?;
        Ok(channel.id.0)
    }
    pub fn open_branch(&self, key: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        if self.core.active_context().1.as_ref() != Some(&self.core.current_project().id) { return Err("Open a conversation in this project first.".into()); }
        let row = list_branch_tree(&self.project_dir()).map_err(|e| e.to_string())?.into_iter().find(|r| r.name == key).ok_or("This branch is no longer available.")?;
        let existing = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let mut channels = store.channels_for_project(&self.core.current_project().id).map_err(|e| e.to_string())?;
            let current = self.core.active_channel_id();
            channels.sort_by_key(|c| c.id.0 == current);
            channels.into_iter().rev().find(|channel| {
                if channel.workspace_mode == WorkspaceMode::MainCheckout {
                    return row.worktree.as_ref().is_some_and(|path| self.project_dir().starts_with(path));
                }
                match store.worktree_for(&channel.id).ok().flatten() {
                    Some(record) => record.branch == key && row.worktree.as_ref().is_some_and(|path| path.canonicalize().is_ok_and(|path| PathBuf::from(&record.path).canonicalize().is_ok_and(|record_path| path == record_path))),
                    None => store.channel_branch(&channel.id).ok().flatten().as_deref() == Some(key),
                }
            })
        };
        if let Some(channel) = existing { self.select_inner(&channel.id.0)?; return Ok(channel.id.0); }
        let name = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let channels = store.channels_for_project(&self.core.current_project().id).map_err(|e| e.to_string())?;
            let mut name = key.to_string(); let mut suffix = 2;
            while channels.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) { name = format!("{key}-{suffix}"); suffix += 1; }
            name
        };
        self.create_inner(name, WorkspaceMode::Worktree, Some(key.into()), row.worktree)
    }
}

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().trim_start_matches('#').trim().to_lowercase();
    if name.is_empty() || name.chars().count() > 64 { return Err("Use a channel name with 1–64 characters.".into()); }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')) { return Err("Use letters, numbers, dashes, underscores, dots, or slashes.".into()); }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{agent::AgentConfig, capability::ApprovalPolicy, model::{AgentId, Project}, ui_state::UiState};
    use tokio::sync::mpsc;

    fn launcher() -> Launcher {
        Arc::new(|channel| {
            let (commands, _) = mpsc::unbounded_channel();
            let (_, updates) = mpsc::unbounded_channel();
            Ok((CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "Project".into(), channel: channel.name.clone(), agent: "Builder".into(), provider: "codex".into() }, updates))
        })
    }
    fn core(channel: &Channel, dir: &std::path::Path) -> Arc<AppCore> {
        let (init, _) = launcher()(channel).unwrap();
        let core = Arc::new(AppCore::new(init));
        core.configure_targets(channel.project_id.as_ref().expect("test channel has a project").0.clone(), dir, channel.agent_id.0.clone());
        core.configure_read_marker(dir, &channel.id.0);
        core
    }
    #[tokio::test]
    async fn projects_switch_in_place_and_import_never_changes_another_folder() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let first = store.import_project_folder(first_dir.path()).unwrap();
        let workspace = discard_core::bootstrap::ensure_workspace(&store, std::path::Path::new(&first.path), "claude", discard_core::bootstrap::Role::Default, discard_core::bootstrap::WorkspaceChoice::Auto).unwrap();
        let second = store.import_project_folder(second_dir.path()).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(store.list_projects().unwrap().iter().find(|p| p.id == first.id).unwrap().path, first.path);
        let core = core(&workspace.channel, first_dir.path());
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        core.begin_project_change().unwrap();
        service.activate_project(&second.id).unwrap();
        core.end_project_change();
        assert_eq!(core.current_project().path, second.path);
        assert_eq!(core.active_channel_id(), workspace.channel.id.0, "sidebar browsing preserves the active conversation");
        core.begin_project_change().unwrap();
        service.activate_project(&first.id).unwrap();
        core.end_project_change();
        assert_eq!(core.active_channel_id(), workspace.channel.id.0);
        assert_eq!(core.current_project().path, first.path);
        let store = store.lock().unwrap();
        store.hide_project(&second.id).unwrap();
        assert_eq!(store.visible_projects().unwrap().len(), 1);
        let channels = store.channels_for_project(&second.id).unwrap();
        assert!(channels.is_empty(), "opening an empty project never creates an implicit agent or conversation");
        assert_eq!(store.import_project_folder(second_dir.path()).unwrap().id, second.id);
        assert!(store.channels_for_project(&second.id).unwrap().is_empty());
        assert_eq!(store.visible_projects().unwrap().len(), 2);
        core.clear_project();
        assert!(core.current_project().id.0.is_empty());
    }

    #[tokio::test]
    async fn creation_switching_and_restore_use_saved_channels_and_distinct_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels.db");
        let store = Store::open(&path).unwrap();
        let project = Project { id: ProjectId::new(), name: "Project".into(), path: dir.path().to_string_lossy().into_owned() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store.insert_agent(&agent, &AgentConfig { name: "Builder".into(), provider: "codex".into(), model: Some("agent-model".into()), avatar: None, system_prompt: "Review changes".into(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        store.create_named_channel(&first, None, None).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        let created = service.create("#Tests", "main_checkout", &first.id.0).unwrap();
        assert_eq!(core.active_channel_id(), created);
        assert_eq!(core.snapshot().channel, "tests");
        assert!(service.create("TESTS", "main_checkout", &created).unwrap_err().contains("already exists"));
        assert!(service.create("space name", "main_checkout", &created).is_err());
        assert!(service.create("old-ui", "main_checkout", &first.id.0).is_err());
        assert!(service.create("isolated", "worktree", &created).is_err(), "non-Git projects must not pretend to create worktrees");
        service.select(&first.id.0).unwrap();
        assert_eq!(core.snapshot().channel, "general");
        let reopened = Store::open(&path).unwrap();
        let rows = reopened.channels_for_project(&project.id).unwrap();
        assert_eq!(rows.len(), 2);
        let second = rows.iter().find(|row| row.id.0 == created).unwrap();
        assert_eq!(second.agent_id, agent);
        assert_eq!(reopened.load_agent(&agent).unwrap().model.as_deref(), Some("agent-model"));
        assert_ne!(reopened.latest_session(&first.id).unwrap(), reopened.latest_session(&second.id).unwrap());
        let restored = tests::core(&first, dir.path());
        let service = Channels::new(restored.clone(), Arc::new(Mutex::new(reopened)), Handle::current(), launcher());
        service.restore().unwrap();
        service.select(&created).unwrap();
        assert_eq!(restored.snapshot().channel, "tests");
    }

    #[tokio::test]
    async fn externally_switched_worktree_gets_new_channel_for_each_branch() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"]).args(args).current_dir(dir.path()).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "-q", "-b", "old"]);
        git(&["commit", "--allow-empty", "-m", "initial"]);
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "p".into(), path: dir.path().to_string_lossy().into_owned() };
        let agent = AgentId::new();
        store.insert_project(&project).unwrap();
        store.insert_agent(&agent, &AgentConfig { name: "Builder".into(), provider: "codex".into(), model: None, avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "old".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
        let manager = WorktreeManager::new(dir.path());
        let prepared = manager.recorded(dir.path(), dir.path(), "old").unwrap();
        store.create_named_channel(&first, Some("old"), Some(&WorktreeRecord { channel_id: first.id.clone(), path: dir.path().to_string_lossy().into_owned(), branch: "old".into(), base_commit: prepared.worktree.base_commit, repo: project.path.clone(), owned: false })).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        git(&["checkout", "-b", "new"]);
        let new = service.open_branch("new").unwrap();
        assert_ne!(new, first.id.0, "the changed checkout must not reuse the old session");
        assert_eq!(service.open_branch("new").unwrap(), new, "a compatible binding is reused");
        let old = service.open_branch("old").unwrap();
        assert_ne!(old, first.id.0, "the old branch needs a fresh checkout, not the moved directory");
        assert_eq!(store.lock().unwrap().channel_branch(&ChannelId(old)).unwrap().as_deref(), Some("old"));
    }
}
