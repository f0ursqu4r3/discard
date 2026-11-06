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
    pub fn activate_project(&self, id: &ProjectId) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch_for_project()?;
        let (project, channel) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let project = store.visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id == *id).ok_or("Project unavailable")?;
            store.select_project(id).map_err(|e| e.to_string())?;
            let main = match store.project_main_agent(id).map_err(|e| e.to_string())? {
                Some(main) => main,
                None => store.list_agents().map_err(|e| e.to_string())?.into_iter()
                    .find(|(_, config)| config.model.as_ref().is_some_and(|model| !model.trim().is_empty()))
                    .map(|(agent, _)| agent)
                    .ok_or("No configured agent with a model is available. Configure an agent in Settings, then open this project again.")?,
            };
            store.ensure_project_general_channel(id, &main).map_err(|e| e.to_string())?;
            let channel = store.project_channel_or_fallback(id, &main).map_err(|e| e.to_string())?;
            (project, channel)
        };
        self.core.select_sidebar_project(Some(&project));
        self.select_inner_mode(&channel.id.0, true)?;
        self.store.lock().unwrap_or_else(|e| e.into_inner()).select_project_channel(id, &channel.id).map_err(|e| e.to_string())?;
        self.core.refresh_branches(self.core.request_branch_refresh());
        Ok(channel.id.0)
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
    fn project_channel(&self, project: &ProjectId, channel: &ChannelId, allow_archived: bool) -> Result<(Channel, discard_core::model::ProjectChannelMetadata, bool), String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let value = store.load_channel(channel).map_err(|e| e.to_string())?;
        if value.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
        let metadata = store.project_channel_metadata(channel).map_err(|e| e.to_string())?.ok_or("Project channel metadata is unavailable")?;
        let archived = store.channel_archived(channel).map_err(|e| e.to_string())?;
        if archived && !allow_archived { return Err("This channel is archived.".into()); }
        Ok((value, metadata, archived))
    }
    pub fn list_project_channels(&self, project: &ProjectId) -> Result<Vec<super::dto::ProjectChannelDto>, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if !store.list_projects().map_err(|e| e.to_string())?.iter().any(|candidate| candidate.id == *project) { return Err("Project unavailable".into()); }
        let rows = store.all_channels_for_project(project).map_err(|e| e.to_string())?;
        rows.into_iter().map(|(channel, archived)| {
            let metadata = store.project_channel_metadata(&channel.id).map_err(|e| e.to_string())?
                .ok_or("Project channel metadata is unavailable")?;
            Ok(super::dto::ProjectChannelDto {
                id: channel.id.0, project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: match metadata.kind { discard_core::model::ProjectChannelKind::General => "general", discard_core::model::ProjectChannelKind::Branch => "branch", discard_core::model::ProjectChannelKind::ArchivedLegacy => "archived_legacy" }.into(),
                branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
            })
        }).collect()
    }
    pub fn list_project_members(&self, project: &ProjectId) -> Result<Vec<super::dto::ChannelMemberDto>, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if !store.list_projects().map_err(|e| e.to_string())?.iter().any(|candidate| candidate.id == *project) { return Err("Project unavailable".into()); }
        let members = store.project_agents(project).map_err(|e| e.to_string())?.into_iter().filter_map(|agent| {
            let config = store.load_agent(&agent).ok()?;
            Some(super::dto::ChannelMemberDto { id: agent.0.clone(), name: config.name, provider: config.provider, archived: store.agent_archived(&agent).unwrap_or(true) })
        }).collect::<Vec<_>>();
        Ok(members)
    }
    pub fn invite_channel_member(&self, project: &ProjectId, channel: &ChannelId, agent: &discard_core::model::AgentId) -> Result<(), String> {
        let (_, _, _) = self.project_channel(project, channel, false)?;
        self.store.lock().unwrap_or_else(|e| e.into_inner()).invite_agent_to_channel(project, channel, agent).map_err(|e| e.to_string())
    }
    pub fn select_canonical_branch(&self, project: &ProjectId, branch: &str, channel_id: &ChannelId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (target, _, _) = self.project_channel(project, channel_id, false)?;
        if target.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
        let previous = { self.store.lock().unwrap_or_else(|e| e.into_inner()).project_branch_channel(project, branch).map_err(|e| e.to_string())? };
        let mut guarded = vec![target.id.0.clone()];
        if let Some(previous) = previous.as_ref().filter(|previous| previous.id != target.id) { guarded.push(previous.id.0.clone()); }
        guarded.sort();
        let mut begun: Vec<String> = Vec::new();
        for id in &guarded {
            if let Err(error) = self.core.begin_channel_change(id) {
                for id in begun { self.core.end_channel_change(&id); }
                return Err(error);
            }
            begun.push(id.clone());
        }
        let result = (|| {
            self.store.lock().unwrap_or_else(|e| e.into_inner()).set_canonical_project_branch(project, branch, channel_id).map_err(|e| e.to_string())?;
            if let Some(previous) = previous.as_ref() {
                if previous.id != target.id { self.core.channel_read_only(&previous.id.0, true, true); }
            }
            self.core.channel_read_only(&target.id.0, false, false);
            if self.core.has_channel(&target.id.0) && self.core.channel_session_closed(&target.id.0) { self.start_mode(&target, true)?; }
            if previous.as_ref().is_some_and(|old| old.id != target.id && self.core.active_channel_id() == old.id.0) {
                if !self.core.has_channel(&target.id.0) { self.start(&target)?; }
                self.select_inner(&target.id.0)?;
            }
            Ok(())
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    pub fn select_canonical_general(&self, project: &ProjectId, channel_id: &ChannelId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (target, metadata, _) = self.project_channel(project, channel_id, true)?;
        if !target.name.eq_ignore_ascii_case("general") || metadata.kind == discard_core::model::ProjectChannelKind::Branch {
            return Err("Select a general channel from this project.".into());
        }
        let previous = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            store.all_channels_for_project(project).map_err(|e| e.to_string())?.into_iter().find_map(|(channel, archived)| {
                (!archived && store.project_channel_metadata(&channel.id).ok().flatten().is_some_and(|m| m.kind == discard_core::model::ProjectChannelKind::General)).then_some(channel)
            })
        };
        let mut guarded: Vec<String> = vec![target.id.0.clone()];
        if let Some(old) = previous.as_ref().filter(|old| old.id != target.id) { guarded.push(old.id.0.clone()); }
        guarded.sort();
        let mut begun: Vec<String> = Vec::new();
        for id in &guarded {
            if let Err(error) = self.core.begin_channel_change(id) {
                for id in begun { self.core.end_channel_change(&id); }
                return Err(error);
            }
            begun.push(id.clone());
        }
        let result = (|| {
            self.store.lock().unwrap_or_else(|e| e.into_inner()).set_canonical_project_channel(project, channel_id).map_err(|e| e.to_string())?;
            let updated = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(channel_id).map_err(|e| e.to_string())?;
            if let Some(old) = previous.as_ref().filter(|old| old.id != updated.id) { self.core.channel_read_only(&old.id.0, true, true); }
            if self.core.has_channel(&updated.id.0) {
                self.core.invalidate_idle_channel(&updated)?;
                self.start_mode(&updated, true)?;
            } else { self.start(&updated)?; }
            self.core.channel_read_only(&updated.id.0, false, false);
            if previous.as_ref().is_some_and(|old| old.id != updated.id && self.core.active_channel_id() == old.id.0) {
                self.select_inner(&updated.id.0)?;
            }
            Ok(())
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    pub fn channel_details(&self, project: &ProjectId, channel: &ChannelId) -> Result<super::dto::ChannelDetailsDto, String> {
        let (channel, metadata, archived) = self.project_channel(project, channel, true)?;
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let topic = store.channel_topic(&channel.id).map_err(|e| e.to_string())?;
        let topic_history = store.channel_topic_history(&channel.id).map_err(|e| e.to_string())?.into_iter().map(|revision| super::dto::ChannelTopicRevisionDto {
            channel_id: revision.channel_id.0, text: revision.text, version: revision.version,
            author_kind: match revision.author_kind { discard_core::model::TopicAuthorKind::User => "user", discard_core::model::TopicAuthorKind::Agent => "agent" }.into(),
            author_id: revision.author_id.map(|id| id.0),
        }).collect();
        let members = store.channel_agents(&channel.id).map_err(|e| e.to_string())?.into_iter().filter_map(|agent| {
            let config = store.load_agent(&agent).ok()?;
            Some(super::dto::ChannelMemberDto { id: agent.0.clone(), name: config.name, provider: config.provider, archived: store.agent_archived(&agent).unwrap_or(true) })
        }).collect();
        let main_agent_id = store.project_main_agent(project).map_err(|e| e.to_string())?.map(|id| id.0);
        Ok(super::dto::ChannelDetailsDto {
            channel: super::dto::ProjectChannelDto {
                id: channel.id.0.clone(), project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: match metadata.kind { discard_core::model::ProjectChannelKind::General => "general", discard_core::model::ProjectChannelKind::Branch => "branch", discard_core::model::ProjectChannelKind::ArchivedLegacy => "archived_legacy" }.into(),
                branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
            },
            topic: super::dto::ChannelTopicDto { channel_id: topic.channel_id.0, text: topic.text, version: topic.version },
            topic_history, members, main_agent_id,
        })
    }
    pub fn update_channel_topic(&self, project: &ProjectId, channel: &ChannelId, expected_version: u64, text: &str) -> Result<super::dto::ChannelTopicDto, String> {
        let (_, _, _) = self.project_channel(project, channel, false)?;
        let topic = self.store.lock().unwrap_or_else(|e| e.into_inner()).save_channel_topic(channel, expected_version, text, discard_core::model::TopicAuthorKind::User, None).map_err(|e| e.to_string())?;
        Ok(super::dto::ChannelTopicDto { channel_id: topic.channel_id.0, text: topic.text, version: topic.version })
    }
    pub fn change_project_main_agent(&self, project: &ProjectId, agent: &discard_core::model::AgentId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channel = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let mut general = None;
            for (candidate, archived) in store.all_channels_for_project(project).map_err(|e| e.to_string())? {
                if !archived && store.project_channel_metadata(&candidate.id).map_err(|e| e.to_string())?.is_some_and(|m| m.kind == discard_core::model::ProjectChannelKind::General) { general = Some(candidate); break; }
            }
            if let Some(general) = general { general }
            else { store.ensure_project_general_channel(project, agent).map_err(|e| e.to_string())? }
        };
        self.core.begin_channel_change(&channel.id.0)?;
        let result = (|| {
            let updated = {
                let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                store.set_project_main_agent(project, agent).map_err(|e| e.to_string())?;
                store.ensure_project_general_channel(project, agent).map_err(|e| e.to_string())?
            };
            if self.core.has_channel(&updated.id.0) {
                self.core.invalidate_idle_channel(&updated)?;
                if let Err(error) = self.start_mode(&updated, true) {
                    self.core.append_channel_notice(&updated.id.0, format!("The project main agent changed, but its channel session could not restart: {error}. Select this channel again to retry."));
                    return Err(error);
                }
            } else { self.start(&updated)?; }
            Ok(())
        })();
        self.core.end_channel_change(&channel.id.0);
        result
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
        } else if let Some(project_id) = channel.project_id.as_ref()
            && !self.store.lock().unwrap_or_else(|e| e.into_inner()).channel_archived(&channel.id).map_err(|e| e.to_string())?
        {
            self.store.lock().unwrap_or_else(|e| e.into_inner()).select_project_channel(project_id, &channel.id).map_err(|e| e.to_string())?;
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
        if let Some(project) = project {
            if name.trim().to_lowercase() != "general" {
                return Err("Project conversations use the project's general channel. Open a Git branch to start another channel.".into());
            }
            let channel = {
                let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                let main = store.project_main_agent(project).map_err(|e| e.to_string())?
                    .ok_or("This project has no main agent. Open the project first.")?;
                if &main != agent { return Err("Choose the project's main agent to open its general channel.".into()); }
                store.ensure_project_general_channel(project, &main).map_err(|e| e.to_string())?
            };
            self.select_inner(&channel.id.0)?;
            return Ok(channel.id.0);
        }
        self.core.can_switch()?;
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).create_conversation(agent, project, name).map_err(|e| e.to_string())?;
        self.select_inner(&channel.id.0)?; Ok(channel.id.0)
    }
    pub fn create(&self, name: &str, mode: &str, source_channel: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        if self.core.active_channel_id() != source_channel { return Err("The channel changed. Open New channel again.".into()); }
        if self.core.current_project().id.0.is_empty() { return Err("Select a project first.".into()); }
        let _ = (name, mode);
        Err("Open a Git branch to start another project channel.".into())
    }
    fn create_inner(&self, name: String, mode: WorkspaceMode, branch: Option<String>, worktree: Option<PathBuf>) -> Result<String, String> {
        let project = self.core.current_project().id;
        let agent_id = self.store.lock().unwrap_or_else(|e| e.into_inner()).project_main_agent(&project).map_err(|e| e.to_string())?
            .ok_or("This project has no main agent. Open the project first.")?;
        let channel = Channel { id: ChannelId::new(), project_id: Some(project), name, agent_id, workspace_mode: mode };
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
            if branch.is_some() {
                let project = channel.project_id.as_ref().ok_or("Project channel is missing its project")?;
                store.invite_agent_to_channel(project, &channel.id, &channel.agent_id).map_err(|e| e.to_string())?;
            }
        }
        self.start(&channel)?;
        self.select_inner(&channel.id.0)?;
        Ok(channel.id.0)
    }
    pub fn open_branch(&self, key: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        let project_id = self.core.current_project().id;
        if project_id.0.is_empty() { return Err("Select a project first.".into()); }
        let row = list_branch_tree(&self.project_dir()).map_err(|e| e.to_string())?.into_iter().find(|r| r.name == key).ok_or("This branch is no longer available.")?;
        let canonical = { self.store.lock().unwrap_or_else(|e| e.into_inner()).project_branch_channel(&project_id, key).map_err(|e| e.to_string())? };
        if let Some(channel) = canonical {
            let main = self.store.lock().unwrap_or_else(|e| e.into_inner()).project_main_agent(&project_id).map_err(|e| e.to_string())?.ok_or("This project has no main agent. Open the project first.")?;
            self.store.lock().unwrap_or_else(|e| e.into_inner()).invite_agent_to_channel(&project_id, &channel.id, &main).map_err(|e| e.to_string())?;
            if !self.core.has_channel(&channel.id.0) { self.start(&channel)?; }
            self.select_inner(&channel.id.0)?;
            return Ok(channel.id.0);
        }
        let name = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let channels = store.channels_for_project(&project_id).map_err(|e| e.to_string())?;
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

    fn configured_agent(store: &Store) -> AgentId {
        store.create_agent(&AgentConfig { reasoning_effort: None, name: "Project agent".into(), provider: "codex".into(), model: Some("model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap()
    }

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
        let agent = configured_agent(&store);
        store.set_project_main_agent(&first.id, &agent).unwrap();
        store.set_project_main_agent(&second.id, &agent).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(store.list_projects().unwrap().iter().find(|p| p.id == first.id).unwrap().path, first.path);
        let core = core(&workspace.channel, first_dir.path());
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        core.begin_project_change().unwrap();
        let second_general = service.activate_project(&second.id).unwrap();
        core.end_project_change();
        assert_eq!(core.current_project().path, second.path);
        assert_eq!(core.active_channel_id(), second_general, "project activation opens its general channel");
        let direct = service.create_conversation(&agent, None, "Direct chat").unwrap();
        let saved_second = store.lock().unwrap().last_project_channel(&second.id).unwrap().unwrap().id;
        service.select(&direct).unwrap();
        assert_eq!(store.lock().unwrap().last_project_channel(&second.id).unwrap().unwrap().id, saved_second, "direct chat selection does not change the saved project channel");
        core.begin_project_change().unwrap();
        let restored_first = service.activate_project(&first.id).unwrap();
        core.end_project_change();
        assert_eq!(restored_first, workspace.channel.id.0, "returning to A restores its saved channel");
        assert_eq!(core.active_channel_id(), workspace.channel.id.0);
        assert_eq!(core.current_project().path, first.path);
        let store = store.lock().unwrap();
        store.hide_project(&second.id).unwrap();
        assert_eq!(store.visible_projects().unwrap().len(), 1);
        let channels = store.channels_for_project(&second.id).unwrap();
        assert_eq!(channels.len(), 1, "project activation creates only the general channel");
        assert_eq!(store.import_project_folder(second_dir.path()).unwrap().id, second.id);
        assert_eq!(store.channels_for_project(&second.id).unwrap().len(), 1);
        assert_eq!(store.visible_projects().unwrap().len(), 2);
        core.clear_project();
        assert!(core.current_project().id.0.is_empty());
    }

    #[tokio::test]
    async fn sidebar_branches_follow_the_project_without_a_conversation() {
        let repo = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["commit", "--allow-empty", "-m", "initial"],
            vec!["branch", "feature"],
        ] {
            let output = std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
                .args(args).current_dir(repo.path()).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(repo.path()).unwrap();
        let other_project = store.import_project_folder(other.path()).unwrap();
        configured_agent(&store);
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit {
            state: UiState::new(ChannelId(String::new()), vec![]), commands,
            project: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
        }));
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        service.activate_project(&project.id).unwrap();
        let snapshot = core.snapshot();
        assert_eq!(snapshot.branches.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), vec!["main", "feature"]);
        assert_eq!(snapshot.channel, "general");
        assert_eq!(store.lock().unwrap().channels_for_project(&project.id).unwrap().len(), 1);

        // A conversation's branch source must not override the browsed project.
        core.configure_branch_source(store, ChannelId("elsewhere".into()), WorkspaceMode::MainCheckout, other.path().into());
        core.refresh_branches(core.request_branch_refresh());
        assert_eq!(core.snapshot().branches.len(), 2);
        assert!(core.snapshot().branches.iter().all(|row| !row.current));
        let stale = core.request_branch_refresh();
        service.activate_project(&other_project.id).unwrap();
        core.refresh_branches(stale);
        assert!(core.snapshot().branches.is_empty());
        service.activate_project(&project.id).unwrap();
        assert_eq!(core.snapshot().branches.len(), 2);
        assert!(core.snapshot().branches.iter().all(|row| !row.current), "the general channel does not highlight the main checkout");
        service.open_branch("feature").unwrap();
        assert!(core.snapshot().branches.iter().any(|row| row.key == "feature" && row.current), "canonical branch metadata highlights its branch without a worktree");
        core.select_sidebar_project(None);
        core.refresh_branches(core.request_branch_refresh());
        assert!(core.snapshot().branches.is_empty());
    }

    #[tokio::test]
    async fn creation_switching_and_restore_use_saved_channels_and_distinct_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels.db");
        let store = Store::open(&path).unwrap();
        let project = Project { id: ProjectId::new(), name: "Project".into(), path: dir.path().to_string_lossy().into_owned() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store.insert_agent(&agent, &AgentConfig { reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: Some("agent-model".into()), avatar: None, system_prompt: "Review changes".into(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        store.create_named_channel(&first, None, None).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        let created = service.activate_project(&project.id).unwrap();
        assert_eq!(created, first.id.0);
        assert_eq!(core.snapshot().channel, "general");
        assert!(service.create("Tests", "main_checkout", &created).unwrap_err().contains("Open a Git branch"));
        assert!(service.create("old-ui", "main_checkout", &first.id.0).unwrap_err().contains("Open a Git branch"));
        service.select(&first.id.0).unwrap();
        assert_eq!(core.snapshot().channel, "general");
        let reopened = Store::open(&path).unwrap();
        let rows = reopened.channels_for_project(&project.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(reopened.load_agent(&agent).unwrap().model.as_deref(), Some("agent-model"));
        let restored = tests::core(&first, dir.path());
        let service = Channels::new(restored.clone(), Arc::new(Mutex::new(reopened)), Handle::current(), launcher());
        service.restore().unwrap();
        service.select(&created).unwrap();
        assert_eq!(restored.snapshot().channel, "general");
    }

    #[tokio::test]
    async fn canonical_branch_channels_reuse_saved_identity_with_an_unrelated_active_agent() {
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
        store.insert_agent(&agent, &AgentConfig { reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: None, avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "old".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
        let manager = WorktreeManager::new(dir.path());
        let prepared = manager.recorded(dir.path(), dir.path(), "old").unwrap();
        store.create_named_channel(&first, Some("old"), Some(&WorktreeRecord { channel_id: first.id.clone(), path: dir.path().to_string_lossy().into_owned(), branch: "old".into(), base_commit: prepared.worktree.base_commit, repo: project.path.clone(), owned: false })).unwrap();
        store.set_project_main_agent(&project.id, &first.agent_id).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        service.activate_project(&project.id).unwrap();
        let direct_agent = store.lock().unwrap().create_agent(&AgentConfig { reasoning_effort: None, name: "Direct".into(), provider: "codex".into(), model: Some("direct-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        service.open_agent(&direct_agent).unwrap();
        git(&["checkout", "-b", "new"]);
        let new = service.open_branch("new").unwrap();
        assert_ne!(new, first.id.0, "the changed checkout must not reuse the old session");
        assert_eq!(store.lock().unwrap().load_channel(&ChannelId(new.clone())).unwrap().agent_id, first.agent_id, "branch channels use the project main agent");
        assert!(store.lock().unwrap().channel_agents(&ChannelId(new.clone())).unwrap().contains(&first.agent_id));
        service.open_agent(&direct_agent).unwrap();
        assert_eq!(service.open_branch("new").unwrap(), new, "a compatible binding is reused");
        let old = service.open_branch("old").unwrap();
        assert_eq!(old, first.id.0, "the canonical branch channel is reused even when another direct agent was active");
        assert_eq!(store.lock().unwrap().channel_branch(&ChannelId(old)).unwrap().as_deref(), Some("old"));
    }

    #[tokio::test]
    async fn project_activation_fails_without_a_configured_agent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit { state: UiState::new(ChannelId(String::new()), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() }));
        let service = Channels::new(core, Arc::new(Mutex::new(store)), Handle::current(), launcher());
        let error = service.activate_project(&project.id).unwrap_err();
        assert!(error.contains("Configure an agent"));
        assert!(service.store.lock().unwrap().channels_for_project(&project.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn topic_api_binds_project_and_channel_and_rejects_stale_user_edits() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store);
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        let core = core(&general, dir.path());
        let service = Channels::new(core, Arc::new(Mutex::new(store)), Handle::current(), launcher());
        let first = service.update_channel_topic(&project.id, &general.id, 0, "Ship the first release").unwrap();
        assert_eq!(first.version, 1);
        let details = service.channel_details(&project.id, &general.id).unwrap();
        assert_eq!(details.topic.text, "Ship the first release");
        assert_eq!(details.topic_history[0].author_kind, "user");
        assert!(service.update_channel_topic(&project.id, &general.id, 0, "stale edit").unwrap_err().contains("Expected version 0"));
        let other_dir = tempfile::tempdir().unwrap();
        let other = service.store.lock().unwrap().import_project_folder(other_dir.path()).unwrap();
        assert!(service.channel_details(&other.id, &general.id).unwrap_err().contains("does not belong"));
    }

    #[tokio::test]
    async fn failed_main_agent_restart_closes_old_transport_and_selection_retries() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        let old_agent = configured_agent(&store.lock().unwrap());
        let new_agent = store.lock().unwrap().create_agent(&AgentConfig { reasoning_effort: None, name: "New main".into(), provider: "claude".into(), model: Some("new-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        store.lock().unwrap().set_project_main_agent(&project.id, &old_agent).unwrap();
        let general = store.lock().unwrap().ensure_project_general_channel(&project.id, &old_agent).unwrap();
        let retry_receivers = Arc::new(Mutex::new(Vec::new()));
        let retries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let launcher: Launcher = {
            let receivers = retry_receivers.clone(); let retries = retries.clone(); let store = store.clone();
            Arc::new(move |channel| {
                if retries.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 { return Err("simulated launcher failure".into()); }
                let cfg = store.lock().unwrap().load_agent(&channel.agent_id).unwrap();
                let (commands, receiver) = mpsc::unbounded_channel(); receivers.lock().unwrap().push(receiver);
                let (_, updates) = mpsc::unbounded_channel();
                Ok((CoreInit { state: UiState::new(channel.id.clone(), store.lock().unwrap().messages_for_channel(&channel.id).unwrap()), commands, project: project.name.clone(), channel: channel.name.clone(), agent: cfg.name, provider: cfg.provider }, updates))
            })
        };
        let (init, _) = launcher(&general).unwrap();
        let core = Arc::new(AppCore::new(init));
        core.configure_targets(project.id.0.clone(), dir.path(), old_agent.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher);
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("simulated launcher failure"));
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, new_agent);
        assert!(core.snapshot().notice.unwrap_or_default().contains("Select this channel again to retry"));
        assert!(core.send_to(Some(&general.id.0), "old agent must not accept this".into()).is_err());
        service.select(&general.id.0).unwrap();
        assert_eq!(core.active_context().0, new_agent);
        assert!(!core.send_to(Some(&general.id.0), "new agent is ready".into()).is_err());
    }
}
