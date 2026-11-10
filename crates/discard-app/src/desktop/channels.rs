//! Real project channels. Git and storage work run off the webview thread.
use discard_core::model::AgentId;
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
        if let Some(id) = self.core.saved_conversation()
            && self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&ChannelId(id.clone())).is_ok() {
                self.select(&id)?;
            }
        Ok(())
    }
    fn start(&self, channel: &Channel) -> Result<(), String> { self.start_mode(channel, false) }
    fn refresh_channel_access(&self, channel: &Channel) -> Result<(), String> {
        let (archived, unavailable) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            (store.channel_archived(&channel.id).map_err(|e| e.to_string())?, store.agent_archived(&channel.agent_id).unwrap_or(true))
        };
        self.core.channel_read_only(&channel.id.0, archived || unavailable, archived);
        Ok(())
    }
    fn start_mode(&self, channel: &Channel, restart: bool) -> Result<(), String> {
        let (init, mut updates) = (self.launch)(channel)?;
        if restart { if !self.core.restart_closed_channel(init, channel)? { return Ok(()); } }
        else { self.core.add_channel(init, channel); }
        self.refresh_channel_access(channel)?;
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
    fn with_active_project_channel<T>(&self, project: &ProjectId, channel: &ChannelId, operation: impl FnOnce(&Store) -> Result<T, String>) -> Result<T, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let value = store.load_channel(channel).map_err(|e| e.to_string())?;
        if value.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
        if store.project_channel_metadata(channel).map_err(|e| e.to_string())?.is_none() { return Err("Project channel metadata is unavailable".into()); }
        if store.channel_archived(channel).map_err(|e| e.to_string())? { return Err("This channel is archived.".into()); }
        operation(&store)
    }
    pub fn list_project_channels(&self, project: &ProjectId) -> Result<Vec<super::dto::ProjectChannelDto>, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if !store.list_projects().map_err(|e| e.to_string())?.iter().any(|candidate| candidate.id == *project) { return Err("Project unavailable".into()); }
        let rows = store.all_channels_for_project(project).map_err(|e| e.to_string())?;
        rows.into_iter().map(|(channel, archived)| {
            let metadata = store.project_channel_metadata(&channel.id).map_err(|e| e.to_string())?
                .ok_or("Project channel metadata is unavailable")?;
            let worktree = store.worktree_for(&channel.id).map_err(|e| e.to_string())?
                .map(|record| super::dto::ChannelWorktreeDto { path: record.path, branch: record.branch });
            Ok(super::dto::ProjectChannelDto {
                worktree,
                id: channel.id.0, project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: match metadata.kind { discard_core::model::ProjectChannelKind::General => "general", discard_core::model::ProjectChannelKind::Branch => "branch", discard_core::model::ProjectChannelKind::ArchivedLegacy => "archived_legacy", discard_core::model::ProjectChannelKind::Legacy => "legacy" }.into(),
                repository: metadata.repository, branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
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
        self.with_active_project_channel(project, channel, |store| store.invite_agent_to_channel(project, channel, agent).map_err(|e| e.to_string()))
    }
    pub fn select_canonical_branch(&self, project: &ProjectId, branch: &str, channel_id: &ChannelId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (target, metadata, _) = self.project_channel(project, channel_id, true)?;
        if target.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
        let previous = { self.store.lock().unwrap_or_else(|e| e.into_inner()).project_repository_branch_channel(project, &metadata.repository, branch).map_err(|e| e.to_string())? };
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
            let updated = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(channel_id).map_err(|e| e.to_string())?;
            if let Some(previous) = previous.as_ref()
                && previous.id != target.id { self.core.channel_read_only(&previous.id.0, true, true); }
            if self.core.has_channel(&updated.id.0) && self.core.channel_session_closed(&updated.id.0) { self.start_mode(&updated, true)?; }
            else { self.refresh_channel_access(&updated)?; }
            if previous.as_ref().is_some_and(|old| old.id != target.id && self.core.active_channel_id() == old.id.0) {
                if !self.core.has_channel(&updated.id.0) { self.start(&updated)?; }
                self.select_inner(&updated.id.0)?;
            }
            self.core.refresh_branches(self.core.request_branch_refresh());
            Ok(())
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    pub fn select_canonical_general(&self, project: &ProjectId, channel_id: &ChannelId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (target, metadata, _) = self.project_channel(project, channel_id, true)?;
        if !target.name.eq_ignore_ascii_case("general") || metadata.kind != discard_core::model::ProjectChannelKind::ArchivedLegacy {
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
                if let Err(error) = self.start_mode(&updated, true) {
                    self.core.append_channel_notice(&updated.id.0, format!("The canonical general channel changed, but its session could not restart: {error}. Select this channel again to retry."));
                    return Err(error);
                }
            } else { self.start(&updated)?; }
            self.refresh_channel_access(&updated)?;
            if previous.as_ref().is_some_and(|old| old.id != updated.id && self.core.active_channel_id() == old.id.0) {
                self.select_inner(&updated.id.0)?;
            }
            self.core.refresh_branches(self.core.request_branch_refresh());
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
                worktree: store.worktree_for(&channel.id).map_err(|e| e.to_string())?
                    .map(|record| super::dto::ChannelWorktreeDto { path: record.path, branch: record.branch }),
                id: channel.id.0.clone(), project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: match metadata.kind { discard_core::model::ProjectChannelKind::General => "general", discard_core::model::ProjectChannelKind::Branch => "branch", discard_core::model::ProjectChannelKind::ArchivedLegacy => "archived_legacy", discard_core::model::ProjectChannelKind::Legacy => "legacy" }.into(),
                repository: metadata.repository, branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
            },
            topic: super::dto::ChannelTopicDto { channel_id: topic.channel_id.0, text: topic.text, version: topic.version },
            topic_history, members, main_agent_id,
        })
    }
    pub fn update_channel_topic(&self, project: &ProjectId, channel: &ChannelId, expected_version: u64, text: &str) -> Result<super::dto::ChannelTopicDto, String> {
        let topic = self.with_active_project_channel(project, channel, |store| store.save_channel_topic(channel, expected_version, text, discard_core::model::TopicAuthorKind::User, None).map_err(|e| e.to_string()))?;
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
            self.store.lock().unwrap_or_else(|e| e.into_inner()).set_project_main_agent(project, agent).map_err(|e| e.to_string())?;
            let mut updated_identity = channel.clone();
            updated_identity.agent_id = agent.clone();
            if self.core.has_channel(&updated_identity.id.0) {
                self.core.invalidate_idle_channel(&updated_identity)?;
            }
            let updated = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&updated_identity.id).map_err(|e| e.to_string())?;
            if self.core.has_channel(&updated.id.0) {
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
    /// Restart loaded project channels that Store reassigned after an agent was deleted.
    pub fn reconcile_deleted_agent(&self, deleted_agent_id: &str) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let ids = self.core.agent_channel_ids(deleted_agent_id);
        let mut failures = Vec::new();
        for id in ids {
            let channel = {
                let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                match store.load_channel(&ChannelId(id.clone())) {
                    Ok(channel) if channel.agent_id.0 != deleted_agent_id => channel,
                    _ => continue,
                }
            };
            if let Err(error) = self.core.invalidate_idle_channel(&channel) {
                failures.push(format!("{id}: {error}"));
                continue;
            }
            if let Err(error) = self.start_mode(&channel, true) {
                self.core.append_channel_notice(&id, format!("This channel now uses its saved replacement agent, but its session could not restart: {error}. Select this channel again to retry."));
                failures.push(format!("{id}: {error}"));
            }
        }
        self.core.refresh_branches(self.core.request_branch_refresh());
        if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
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
    fn create_inner(&self, name: String, row: &discard_core::branch_tree::TreeRow, repository: &str, repository_path: &std::path::Path) -> Result<String, String> {
        let project = self.core.current_project().id;
        let agent_id = self.store.lock().unwrap_or_else(|e| e.into_inner()).project_main_agent(&project).map_err(|e| e.to_string())?
            .ok_or("This project has no main agent. Open the project first.")?;
        let channel = Channel { id: ChannelId::new(), project_id: Some(project), name, agent_id, workspace_mode: WorkspaceMode::Worktree };
        let record = row.worktree.as_ref().map(|path| -> Result<WorktreeRecord, String> {
            let manager = WorktreeManager::new(&self.project_dir());
            let prepared = manager.recorded(repository_path, path, &row.name).map_err(|e| e.to_string())?;
            Ok(WorktreeRecord { channel_id: channel.id.clone(), path: path.to_string_lossy().into_owned(), branch: prepared.worktree.branch,
                base_commit: prepared.worktree.base_commit, repo: repository_path.to_string_lossy().into_owned(), owned: false })
        }).transpose()?;
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let existing = store.channels_for_project(&self.core.current_project().id).map_err(|e| e.to_string())?;
            if existing.iter().any(|c| c.name.eq_ignore_ascii_case(&channel.name)) { return Err("A channel with this name already exists.".into()); }
            // A detached commit is not a branch and must not claim a canonical branch channel.
            let branch = (!row.detached).then_some(row.name.as_str());
            store.create_named_repository_channel(&channel, repository, branch, record.as_ref()).map_err(|e| e.to_string())?;
            store.register_bootstrap_project_channel(&channel.id).map_err(|e| e.to_string())?;
        }
        self.start(&channel)?;
        self.select_inner(&channel.id.0)?;
        Ok(channel.id.0)
    }
    pub fn decide_write_request(&self, requests: &discard_core::write_requests::WriteRequests, workspace: &discard_core::workspace::GitWorkspace, grants: &discard_core::workspace_grants::MainCheckoutGrants, channel: &str, id: &str, approve: bool) -> Result<discard_core::write_requests::WriteRequest, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        if approve {
            if self.core.active_channel_id() != channel { return Err("Open this channel before approving.".into()); }
            self.core.begin_write_approval(channel)?;
        }
        let result: Result<_, String> = (|| {
            let channel_id = ChannelId(channel.into());
            let request = requests.begin(&channel_id, id, approve)?;
            let mut note = None;
            let request = if approve {
                let action = (|| {
                    let project = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&channel_id).map_err(|e| e.to_string())?.project_id;
                    let (repo, branch, path) = workspace.write_request_target(&channel_id, &AgentId(request.agent_id.clone()), project.as_ref(), request.kind)?;
                    if request.branch != branch || std::path::Path::new(&request.path) != path { return Err("The request target changed. Ask again.".into()); }
                    match request.kind {
                        discard_core::write_requests::WriteRequestKind::WorktreeCreate => {
                            let record = workspace.create_for_branch(&repo, &channel_id, &branch)?;
                            note = Some(format!("Discard created a worktree for {branch} at {}.", record.path));
                            Ok(format!("Created a worktree for `{branch}` at `{}`.", record.path))
                        }
                        discard_core::write_requests::WriteRequestKind::MainCheckoutWrite => {
                            note = Some("The user allowed writes in the main checkout for one response.".into());
                            Ok("The user allowed writes in the main checkout for one response.".into())
                        }
                    }
                })().map_err(|error: String| match request.kind {
                    discard_core::write_requests::WriteRequestKind::WorktreeCreate => format!("Worktree creation failed: {error}"),
                    discard_core::write_requests::WriteRequestKind::MainCheckoutWrite => format!("Main-checkout write access failed: {error}"),
                });
                requests.finish(id, action)
            } else { request };
            let message = discard_core::model::Message {
                id: discard_core::model::MessageId::new(),
                channel_id,
                role: discard_core::model::Role::System,
                blocks: vec![discard_core::model::Block::Markdown { text: request.result.clone().unwrap_or_else(|| request.state.clone()) }],
            };
            self.store.lock().unwrap_or_else(|e| e.into_inner()).insert_message(&message).map_err(|e| e.to_string())?;
            self.core.apply_channel(channel, discard_core::ui_state::Update::ConversationMarker(message));
            self.core.refresh_branches(self.core.request_branch_refresh());
            Ok((request, note))
        })();
        if approve { self.core.end_project_change(); }
        let (request, note) = result?;
        // Keep the channel operation lock until the follow-up is queued, so Send cannot race approval.
        if let Some(note) = note {
            let agent = AgentId(request.agent_id.clone()); let channel_id = ChannelId(channel.into());
            let main = request.kind == discard_core::write_requests::WriteRequestKind::MainCheckoutWrite;
            if main { grants.grant(&channel_id, &agent); }
            if let Err(error) = self.core.continue_in(channel, agent.clone(), note) {
                if main { grants.take(&channel_id, &agent); }
                return Err(error);
            }
        }
        Ok(request)
    }

    pub fn continue_after_approval(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.continue_in(channel, agent, note)
    }

    pub fn send_message(&self, channel: &str, text: String) -> Result<(), String> {
        self.send_with_attachments(channel, text, Vec::new())
    }
    pub fn send_with_attachments(&self, channel: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let recipient = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let id = discard_core::mentions::recipient(&store, &ChannelId(channel.into()), &text)?;
            if store.agent_archived(&id).map_err(|e| e.to_string())? { return Err("This agent is archived. Restore it before sending a message.".into()); }
            let config = store.load_agent(&id).map_err(|e| e.to_string())?;
            Some(discard_core::model::MessageAuthor { agent_id: id, name: config.name, provider: config.provider, avatar: config.avatar })
        };
        self.core.send_to_agent_with_attachments(Some(channel), text, recipient, attachments)
    }

    pub fn open_branch(&self, key: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch()?;
        let project_id = self.core.current_project().id;
        if project_id.0.is_empty() { return Err("Select a project first.".into()); }
        let repository_id = if key.starts_with('[') {
            let parts: Vec<String> = serde_json::from_str(key).map_err(|_| "Invalid repository row.")?;
            if !(2..=3).contains(&parts.len()) { return Err("Invalid repository row.".into()); }
            parts[0].clone()
        } else { String::new() };
        let repository = discard_core::worktree::discover_repositories(&self.project_dir()).map_err(|e| e.to_string())?
            .into_iter().find(|repository| repository.id == repository_id).ok_or("This repository is no longer available.")?;
        let row = list_branch_tree(&repository.path).map_err(|e| e.to_string())?.into_iter()
            .find(|row| row.selection_key(&repository_id) == key).ok_or("This branch or worktree is no longer available.")?;
        let branch = row.name.clone();
        let canonical = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            if row.detached {
                let mut found = None;
                for channel in store.channels_for_project(&project_id).map_err(|e| e.to_string())? {
                    if store.channel_archived(&channel.id).map_err(|e| e.to_string())? { continue; }
                    let metadata = store.project_channel_metadata(&channel.id).map_err(|e| e.to_string())?;
                    if !metadata.is_some_and(|metadata| metadata.kind == discard_core::model::ProjectChannelKind::Legacy) { continue; }
                    if let Some(record) = store.worktree_for(&channel.id).map_err(|e| e.to_string())?
                        && row.worktree.as_ref() == Some(&PathBuf::from(&record.path)) && record.branch == branch
                            && record.repo == repository.path {
                            found = Some(channel);
                            break;
                        }
                }
                found
            } else {
                store.project_repository_branch_channel(&project_id, &repository_id, &branch).map_err(|e| e.to_string())?
            }
        };
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
            let base = if repository_id.is_empty() { branch.clone() } else { format!("{}/{}", repository.label, branch) };
            let mut name = base.clone(); let mut suffix = 2;
            while channels.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) { name = format!("{base}-{suffix}"); suffix += 1; }
            name
        };
        self.create_inner(name, &row, &repository.id, &repository.path)
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
        store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Project agent".into(), provider: "codex".into(), model: Some("model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap()
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
    async fn write_approval_queues_one_scoped_continue_and_no_user_message() {
        use discard_core::{write_requests::{WriteRequests, WriteRequestKind}, workspace::GitWorkspace, workspace_grants::MainCheckoutGrants, ui_state::Command};
        for kind in [WriteRequestKind::WorktreeCreate, WriteRequestKind::MainCheckoutWrite] {
            for approve in [false, true] {
                let dir = tempfile::tempdir().unwrap(); let repo = dir.path().join("repo"); std::fs::create_dir(&repo).unwrap();
                for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-qm", "init"], vec!["branch", "feature"]] {
                    let out = discard_core::process::spawn_std(std::process::Command::new("git").current_dir(&repo).args(["-c", "core.hooksPath=/dev/null", "-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap(); assert!(out.status.success());
                }
                let store = Store::open_in_memory().unwrap();
                let mut config = AgentConfig::from_toml("name='A'\nprovider='fake'\nmodel='test'\npolicy='ask'").unwrap(); config.workspace_write = true;
                let agent = store.create_agent(&config).unwrap(); let project = store.import_project_folder(&repo).unwrap();
                let branch = if kind == WriteRequestKind::WorktreeCreate { "feature" } else { "main" };
                let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: branch.into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
                store.create_named_channel(&channel, Some(branch), None).unwrap();
                if kind == WriteRequestKind::MainCheckoutWrite {
                    store.save_worktree(&WorktreeRecord {
                        channel_id: channel.id.clone(),
                        path: repo.to_string_lossy().into(),
                        branch: branch.into(),
                        repo: repo.to_string_lossy().into(),
                        base_commit: String::new(),
                        owned: false,
                    }).unwrap();
                }
                let store = Arc::new(Mutex::new(store)); let workspace = GitWorkspace::new(&dir.path().join("worktrees"), store.clone());
                let requests = WriteRequests::default(); let grants = MainCheckoutGrants::default();
                let (_, branch, path) = workspace.write_request_target(&channel.id, &agent, Some(&project.id), kind).unwrap();
                let request = requests.request(kind, &channel.id, &agent, branch, path.to_string_lossy().into()).unwrap();
                let (commands, mut received) = mpsc::unbounded_channel();
                let core = Arc::new(AppCore::new(CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "P".into(), channel: channel.name.clone(), agent: "A".into(), provider: "fake".into() }));
                core.configure_targets(project.id.0.clone(), &repo, agent.0.clone());
                let channels = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
                assert!(channels.decide_write_request(&requests, &workspace, &grants, "other", &request.id, true).is_err());
                core.send_to(Some(&channel.id.0), "initial request".into()).unwrap();
                assert!(matches!(received.try_recv(), Ok(Command::Send(_))));
                assert_eq!(channels.decide_write_request(&requests, &workspace, &grants, &channel.id.0, &request.id, true).unwrap_err(), "Wait for the current response to finish.");
                core.apply(Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
                let result = channels.decide_write_request(&requests, &workspace, &grants, &channel.id.0, &request.id, approve).unwrap();
                assert_eq!(result.state, if approve { "completed" } else { "denied" });
                if approve {
                    assert!(matches!(received.try_recv(), Ok(Command::Continue { agent: recipient, .. }) if recipient == agent));
                    if kind == WriteRequestKind::WorktreeCreate { assert!(store.lock().unwrap().worktree_for(&channel.id).unwrap().unwrap().owned); }
                    else { assert!(grants.take(&channel.id, &agent)); assert!(!grants.take(&channel.id, &agent)); }
                }
                assert!(received.try_recv().is_err());
                core.apply(Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
                assert!(channels.decide_write_request(&requests, &workspace, &grants, &channel.id.0, &request.id, true).is_err());
                assert!(received.try_recv().is_err());
                let messages = store.lock().unwrap().messages_for_channel(&channel.id).unwrap();
                assert_eq!(messages.len(), 1); assert_eq!(messages[0].role, discard_core::model::Role::System);
            }
        }
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
    async fn detached_worktrees_open_distinct_durable_channels_without_creating_branches() {
        use discard_core::{engine::WorkspaceService, workspace::GitWorkspace};
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let output = discard_core::process::spawn_std(std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
                .args(args).current_dir(&repo).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "--allow-empty", "-m", "initial"]);
        let head = git(&["rev-parse", "HEAD"]);
        let first_path = dir.path().join("first");
        let second_path = dir.path().join("second");
        for path in [&first_path, &second_path] {
            git(&["worktree", "add", "--detach", path.to_str().unwrap(), "HEAD"]);
        }
        let database = dir.path().join("channels.db");
        let store = Store::open(&database).unwrap();
        let project = store.import_project_folder(&repo).unwrap();
        configured_agent(&store);
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit {
            state: UiState::new(ChannelId(String::new()), vec![]), commands,
            project: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
        }));
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        service.activate_project(&project.id).unwrap();
        let rows: Vec<_> = core.snapshot().branches.into_iter().filter(|row| row.detached).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, rows[1].label);
        assert_ne!(rows[0].key, rows[1].key);
        // A real branch with the same short SHA must retain its own channel.
        git(&["branch", &rows[0].label]);
        let branch_channel = service.open_branch(&rows[0].label).unwrap();
        assert!(core.snapshot().branches.iter().filter(|row| row.current).all(|row| !row.detached));
        let first = service.open_branch(&rows[0].key).unwrap();
        let second = service.open_branch(&rows[1].key).unwrap();
        assert_ne!(first, second);
        assert_ne!(first, branch_channel);
        assert_eq!(service.open_branch(&rows[0].key).unwrap(), first);
        let selected: Vec<_> = core.snapshot().branches.into_iter().filter(|row| row.current).collect();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].key, rows[0].key);
        let workspace = GitWorkspace::new(&dir.path().join("managed"), store.clone());
        for (id, row) in [(&first, &rows[0]), (&second, &rows[1])] {
            let channel = ChannelId(id.clone());
            let path = PathBuf::from(row.worktree.as_ref().unwrap());
            assert_eq!(workspace.conversation_directory(&repo, &channel).unwrap(), path);
            assert_eq!(workspace.prepare(&repo, &channel, &row.label).unwrap().cwd, path);
            let store = store.lock().unwrap();
            assert!(!store.worktree_for(&channel).unwrap().unwrap().owned);
            assert_eq!(store.channel_branch(&channel).unwrap(), None);
        }
        assert_eq!(git(&["rev-parse", "HEAD"]), head);
        assert_eq!(git(&["branch", "--format=%(refname:short)"]).lines().count(), 2);
        let channels = service.list_project_channels(&project.id).unwrap();
        assert_eq!(channels.len(), 4);
        let binding = channels.iter().find(|channel| channel.id == first).unwrap().worktree.as_ref().unwrap();
        assert_eq!(Some(&binding.path), rows[0].worktree.as_ref());
        assert_eq!(binding.branch, rows[0].label);
        // Reopening the store retains the same path-bound conversations.
        let reopened = Arc::new(Mutex::new(Store::open(&database).unwrap()));
        let restored = Channels::new(core.clone(), reopened, Handle::current(), launcher());
        assert_eq!(restored.open_branch(&rows[1].key).unwrap(), second);
        // A stale key cannot silently open the same path on a different checkout.
        let path = rows[0].worktree.as_ref().unwrap();
        let output = discard_core::process::spawn_std(std::process::Command::new("git").args(["-C", path, "checkout", "-b", "changed"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
        assert!(output.status.success());
        assert!(restored.open_branch(&rows[0].key).is_err());
        assert!(workspace.prepare(&repo, &ChannelId(first), &rows[0].label).is_err());
        // The main checkout can also have detached HEAD, including in a nested repository.
        git(&["checkout", "--detach"]);
        let nested_project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        service.activate_project(&nested_project.id).unwrap();
        let main = core.snapshot().branches.into_iter().find(|row| row.detached && row.kind == "main").unwrap();
        assert!(!main.repository.is_empty());
        let main_channel = service.open_branch(&main.key).unwrap();
        assert_eq!(workspace.conversation_directory(dir.path(), &ChannelId(main_channel.clone())).unwrap(), repo.canonicalize().unwrap());
        assert_eq!(service.open_branch(&main.key).unwrap(), main_channel);
        assert_eq!(core.snapshot().branches.iter().filter(|row| row.current).count(), 1);
    }

    #[tokio::test]
    async fn nested_repository_channels_keep_branch_identity_and_workspace_scope() {
        use discard_core::{engine::WorkspaceService, workspace::GitWorkspace};
        let dir = tempfile::tempdir().unwrap();
        for name in ["frontend", "backend"] {
            let repo = dir.path().join(name);
            std::fs::create_dir(&repo).unwrap();
            for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-m", "initial"], vec!["branch", "feature"]] {
                let output = discard_core::process::spawn_std(std::process::Command::new("git")
                    .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
                    .args(args).current_dir(&repo).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            }
        }
        let database = dir.path().join("channels.db");
        let store = Store::open(&database).unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
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
        assert_eq!(snapshot.repositories.len(), 2);
        let main_rows: Vec<_> = snapshot.branches.iter().filter(|row| row.label == "main").collect();
        assert_eq!(main_rows.len(), 2);
        let first = service.open_branch(&main_rows[0].key).unwrap();
        let second = service.open_branch(&main_rows[1].key).unwrap();
        assert_ne!(first, second);
        assert_eq!(service.open_branch(&main_rows[0].key).unwrap(), first);
        assert_eq!(core.snapshot().branches.iter().filter(|row| row.current).count(), 1);
        assert_eq!(core.snapshot().branches.iter().find(|row| row.current).unwrap().key, main_rows[0].key);
        {
            let store = store.lock().unwrap();
            assert!(!store.channel_archived(&ChannelId(first.clone())).unwrap());
            assert!(!store.channel_archived(&ChannelId(second.clone())).unwrap());
            for (id, row) in [(&first, main_rows[0]), (&second, main_rows[1])] {
                let record = store.worktree_for(&ChannelId(id.clone())).unwrap().unwrap();
                assert_eq!(record.repo, row.repository);
                assert_eq!(record.path, row.repository);
            }
        }
        let workspace = GitWorkspace::new(&dir.path().join("worktrees"), store.clone());
        assert_eq!(workspace.conversation_directory(dir.path(), &ChannelId(first.clone())).unwrap(), PathBuf::from(&main_rows[0].repository));
        let borrowed = workspace.prepare(dir.path(), &ChannelId(first.clone()), "main").unwrap();
        assert_eq!(borrowed.cwd, PathBuf::from(&main_rows[0].repository));
        let feature = snapshot.branches.iter().find(|row| row.label == "feature" && row.repository == main_rows[1].repository).unwrap();
        let feature_id = service.open_branch(&feature.key).unwrap();
        assert!(store.lock().unwrap().worktree_for(&ChannelId(feature_id.clone())).unwrap().is_none(), "opening a branch does not create a worktree");
        assert_eq!(workspace.conversation_directory(dir.path(), &ChannelId(feature_id.clone())).unwrap(), PathBuf::from(&feature.repository));
        let prepared = workspace.prepare(dir.path(), &ChannelId(feature_id.clone()), "feature").unwrap();
        assert_eq!(workspace.conversation_directory(dir.path(), &ChannelId(feature_id.clone())).unwrap(), prepared.cwd);
        assert!(prepared.cwd.exists());
        let record = store.lock().unwrap().worktree_for(&ChannelId(feature_id.clone())).unwrap().unwrap();
        assert_eq!(record.repo, feature.repository);
        assert_eq!(record.branch, "feature");
        // Persisted bindings must remain distinct after reopening the database.
        let reopened = Store::open(&database).unwrap();
        assert_eq!(reopened.project_repository_branch_channel(&project.id, &main_rows[0].repository, "main").unwrap().unwrap().id.0, first);
        assert_eq!(reopened.project_repository_branch_channel(&project.id, &main_rows[1].repository, "main").unwrap().unwrap().id.0, second);
        assert!(reopened.project_branch_channel(&project.id, "main").unwrap().is_none());
        let unknown = discard_core::worktree::repository_branch_key("/outside-project", "main");
        assert!(service.open_branch(&unknown).is_err());
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
            let output = discard_core::process::spawn_std(std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
                .args(args).current_dir(repo.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
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
        store.insert_agent(&agent, &AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: Some("agent-model".into()), avatar: None, system_prompt: "Review changes".into(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
            let output = discard_core::process::spawn_std(std::process::Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"]).args(args).current_dir(dir.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "-q", "-b", "old"]);
        git(&["commit", "--allow-empty", "-m", "initial"]);
        let store = Store::open_in_memory().unwrap();
        let project = Project { id: ProjectId::new(), name: "p".into(), path: dir.path().to_string_lossy().into_owned() };
        let agent = AgentId::new();
        store.insert_project(&project).unwrap();
        store.insert_agent(&agent, &AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: None, avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
        let direct_agent = store.lock().unwrap().create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Direct".into(), provider: "codex".into(), model: Some("direct-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
    async fn migrated_project_branch_and_topic_survive_service_reload() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = discard_core::process::spawn_std(std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(dir.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output())
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "--allow-empty", "-m", "initial"]);
        git(&["branch", "feature/persisted"]);

        let db = dir.path().join("project-channels.db");
        let (project_id, agent_id, general_id, branch_id) = {
            let store = Store::open(&db).unwrap();
            let project = store.import_project_folder(dir.path()).unwrap();
            let agent = configured_agent(&store);
            let general = store.create_conversation(&agent, Some(&project.id), "general").unwrap();
            let branch = store.create_conversation(&agent, Some(&project.id), "Feature").unwrap();
            for (channel, text) in [(&general, "legacy general history"), (&branch, "legacy branch history")] {
                store.insert_message(&discard_core::model::Message {
                    id: discard_core::model::MessageId::new(),
                    channel_id: channel.id.clone(),
                    role: discard_core::model::Role::User,
                    blocks: vec![discard_core::model::Block::Markdown { text: text.into() }],
                }).unwrap();
            }
            (project.id, agent, general.id, branch.id)
        };

        // Reopen this database as a v3 fixture. The app Store then exercises the real v4 migration.
        {
            let connection = rusqlite::Connection::open(&db).unwrap();
            connection.execute_batch("DROP TABLE channel_topic_revisions;
                DROP TABLE project_last_channels;
                DROP TABLE project_channel_views;
                DROP TABLE channel_agents;
                DROP TABLE project_agents;
                DROP TABLE project_main_agents;
                DROP TABLE project_channels;
                DROP TABLE channel_branches;
                CREATE TABLE channel_branches (channel_id TEXT PRIMARY KEY REFERENCES channels(id), branch TEXT NOT NULL);").unwrap();
            connection.execute("INSERT INTO channel_branches(channel_id,branch) VALUES(?1,'feature/persisted')", [&branch_id.0]).unwrap();
            connection.execute_batch("ALTER TABLE projects DROP COLUMN avatar; ALTER TABLE messages DROP COLUMN author_json;").unwrap();
            connection.execute_batch("DROP TABLE always_allow_rules;").unwrap();
            connection.pragma_update(None, "user_version", 3).unwrap();
        }

        let migrated = Store::open(&db).unwrap();
        assert_eq!(migrated.project_channel_metadata(&branch_id).unwrap().unwrap().branch.as_deref(), Some("feature/persisted"));
        assert_eq!(migrated.messages_for_channel(&branch_id).unwrap().len(), 1);
        let general = migrated.load_channel(&general_id).unwrap();
        let app_core = core(&general, dir.path());
        let store = Arc::new(Mutex::new(migrated));
        let service = Channels::new(app_core.clone(), store.clone(), Handle::current(), launcher());

        assert_eq!(service.activate_project(&project_id).unwrap(), general_id.0);
        let unrelated = store.lock().unwrap().create_agent(&AgentConfig {
            workspace_write: false, sandbox_autorun: false,
            reasoning_effort: None,
            name: "Direct agent".into(),
            provider: "codex".into(),
            model: Some("direct-model".into()),
            avatar: None,
            system_prompt: String::new(),
            policy: ApprovalPolicy::Ask,
            plugin_write: false,
        }).unwrap();
        service.open_agent(&unrelated).unwrap();
        assert_eq!(service.open_branch("feature/persisted").unwrap(), branch_id.0);
        assert_eq!(app_core.snapshot().channel, "Feature");
        assert_eq!(store.lock().unwrap().channel_agents(&branch_id).unwrap(), vec![agent_id.clone()]);
        service.update_channel_topic(&project_id, &branch_id, 0, "Keep the migrated branch history").unwrap();
        assert_eq!(store.lock().unwrap().messages_for_channel(&branch_id).unwrap().len(), 1);

        drop(service);
        drop(app_core);
        drop(store);
        let reopened = Store::open(&db).unwrap();
        assert_eq!(reopened.last_project_channel(&project_id).unwrap().unwrap().id, branch_id);
        assert_eq!(reopened.channel_topic(&branch_id).unwrap().text, "Keep the migrated branch history");
        assert_eq!(reopened.channel_topic_history(&branch_id).unwrap().len(), 1);
        let branch_messages = reopened.messages_for_channel(&branch_id).unwrap();
        assert!(matches!(
            branch_messages.as_slice(),
            [discard_core::model::Message { blocks, .. }]
                if matches!(blocks.as_slice(), [discard_core::model::Block::Markdown { text }] if text == "legacy branch history")
        ));
        assert_eq!(reopened.load_channel(&general_id).unwrap().agent_id, agent_id);

        let reopened_general = reopened.load_channel(&general_id).unwrap();
        let reopened_core = core(&reopened_general, dir.path());
        let reopened_store = Arc::new(Mutex::new(reopened));
        let reopened_service = Channels::new(reopened_core.clone(), reopened_store, Handle::current(), launcher());
        assert_eq!(reopened_service.activate_project(&project_id).unwrap(), branch_id.0);
        assert_eq!(reopened_core.active_channel_id(), branch_id.0);
        assert_eq!(reopened_core.snapshot().channel, "Feature");
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
    async fn saved_branch_channel_opens_by_id_when_git_listing_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let (project, agent, branch) = {
            let store = store.lock().unwrap();
            let project = store.import_project_folder(dir.path()).unwrap();
            let agent = configured_agent(&store);
            store.set_project_main_agent(&project.id, &agent).unwrap();
            let branch = store.create_conversation(&agent, Some(&project.id), "feature/x").unwrap();
            store.bind_branch(&branch.id, "feature/x").unwrap();
            (project, agent, branch)
        };
        let core = core(&branch, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        assert!(list_branch_tree(dir.path()).is_err(), "test project has no Git metadata");
        assert_eq!(service.select(&branch.id.0).unwrap(), ());
        assert_eq!(core.active_channel_id(), branch.id.0);
        assert_eq!(core.active_context().0, agent);
        assert_eq!(store.lock().unwrap().project_channel_metadata(&branch.id).unwrap().unwrap().branch.as_deref(), Some("feature/x"));
        assert_eq!(store.lock().unwrap().load_channel(&branch.id).unwrap().project_id, Some(project.id));
    }

    #[tokio::test]
    async fn builder_bootstrap_on_v4_database_is_listed_and_keeps_its_coding_identity() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("builder.db");
        let project_dir = dir.path().join("workspace");
        std::fs::create_dir(&project_dir).unwrap();
        let store = Store::open(&db).unwrap();
        drop(store);
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch("ALTER TABLE projects DROP COLUMN avatar; ALTER TABLE messages DROP COLUMN author_json;").unwrap();
            conn.execute_batch("DROP TABLE always_allow_rules;").unwrap();
            conn.pragma_update(None, "user_version", 4).unwrap();
        }
        let store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
        let builder = discard_core::bootstrap::ensure_workspace(
            &store.lock().unwrap(), &project_dir, "codex", discard_core::bootstrap::Role::Builder,
            discard_core::bootstrap::WorkspaceChoice::Main,
        ).unwrap();
        let app_core = core(&builder.channel, &project_dir);
        let service = Channels::new(app_core, store.clone(), Handle::current(), launcher());
        let listed = service.list_project_channels(&builder.project.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, builder.channel.id.0);
        assert_eq!(listed[0].kind, "legacy");
        assert_eq!(listed[0].workspace_mode, "main_checkout");
        let details = service.channel_details(&builder.project.id, &builder.channel.id).unwrap();
        assert_eq!(details.channel.kind, "legacy");
        assert!(!details.channel.archived);
        assert_eq!(details.channel.agent_id, builder.channel.agent_id.0);
        drop(service);
        drop(store);

        let reopened = Arc::new(Mutex::new(Store::open(&db).unwrap()));
        let next = discard_core::bootstrap::ensure_workspace(
            &reopened.lock().unwrap(), &project_dir, "codex", discard_core::bootstrap::Role::Builder,
            discard_core::bootstrap::WorkspaceChoice::Main,
        ).unwrap();
        assert_eq!(next.channel.id, builder.channel.id);
        assert_eq!(next.session_id, builder.session_id);
        let reopened_service = Channels::new(core(&next.channel, &project_dir), reopened, Handle::current(), launcher());
        assert_eq!(reopened_service.list_project_channels(&next.project.id).unwrap()[0].id, builder.channel.id.0);
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
        service.store.lock().unwrap().archive_channel(&general.id).unwrap();
        assert!(service.update_channel_topic(&project.id, &general.id, 1, "late edit").unwrap_err().contains("archived"));
        assert!(service.invite_channel_member(&project.id, &general.id, &agent).unwrap_err().contains("archived"));
    }

    #[tokio::test]
    async fn topic_validation_and_cas_hold_the_store_lock_against_archive() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store.lock().unwrap());
        store.lock().unwrap().set_project_main_agent(&project.id, &agent).unwrap();
        let channel = store.lock().unwrap().ensure_project_general_channel(&project.id, &agent).unwrap();
        let core = core(&channel, dir.path());
        let service = Arc::new(Channels::new(core, store.clone(), Handle::current(), launcher()));
        let (validated_tx, validated_rx) = std::sync::mpsc::sync_channel(0);
        let (continue_tx, continue_rx) = std::sync::mpsc::sync_channel(0);
        let update_service = service.clone();
        let project_id = project.id.clone();
        let channel_id = channel.id.clone();
        let update = std::thread::spawn(move || update_service.with_active_project_channel(&project_id, &channel_id, |store| {
            validated_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
            store.save_channel_topic(&channel_id, 0, "atomic topic", discard_core::model::TopicAuthorKind::User, None).map_err(|e| e.to_string())
        }));
        validated_rx.recv().unwrap();
        let (archive_started_tx, archive_started_rx) = std::sync::mpsc::sync_channel(0);
        let archive_store = store.clone();
        let archive_id = channel.id.clone();
        let archive = std::thread::spawn(move || {
            archive_started_tx.send(()).unwrap();
            archive_store.lock().unwrap().archive_channel(&archive_id).unwrap();
        });
        archive_started_rx.recv().unwrap();
        continue_tx.send(()).unwrap();
        assert_eq!(update.join().unwrap().unwrap().version, 1);
        archive.join().unwrap();
        assert!(store.lock().unwrap().channel_archived(&channel.id).unwrap());
        assert_eq!(store.lock().unwrap().channel_topic(&channel.id).unwrap().text, "atomic topic");
    }

    #[tokio::test]
    async fn failed_main_agent_restart_closes_old_transport_and_selection_retries() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        let old_agent = configured_agent(&store.lock().unwrap());
        let new_agent = store.lock().unwrap().create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "New main".into(), provider: "claude".into(), model: Some("new-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
        assert!(core.begin_channel_change(&general.id.0).is_ok());
        assert!(!core.archive_started_for(Some(&general.id.0)), "archive cannot start while channel identity is changing");
        core.end_channel_change(&general.id.0);
        assert!(core.archive_started_for(Some(&general.id.0)));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("archive confirmation"), "channel identity cannot change while the archive worker is active");
        assert_eq!(store.lock().unwrap().project_main_agent(&project.id).unwrap(), Some(old_agent.clone()));
        core.archive_finished(super::super::plan::ArchiveStep::NeedsConfirm(vec!["dirty worktree".into()]));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("archive confirmation"));
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, old_agent);
        assert!(core.archive_started_for(Some(&general.id.0)), "accepting an archive confirmation can restart the archive worker");
        core.archive_finished(super::super::plan::ArchiveStep::Failed("simulated archive failure".into()));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("simulated launcher failure"));
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, new_agent);
        assert!(core.snapshot().notice.unwrap_or_default().contains("Select this channel again to retry"));
        assert!(core.send_to(Some(&general.id.0), "old agent must not accept this".into()).is_err());
        service.select(&general.id.0).unwrap();
        assert_eq!(core.active_context().0, new_agent);
        assert!(!core.send_to(Some(&general.id.0), "new agent is ready".into()).is_err());
    }

    #[tokio::test]
    async fn deleting_main_agent_restarts_loaded_active_and_parked_general_channels() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let (first, _second, old, first_replacement, second_replacement, first_general, second_general) = {
            let store = store.lock().unwrap();
            let first = store.import_project_folder(first_dir.path()).unwrap();
            let second = store.import_project_folder(second_dir.path()).unwrap();
            let old = configured_agent(&store);
            let first_replacement = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Replacement one".into(), provider: "codex".into(), model: Some("one".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
            let second_replacement = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, reasoning_effort: None, name: "Replacement two".into(), provider: "codex".into(), model: Some("two".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
            store.set_project_main_agent(&first.id, &old).unwrap();
            store.set_project_main_agent(&second.id, &old).unwrap();
            let first_general = store.ensure_project_general_channel(&first.id, &old).unwrap();
            let second_general = store.ensure_project_general_channel(&second.id, &old).unwrap();
            store.insert_message(&discard_core::model::Message {
                id: discard_core::model::MessageId::new(), channel_id: first_general.id.clone(),
                role: discard_core::model::Role::Agent,
                blocks: vec![discard_core::model::Block::Markdown { text: "retained general history".into() }],
            }).unwrap();
            store.invite_agent_to_channel(&first.id, &first_general.id, &first_replacement).unwrap();
            store.invite_agent_to_channel(&second.id, &second_general.id, &second_replacement).unwrap();
            (first, second, old, first_replacement, second_replacement, first_general, second_general)
        };
        let receivers = Arc::new(Mutex::new(Vec::new()));
        let fail_second_replacement = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let launch_store = store.clone();
        let launch_receivers = receivers.clone();
        let fail_flag = fail_second_replacement.clone();
        let failed_channel_id = second_general.id.clone();
        let launch: Launcher = Arc::new(move |channel| {
            if fail_flag.load(std::sync::atomic::Ordering::SeqCst) && channel.id == failed_channel_id {
                fail_flag.store(false, std::sync::atomic::Ordering::SeqCst);
                return Err("simulated replacement launch failure".into());
            }
            let cfg = launch_store.lock().unwrap().load_agent(&channel.agent_id).unwrap();
            let (commands, receiver) = mpsc::unbounded_channel();
            launch_receivers.lock().unwrap().push(receiver);
            let (_, updates) = mpsc::unbounded_channel();
            Ok((CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "Project".into(), channel: channel.name.clone(), agent: cfg.name, provider: cfg.provider }, updates))
        });
        let initial = launch(&first_general).unwrap().0;
        let core = Arc::new(AppCore::new(initial));
        core.configure_targets(first.id.0.clone(), first_dir.path(), old.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launch);
        service.select(&second_general.id.0).unwrap();
        assert_eq!(core.active_channel_id(), second_general.id.0);
        assert!(core.has_channel(&first_general.id.0), "the first general remains parked");
        core.begin_agent_change(&old.0).unwrap();
        store.lock().unwrap().delete_agent(&old, discard_core::store::DeleteAgentOptions { delete_conversations: false, delete_private_memory: false }).unwrap();
        fail_second_replacement.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(service.reconcile_deleted_agent(&old.0).unwrap_err().contains("simulated replacement launch failure"));
        core.end_agent_change(&old.0);

        assert_eq!(store.lock().unwrap().load_channel(&first_general.id).unwrap().agent_id, first_replacement);
        assert_eq!(store.lock().unwrap().load_channel(&second_general.id).unwrap().agent_id, second_replacement);
        assert!(store.lock().unwrap().messages_for_channel(&first_general.id).unwrap().iter().any(|message| matches!(message.blocks.as_slice(), [discard_core::model::Block::Markdown { text }] if text == "retained general history")));
        assert_eq!(core.active_context().0, second_replacement);
        assert!(!core.channel_session_closed(&first_general.id.0));
        assert!(core.channel_session_closed(&second_general.id.0), "failed replacement stays closed and retryable");
        assert_eq!(receivers.lock().unwrap().len(), 3, "the failed channel has no live replacement receiver");
        service.select(&second_general.id.0).unwrap();
        assert!(!core.channel_session_closed(&second_general.id.0));
        assert!(core.send_to(Some(&second_general.id.0), "replacement is ready".into()).is_ok());
    }

    #[tokio::test]
    async fn canonical_branch_selection_promotes_a_linked_archive() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store);
        let archived = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "Feature old".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let current = Channel { id: ChannelId::new(), name: "Feature current".into(), ..archived.clone() };
        store.create_named_channel(&archived, Some("feature/shared"), None).unwrap();
        store.create_named_channel(&current, Some("feature/shared"), None).unwrap();
        assert!(store.channel_archived(&archived.id).unwrap());
        let core = core(&current, dir.path());
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        service.select_canonical_branch(&project.id, "feature/shared", &archived.id).unwrap();
        assert_eq!(core.active_channel_id(), archived.id.0);
        assert!(!store.lock().unwrap().channel_archived(&archived.id).unwrap());
        assert!(store.lock().unwrap().channel_archived(&current.id).unwrap());
    }

    #[tokio::test]
    async fn canonical_branch_refresh_keeps_an_archived_lead_read_only_with_an_open_sender() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store.lock().unwrap());
        let channel = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "Feature".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        store.lock().unwrap().create_named_channel(&channel, Some("feature"), None).unwrap();
        store.lock().unwrap().set_agent_archived(&agent, true).unwrap();
        let receivers = Arc::new(Mutex::new(Vec::new()));
        let launch: Launcher = {
            let receivers = receivers.clone(); let store = store.clone();
            Arc::new(move |channel| {
                let cfg = store.lock().unwrap().load_agent(&channel.agent_id).unwrap();
                let (commands, receiver) = mpsc::unbounded_channel(); receivers.lock().unwrap().push(receiver);
                let (_, updates) = mpsc::unbounded_channel();
                Ok((CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "Project".into(), channel: channel.name.clone(), agent: cfg.name, provider: cfg.provider }, updates))
            })
        };
        let (init, _) = launch(&channel).unwrap();
        let core = Arc::new(AppCore::new(init));
        core.configure_targets(project.id.0.clone(), dir.path(), agent.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launch);
        service.select_canonical_branch(&project.id, "feature", &channel.id).unwrap();
        assert!(core.snapshot().composer_disabled, "canonical selection preserves archived-agent read-only state");
        assert!(core.send_to(Some(&channel.id.0), "must not be sent".into()).is_err());
        assert!(matches!(receivers.lock().unwrap()[0].try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }
}
