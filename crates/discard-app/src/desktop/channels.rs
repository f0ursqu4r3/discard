//! Real project channels. Git and storage work run off the webview thread.
use discard_core::model::{AgentId, MessageId};
use super::core::{AppCore, CoreInit, WINDOW_CLOSED};
use super::threads::ThreadRecord;
use super::view::ViewId;
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
    fn project_dir(&self, view: &ViewId) -> PathBuf { self.core.current_project(view).path.into() }
    /// The view that carries out a selection of `target` for `view`: the view itself, or the main view when `view` is a
    /// project view and the target is outside its project (decisions 17 and 17b). A closed view is an error. It never
    /// acts on another view.
    fn routed(&self, view: &ViewId, target: Option<&ProjectId>) -> Result<ViewId, String> {
        if !self.core.has_view(view) { return Err(WINDOW_CLOSED.into()); }
        Ok(self.core.route(view, target))
    }
    /// The channel that a rail click on a project opens: the saved channel of the project, or its `#general`.
    fn saved_channel(store: &Store, id: &ProjectId) -> Result<Channel, String> {
        let main = match store.project_main_agent(id).map_err(|e| e.to_string())? {
            Some(main) => main,
            None => store.list_agents().map_err(|e| e.to_string())?.into_iter()
                .find(|(_, config)| config.model.as_ref().is_some_and(|model| !model.trim().is_empty()))
                .map(|(agent, _)| agent)
                .ok_or("No configured agent with a model is available. Configure an agent in Settings, then open this project again.")?,
        };
        store.ensure_project_general_channel(id, &main).map_err(|e| e.to_string())?;
        store.project_channel_or_fallback(id, &main).map_err(|e| e.to_string())
    }
    /// Caller holds the project-change guard. Prepare runtimes before replacing visible state. A project view that asks
    /// for another project routes to the main view. Returns the view that changed and the ID of its channel. Only the
    /// main view changes the selected project of the store.
    pub fn activate_project(&self, view: &ViewId, id: &ProjectId) -> Result<(ViewId, String), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let view = &self.routed(view, Some(id))?;
        self.core.can_switch_for_project(view)?;
        let (project, channel) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let project = store.visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id == *id).ok_or("Project unavailable")?;
            if view.is_main() { store.select_project(id).map_err(|e| e.to_string())?; }
            let channel = Self::saved_channel(&store, id)?;
            (project, channel)
        };
        self.core.select_sidebar_project(view, Some(&project));
        self.select_inner_mode(view, &channel.id.0, true)?;
        self.store.lock().unwrap_or_else(|e| e.into_inner()).select_project_channel(id, &channel.id).map_err(|e| e.to_string())?;
        self.core.refresh_branches_in_background(view);
        Ok((view.clone(), channel.id.0))
    }
    /// Move the main view off a project that is about to be removed: to the next visible project, or to no project.
    /// A project view never changes project, so no other view moves.
    pub fn leave_removed_project(&self, id: &ProjectId) -> Result<(), String> {
        let main = ViewId::main();
        if self.core.current_project(&main).id != *id { return Ok(()); }
        let next = self.store.lock().unwrap_or_else(|e| e.into_inner()).visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id != *id);
        match next {
            Some(next) => { self.activate_project(&main, &next.id)?; }
            None => self.core.select_sidebar_project(&main, None),
        }
        Ok(())
    }
    /// Open the view of the project window of `project`, fixed to the project, on the saved channel of the project.
    /// A view that exists is returned as it is. The new view starts its first branch refresh.
    pub fn open_project_view(&self, project: &ProjectId) -> Result<ViewId, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let view = ViewId::project(project);
        if self.core.has_view(&view) { return Ok(view); }
        let (entry, channel) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let entry = store.visible_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id == *project).ok_or("Project unavailable")?;
            let channel = Self::saved_channel(&store, project)?;
            (entry, channel)
        };
        let opened = self.core.open_view(view.clone(), &entry);
        let exists = self.core.has_view(&view);
        let view = view_after_open(view, opened, exists)?;
        if !opened { return Ok(view); }
        if let Err(error) = self.select_inner_mode(&view, &channel.id.0, true) {
            // A project view with no channel is of no use. A failed open leaves no view behind.
            self.core.drop_view(&view);
            return Err(error);
        }
        self.core.refresh_branches_in_background(&view);
        Ok(view)
    }
    pub fn restore(&self, view: &ViewId) -> Result<(), String> {
        if let Some(id) = self.core.saved_conversation()
            && self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&ChannelId(id.clone())).is_ok() {
                self.select(view, &id)?;
            }
        Ok(())
    }
    fn start(&self, channel: &Channel) -> Result<(), String> { self.start_mode(channel, false) }
    /// Copy the saved mute flag of a channel to its loaded state, so the unread marker follows it.
    fn sync_muted(&self, id: &ChannelId) {
        let muted = self.store.lock().unwrap_or_else(|e| e.into_inner()).channel_muted(id).unwrap_or(false);
        self.core.set_channel_muted(&id.0, muted);
    }
    fn refresh_channel_access(&self, channel: &Channel) -> Result<(), String> {
        let (archived, unavailable) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            // A thread is archived with its parent channel.
            let owner = store.thread_info(&channel.id).map_err(|e| e.to_string())?.map_or_else(|| channel.id.clone(), |thread| thread.parent_channel_id);
            (store.channel_archived(&owner).map_err(|e| e.to_string())?, store.agent_archived(&channel.agent_id).unwrap_or(true))
        };
        self.core.channel_read_only(&channel.id.0, archived || unavailable, archived);
        Ok(())
    }
    fn start_mode(&self, channel: &Channel, restart: bool) -> Result<(), String> {
        // A channel has one session. A channel that the pool holds, another view's included, launches nothing.
        if !restart && self.core.has_channel(&channel.id.0) { return Ok(()); }
        let (init, mut updates) = (self.launch)(channel)?;
        let thread = self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(&channel.id).map_err(|e| e.to_string())?;
        if restart { if !self.core.restart_closed_channel(init, channel)? { return Ok(()); } }
        else if let Some(thread) = &thread { self.core.add_thread(init, channel, thread); }
        else { self.core.add_channel(init, channel); }
        self.refresh_channel_access(channel)?;
        if thread.is_none() { self.sync_muted(&channel.id); }
        if thread.is_none() && !restart { self.refresh_threads(&channel.id)?; }
        let core = self.core.clone();
        let store = self.store.clone();
        let id = channel.id.0.clone();
        let generation = core.channel_runtime_generation(&id).ok_or("Conversation unavailable")?;
        self.runtime.spawn(async move {
            while let Some(update) = updates.recv().await {
                if core.apply_runtime(&id, Some(generation), update) { core.refresh_branches_selecting(&id); }
            }
            // The engine of a sub-agent thread ended without a final state: the task fails.
            if core.sub_agents().is_running(&id) { sub_agent_stream_closed(&core, &store, &id); }
        });
        Ok(())
    }
    /// True when the channel belongs to a project. Selecting it leaves the catalogs as they are.
    pub fn in_project(&self, id: &str) -> bool {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&ChannelId(id.to_owned())).is_ok_and(|channel| channel.project_id.is_some())
    }
    /// Select a channel. A thread opens beside its parent channel, which becomes the selected channel. A project view
    /// that selects a channel of another project, or a direct message, routes to the main view. Returns the view that
    /// changed.
    pub fn select(&self, view: &ViewId, id: &str) -> Result<ViewId, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let thread = self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(&ChannelId(id.to_owned())).map_err(|e| e.to_string())?;
        if let Some(thread) = thread { return self.select_thread_inner(view, &thread); }
        self.select_inner(view, id)
    }
    /// Select the parent of a thread and open the thread beside it, in the view that the parent project routes to.
    fn select_thread_inner(&self, view: &ViewId, thread: &discard_core::model::ThreadInfo) -> Result<ViewId, String> {
        let project = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&thread.parent_channel_id).map_err(|e| e.to_string())?.project_id;
        let view = self.routed(view, project.as_ref())?;
        if self.core.active_channel_id(&view) != thread.parent_channel_id.0 { self.select_inner(&view, &thread.parent_channel_id.0)?; }
        self.open_thread_inner(&view, &thread.channel_id)?;
        Ok(view)
    }
    /// Select the parent channel of a thread and open the thread beside it. Returns the view that changed and the parent
    /// channel ID.
    pub fn select_thread(&self, view: &ViewId, thread: &ChannelId) -> Result<(ViewId, String), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let info = self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(thread).map_err(|e| e.to_string())?.ok_or("This thread is unavailable.")?;
        let changed = self.select_thread_inner(view, &info)?;
        Ok((changed, info.parent_channel_id.0))
    }
    /// Show a thread of the selected channel beside it, loading its session first.
    pub fn open_thread(&self, view: &ViewId, thread: &ChannelId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.open_thread_inner(view, thread)
    }
    fn open_thread_inner(&self, view: &ViewId, thread: &ChannelId) -> Result<(), String> {
        self.load_thread(thread)?;
        self.core.open_thread(view, &thread.0)
    }
    /// Start or restart the session of a thread when it has none that works.
    fn load_thread(&self, thread: &ChannelId) -> Result<(), String> {
        let channel = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            if store.thread_info(thread).map_err(|e| e.to_string())?.is_none() { return Err("This thread is unavailable.".into()); }
            store.load_channel(thread).map_err(|e| e.to_string())?
        };
        if !self.core.has_channel(&thread.0) { self.start_mode(&channel, false) }
        else if self.core.channel_session_closed(&thread.0) { self.start_mode(&channel, true) }
        else { Ok(()) }
    }
    /// Start the engine of a new sub-agent thread and send its brief. The thread loads as a parked thread: it does
    /// not open in the panel, it does not become followed, and its parent need not be the selected channel. It
    /// stays loaded when the run ends, so the user can open it later. It never takes another message.
    pub(crate) fn start_sub_agent_thread(&self, thread: &Channel, brief: String, author: discard_core::model::MessageAuthor) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        // The caller is a worker thread. The launcher spawns the engine on the runtime.
        let _runtime = self.runtime.enter();
        self.start_mode(thread, false)?;
        self.core.send_brief(&thread.id.0, brief, author)
    }
    /// End a sub-agent thread whose task could not start: it is `Failed` in storage, and the thread shows why. The
    /// parked runtime and its engine go, so a failed start leaves nothing running.
    pub(crate) fn fail_sub_agent_thread(&self, thread: &ChannelId, error: &str) {
        let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).finish_sub_agent_thread(thread, discard_core::model::ThreadState::Failed);
        if self.core.has_channel(&thread.0) {
            self.core.apply_channel(&thread.0, Update::Error(error.to_owned()));
            self.core.apply_channel(&thread.0, Update::SubAgentFinished { state: discard_core::model::ThreadState::Failed, text: String::new() });
            self.core.drop_thread_runtime(&thread.0);
        }
    }
    /// Read the threads and reference rows of a channel from storage into the app state.
    pub fn refresh_threads(&self, channel: &ChannelId) -> Result<(), String> {
        let (records, posted) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            if store.thread_info(channel).map_err(|e| e.to_string())?.is_some() { return Ok(()); }
            // A sub-agent thread has no anchor while its parent turn runs. With no turn running, a thread that is
            // still unanchored lost its parent turn (a crash): hang it under the user message it came from.
            if !self.core.channel_running(&channel.0) { store.anchor_sub_agent_threads(channel, None).map_err(|e| e.to_string())?; }
            let mut records = Vec::new();
            for info in store.threads_for_channel(channel).map_err(|e| e.to_string())? {
                let messages = store.messages_for_channel(&info.channel_id).map_err(|e| e.to_string())?;
                let authors = store.message_authors(&info.channel_id).map_err(|e| e.to_string())?;
                records.push(ThreadRecord::build(&info, &messages, &authors));
            }
            (records, store.posted_sources(channel).map_err(|e| e.to_string())?)
        };
        self.core.set_channel_threads(&channel.0, records);
        if self.core.has_channel(&channel.0) { self.core.set_posted(&channel.0, posted); }
        Ok(())
    }
    /// The followed threads of one channel, for the channel bar.
    pub fn thread_links(&self, channel: &ChannelId) -> Result<Vec<super::dto::ThreadLinkDto>, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        super::threads::stored_links(&store, channel, |id| self.core.last_read(id))
    }
    /// Start a thread on a saved message of the selected channel. The thread row, its session and its first
    /// message come together: a failure before the message is queued leaves no thread behind. The new thread opens
    /// beside the channel. Returns the thread channel ID.
    pub fn start_thread(&self, view: &ViewId, parent: &str, anchor: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>, also_post: bool) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        if text.trim().is_empty() && attachments.is_empty() { return Err("Enter a message to start the thread.".into()); }
        if self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(&ChannelId(parent.to_owned())).map_err(|e| e.to_string())?.is_some_and(|thread| thread.kind == discard_core::model::ThreadKind::SubAgent) {
            return Err("A sub-agent thread cannot start another thread.".into());
        }
        self.core.can_start_thread(parent)?;
        let (parent_id, anchor_id) = (ChannelId(parent.to_owned()), MessageId(anchor.to_owned()));
        let (thread, recipient) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let parent_channel = store.load_channel(&parent_id).map_err(|e| e.to_string())?;
            let default = discard_core::mentions::anchor_agent(&store, &parent_channel, &anchor_id)?;
            let recipient = discard_core::mentions::resolve(&store, &parent_id, &text)?.unwrap_or_else(|| default.clone());
            if store.agent_archived(&recipient).map_err(|e| e.to_string())? { return Err("This agent is archived. Restore it before sending a message.".into()); }
            let config = store.load_agent(&recipient).map_err(|e| e.to_string())?;
            let recipient = discard_core::model::MessageAuthor { agent_id: recipient, name: config.name, provider: config.provider, avatar: config.avatar };
            (store.create_thread(&parent_id, &anchor_id, &discard_core::store::thread_name(&text), &default).map_err(|e| e.to_string())?, recipient)
        };
        let sent = self.start_mode(&thread, false).and_then(|()| self.core.send_in_thread(&thread.id.0, text, attachments, also_post, Some(recipient)));
        if let Err(error) = sent {
            self.core.remove_thread(&thread.id.0);
            let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).discard_empty_thread(&thread.id);
            return Err(error);
        }
        self.core.open_thread(view, &thread.id.0)?;
        Ok(thread.id.0)
    }
    /// Show a thread message in the thread's parent channel. Posting again changes nothing. Returns the ID of the
    /// parent's reference row.
    pub fn post_thread_message(&self, message: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (reference, source) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let (reference, _) = store.post_thread_message(&MessageId(message.to_owned())).map_err(|e| e.to_string())?;
            let source = store.posted_sources(&reference.channel_id).map_err(|e| e.to_string())?.remove(&reference.id).ok_or("The posted message is unavailable.")?;
            (reference, source)
        };
        self.load_thread(&source.thread_channel_id)?;
        let id = reference.id.0.clone();
        self.core.message_posted(&source.thread_channel_id.0.clone(), reference, source);
        Ok(id)
    }
    /// Follow or leave a thread. A followed thread shows in the channel bar.
    pub fn set_thread_followed(&self, thread: &ChannelId, followed: bool) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.store.lock().unwrap_or_else(|e| e.into_inner()).set_thread_followed(thread, followed).map_err(|e| e.to_string())?;
        self.core.set_thread_followed(&thread.0, followed);
        Ok(())
    }
    /// Give a thread a new name. Returns the stored name: trimmed, with line breaks changed to spaces.
    pub fn rename_thread(&self, thread: &ChannelId, name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let name = self.store.lock().unwrap_or_else(|e| e.into_inner()).rename_thread(thread, name).map_err(|e| e.to_string())?;
        self.core.thread_renamed(&thread.0, &name);
        Ok(name)
    }
    /// How many messages of a thread were posted to its channel. The delete dialog says how many stay.
    pub fn thread_posted_count(&self, thread: &ChannelId) -> Result<usize, String> {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).posted_message_count(thread).map_err(|e| e.to_string())
    }
    /// Delete a thread and everything it owns. A message that was posted to the parent stays there as a normal
    /// message. Returns how many messages stayed. Refused while a turn runs in the thread, while its sub-agent task
    /// runs, and while an approval waits. A loaded provider session closes first.
    pub fn delete_thread(&self, thread: &ChannelId) -> Result<usize, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        if self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(thread).map_err(|e| e.to_string())?.is_none() { return Err("This thread is unavailable.".into()); }
        self.core.begin_thread_delete(&thread.0)?;
        let result = (|| {
            self.core.drop_thread_runtime(&thread.0);
            let deletion = self.store.lock().unwrap_or_else(|e| e.into_inner()).delete_thread(thread).map_err(|e| e.to_string())?;
            self.core.thread_deleted(&thread.0);
            Ok(deletion.posted.len())
        })();
        self.core.end_channel_change(&thread.0);
        result
    }
    /// Remove a posted reference row from its channel. The message stays in its thread and can be posted again.
    pub fn remove_posted_message(&self, reference: &str) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let reference = MessageId(reference.to_owned());
        let source = self.store.lock().unwrap_or_else(|e| e.into_inner()).remove_posted_reference(&reference).map_err(|e| e.to_string())?;
        self.core.reference_removed(&reference, &source);
        Ok(())
    }
    /// Rename a text channel with the checks of channel creation. Returns the stored name.
    pub fn rename_text_channel(&self, project: &ProjectId, channel: &ChannelId, name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let renamed = self.store.lock().unwrap_or_else(|e| e.into_inner()).rename_text_channel(project, channel, name).map_err(|e| e.to_string())?;
        self.core.channel_renamed(&channel.0, &renamed.name);
        Ok(renamed.name)
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
            let threads = super::threads::stored_links(&store, &channel.id, |id| self.core.last_read(id))?;
            let has_messages = store.channel_has_messages(&channel.id).map_err(|e| e.to_string())?;
            let muted = store.channel_muted(&channel.id).map_err(|e| e.to_string())?;
            Ok(super::dto::ProjectChannelDto {
                worktree, has_messages, muted,
                id: channel.id.0, project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: metadata.kind.as_str().into(),
                repository: metadata.repository, branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
                threads,
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
    pub fn select_canonical_branch(&self, view: &ViewId, project: &ProjectId, branch: &str, channel_id: &ChannelId) -> Result<(), String> {
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
            // Every view that selected the old canonical channel moves to the new one (decision 22).
            if let Some(old) = previous.as_ref().filter(|old| old.id != target.id) && !self.core.views_selecting(&old.id.0).is_empty() {
                if !self.core.has_channel(&updated.id.0) { self.start(&updated)?; }
                return self.move_views(view, &old.id.0, &updated.id.0);
            }
            self.refresh_branches_of(view, &[]);
            Ok(())
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    pub fn select_canonical_general(&self, view: &ViewId, project: &ProjectId, channel_id: &ChannelId) -> Result<(), String> {
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
            // Every view that selected the old canonical channel moves to the new one (decision 22).
            match previous.as_ref().filter(|old| old.id != updated.id) {
                Some(old) => self.move_views(view, &old.id.0, &updated.id.0),
                None => { self.refresh_branches_of(view, &[]); Ok(()) }
            }
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    /// Move every view that selects channel `from` to channel `to`. A view that cannot move does not stop the others.
    /// The views that moved, and `view` that ran the action, reload their branches before the first error returns.
    fn move_views(&self, view: &ViewId, from: &str, to: &str) -> Result<(), String> {
        let (mut moved, mut failure) = (Vec::new(), None);
        for shown in self.core.views_selecting(from) {
            match self.select_inner(&shown, to) { Ok(_) => moved.push(shown), Err(error) => { failure.get_or_insert(error); } }
        }
        self.refresh_branches_of(view, &moved);
        failure.map_or(Ok(()), Err)
    }
    /// Reload the branches of the view that ran an action and of the views that the action moved.
    fn refresh_branches_of(&self, view: &ViewId, moved: &[ViewId]) {
        self.core.refresh_branches_in_background(view);
        for other in moved.iter().filter(|other| *other != view) { self.core.refresh_branches_in_background(other); }
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
        let threads = super::threads::stored_links(&store, &channel.id, |id| self.core.last_read(id))?;
        let has_messages = store.channel_has_messages(&channel.id).map_err(|e| e.to_string())?;
        Ok(super::dto::ChannelDetailsDto {
            channel: super::dto::ProjectChannelDto {
                has_messages,
                muted: store.channel_muted(&channel.id).map_err(|e| e.to_string())?,
                worktree: store.worktree_for(&channel.id).map_err(|e| e.to_string())?
                    .map(|record| super::dto::ChannelWorktreeDto { path: record.path, branch: record.branch }),
                id: channel.id.0.clone(), project_id: project.0.clone(), name: channel.name,
                agent_id: channel.agent_id.0, workspace_mode: channel.workspace_mode.as_str().into(),
                kind: metadata.kind.as_str().into(),
                repository: metadata.repository, branch: metadata.branch, archived, archived_into: metadata.archived_into.map(|id| id.0),
                threads,
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
        // The active general channel and every active text channel follow the project main agent.
        let channels = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let mut channels = Vec::new();
            let mut general = None;
            for (candidate, archived) in store.all_channels_for_project(project).map_err(|e| e.to_string())? {
                if archived { continue; }
                let kind = store.project_channel_metadata(&candidate.id).map_err(|e| e.to_string())?.map(|m| m.kind);
                if kind == Some(discard_core::model::ProjectChannelKind::General) { general = Some(candidate.id.clone()); }
                if matches!(kind, Some(discard_core::model::ProjectChannelKind::General | discard_core::model::ProjectChannelKind::Text)) { channels.push(candidate); }
            }
            let general = match general {
                Some(general) => general,
                None => { let created = store.ensure_project_general_channel(project, agent).map_err(|e| e.to_string())?; let id = created.id.clone(); channels.insert(0, created); id }
            };
            (channels, general)
        };
        let (channels, general) = channels;
        let mut guarded: Vec<String> = channels.iter().map(|channel| channel.id.0.clone()).collect();
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
            self.store.lock().unwrap_or_else(|e| e.into_inner()).set_project_main_agent(project, agent).map_err(|e| e.to_string())?;
            let mut failure = None;
            for channel in &channels {
                let mut updated_identity = channel.clone();
                updated_identity.agent_id = agent.clone();
                if self.core.has_channel(&updated_identity.id.0) {
                    self.core.invalidate_idle_channel(&updated_identity)?;
                }
                let updated = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&updated_identity.id).map_err(|e| e.to_string())?;
                let restarted = if self.core.has_channel(&updated.id.0) {
                    self.start_mode(&updated, true).inspect_err(|error| {
                        self.core.append_channel_notice(&updated.id.0, format!("The project main agent changed, but its channel session could not restart: {error}. Select this channel again to retry."));
                    })
                } else if channel.id == general { self.start(&updated) }
                else { Ok(()) };
                if let Err(error) = restarted { failure.get_or_insert(error); }
            }
            failure.map_or(Ok(()), Err)
        })();
        for id in begun { self.core.end_channel_change(&id); }
        result
    }
    fn select_inner(&self, view: &ViewId, id: &str) -> Result<ViewId, String> { self.select_inner_mode(view, id, false) }
    /// Select a channel in a view. Every selection of a channel ends here, so this is where a project view stays fixed
    /// to its project: a channel of another project, or a direct message, changes the main view instead. Returns the
    /// view that changed.
    fn select_inner_mode(&self, view: &ViewId, id: &str, project_change: bool) -> Result<ViewId, String> {
        let (channel, project) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let channel = store.load_channel(&ChannelId(id.to_owned())).map_err(|e| e.to_string())?;
            let project = channel.project_id.as_ref().and_then(|id| store.list_projects().ok()?.into_iter().find(|p| p.id == *id));
            (channel, project)
        };
        let view = &self.routed(view, channel.project_id.as_ref())?;
        if project_change { self.core.can_switch_for_project(view)?; } else { self.core.can_switch(view)?; }
        let restart = self.core.has_channel(id) && self.core.channel_session_closed(id);
        if !self.core.has_channel(id) { self.start(&channel)?; }
        if project_change { self.core.switch_channel_for_project(view, id)?; } else { self.core.switch_channel(view, id)?; }
        self.sync_muted(&channel.id);
        if let Some(project) = project { self.core.configure_branch_source(view, self.store.clone(), channel.id.clone(), channel.workspace_mode, project.path.into()); }
        else { self.core.clear_branch_source(view); }
        self.refresh_threads(&channel.id)?;
        self.core.clear_slots();
        // The rows are in state already. Mark the selected row from them. Git runs only in a refresh.
        self.core.reselect_current_branch(view);
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
        Ok(view.clone())
    }
    /// Restart the loaded channels of a project after a folder change. Every view of the project gets the new path and
    /// branch source, not only the view that ran the change.
    pub fn reload_project(&self, id: &ProjectId) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channels: Vec<_> = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let conversations: Vec<_> = store.all_conversations().map_err(|e| e.to_string())?.into_iter().filter(|c| c.project_id.as_ref() == Some(id)).collect();
            // Threads are not conversations, but their sessions end with the project's transports too.
            let mut threads = Vec::new();
            for conversation in &conversations {
                for thread in store.threads_for_channel(&conversation.id).map_err(|e| e.to_string())? { threads.push(store.load_channel(&thread.channel_id).map_err(|e| e.to_string())?); }
            }
            conversations.into_iter().chain(threads).filter(|c| self.core.has_channel(&c.id.0)).collect()
        };
        self.core.invalidate_project_channels(id)?;
        for channel in channels { self.start_mode(&channel, true)?; }
        let project = self.store.lock().unwrap_or_else(|e| e.into_inner()).list_projects().map_err(|e| e.to_string())?.into_iter().find(|p| p.id == *id);
        let mut failure = None;
        for shown in self.core.view_ids() {
            let browses = self.core.current_project(&shown).id == *id;
            let selects = self.core.active_context(&shown).1.as_ref() == Some(id);
            if !browses && !selects { continue; }
            if browses && let Some(project) = &project { self.core.select_sidebar_project(&shown, Some(project)); }
            if selects && let Err(error) = self.select_inner_mode(&shown, &self.core.active_channel_id(&shown), true) { failure.get_or_insert(error); }
            self.core.refresh_branches_in_background(&shown);
        }
        failure.map_or(Ok(()), Err)
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
        for view in self.core.view_ids() { self.core.refresh_branches_in_background(&view); }
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
    /// `/clear`: the next turn of the channel starts a new provider session with no earlier context. The transcript
    /// keeps its messages and shows a marker. A thread has no session of its own, so it is refused.
    pub fn clear_context(&self, channel_id: &str) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let id = ChannelId(channel_id.to_owned());
        let (channel, thread) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            (store.load_channel(&id).map_err(|e| e.to_string())?, store.thread_info(&id).map_err(|e| e.to_string())?)
        };
        if thread.is_some() { return Err("/clear does not apply to a thread.".into()); }
        if self.core.channel_running(channel_id) { return Err("Stop the turn before clearing the session.".into()); }
        let marker = self.store.lock().unwrap_or_else(|e| e.into_inner()).reset_channel_context(&id).map_err(|e| e.to_string())?;
        self.core.apply_channel(channel_id, discard_core::ui_state::Update::ConversationMarker(marker));
        // Close the idle provider transport of the channel and open a new one.
        if self.core.invalidate_idle_channel(&channel)? { self.start_mode(&channel, true)?; }
        Ok(())
    }
    /// `/model`: save a new model for the agent through the same checks as the agent editor, then refresh every channel
    /// of the agent. A turn reads the model of its agent when it starts, so the next turn uses the new one. The change
    /// is refused while a turn of the agent runs.
    pub fn change_agent_model(&self, agent: &discard_core::model::AgentId, model: &str, matrix: &discard_core::capability::CapabilityMatrix) -> Result<discard_core::agent::AgentConfig, String> {
        let name = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_agent(agent).map_err(|e| e.to_string())?.name;
        if self.core.agent_running(&agent.0) { return Err(format!("Stop the turn of {name} before changing its model.")); }
        self.core.begin_agent_change(&agent.0)?;
        let result = (|| {
            let saved = discard_core::agent_edit::change_model(&self.store.lock().unwrap_or_else(|e| e.into_inner()), agent, model, matrix).map_err(|error| match error {
                discard_core::agent_edit::SaveError::Unchanged => format!("{name} already uses {model}."),
                other => other.to_string(),
            })?;
            self.reload_agent(&agent.0)?;
            Ok(saved)
        })();
        self.core.end_agent_change(&agent.0);
        result
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
    pub fn open_agent(&self, view: &ViewId, agent: &discard_core::model::AgentId) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).default_conversation(agent).map_err(|e| e.to_string())?;
        self.select_inner(view, &channel.id.0)?; Ok(channel.id.0)
    }
    pub fn create_conversation(&self, view: &ViewId, agent: &discard_core::model::AgentId, project: Option<&ProjectId>, name: &str) -> Result<String, String> {
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
            self.select_inner(view, &channel.id.0)?;
            return Ok(channel.id.0);
        }
        self.core.can_switch(view)?;
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).create_conversation(agent, project, name).map_err(|e| e.to_string())?;
        self.select_inner(view, &channel.id.0)?; Ok(channel.id.0)
    }
    pub fn create(&self, view: &ViewId, name: &str, mode: &str, source_channel: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch(view)?;
        if self.core.active_channel_id(view) != source_channel { return Err("The channel changed. Open New channel again.".into()); }
        if self.core.current_project(view).id.0.is_empty() { return Err("Select a project first.".into()); }
        let _ = (name, mode);
        Err("Open a Git branch to start another project channel.".into())
    }
    /// Create a text channel for the current project, start it, and select it. Returns the channel ID.
    pub fn create_text_channel(&self, view: &ViewId, project: &ProjectId, name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch(view)?;
        if self.core.current_project(view).id != *project { return Err("Open this project before you add a channel.".into()); }
        let channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).create_project_text_channel(project, name).map_err(|e| e.to_string())?;
        self.start(&channel)?;
        self.select_inner(view, &channel.id.0)?;
        // A new channel changes the list. Refresh it without a wait.
        self.core.refresh_branches_in_background(view);
        Ok(channel.id.0)
    }
    fn create_inner(&self, view: &ViewId, name: String, row: &discard_core::branch_tree::TreeRow, repository: &str, repository_path: &std::path::Path) -> Result<String, String> {
        let project = self.core.current_project(view).id;
        let agent_id = self.store.lock().unwrap_or_else(|e| e.into_inner()).project_main_agent(&project).map_err(|e| e.to_string())?
            .ok_or("This project has no main agent. Open the project first.")?;
        let channel = Channel { id: ChannelId::new(), project_id: Some(project), name, agent_id, workspace_mode: WorkspaceMode::Worktree };
        let record = row.worktree.as_ref().map(|path| -> Result<WorktreeRecord, String> {
            let manager = WorktreeManager::new(&self.project_dir(view));
            let prepared = manager.recorded(repository_path, path, &row.name).map_err(|e| e.to_string())?;
            Ok(WorktreeRecord { channel_id: channel.id.clone(), path: path.to_string_lossy().into_owned(), branch: prepared.worktree.branch,
                base_commit: prepared.worktree.base_commit, repo: repository_path.to_string_lossy().into_owned(), owned: false })
        }).transpose()?;
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let existing = store.channels_for_project(&self.core.current_project(view).id).map_err(|e| e.to_string())?;
            if existing.iter().any(|c| c.name.eq_ignore_ascii_case(&channel.name)) { return Err("A channel with this name already exists.".into()); }
            // A detached commit is not a branch and must not claim a canonical branch channel.
            let branch = (!row.detached).then_some(row.name.as_str());
            store.create_named_repository_channel(&channel, repository, branch, record.as_ref()).map_err(|e| e.to_string())?;
            store.register_bootstrap_project_channel(&channel.id).map_err(|e| e.to_string())?;
        }
        self.start(&channel)?;
        self.select_inner(view, &channel.id.0)?;
        // A new channel changes the list. Refresh it without a wait.
        self.core.refresh_branches_in_background(view);
        Ok(channel.id.0)
    }
    #[expect(clippy::too_many_arguments, reason = "The services and the fields of the request come from the command.")]
    pub fn decide_write_request(&self, view: &ViewId, requests: &discard_core::write_requests::WriteRequests, workspace: &discard_core::workspace::GitWorkspace, grants: &discard_core::workspace_grants::MainCheckoutGrants, channel: &str, id: &str, approve: bool) -> Result<discard_core::write_requests::WriteRequest, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        if approve {
            if self.core.active_channel_id(view) != channel { return Err("Open this channel before approving.".into()); }
            self.core.begin_write_approval(channel)?;
        }
        let result: Result<_, String> = (|| {
            let channel_id = ChannelId(channel.into());
            if approve { self.refuse_busy_move_target(requests, &channel_id, id)?; }
            let request = requests.begin(&channel_id, id, approve)?;
            let mut note = None;
            let request = if !approve { request } else {
                use discard_core::write_requests::WriteRequestKind::{ConversationMove, MainCheckoutWrite, WorktreeCreate};
                match request.kind {
                    ConversationMove => {
                        let action = self.approve_move(view, workspace, &channel_id, &request).map_err(|error| format!("Conversation move failed: {error}"));
                        requests.finish(id, action)
                    }
                    WorktreeCreate | MainCheckoutWrite => {
                        let action = (|| {
                            let project = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(&channel_id).map_err(|e| e.to_string())?.project_id;
                            let (repo, branch, path) = workspace.write_request_target(&channel_id, &AgentId(request.agent_id.clone()), project.as_ref(), request.kind)?;
                            if request.branch != branch || std::path::Path::new(&request.path) != path { return Err("The request target changed. Ask again.".into()); }
                            match request.kind {
                                WorktreeCreate => {
                                    let record = workspace.create_for_branch(&repo, &channel_id, &branch)?;
                                    note = Some(format!("Discard created a worktree for {branch} at {}.", record.path));
                                    Ok(format!("Created a worktree for `{branch}` at `{}`.", record.path))
                                }
                                MainCheckoutWrite => {
                                    note = Some("The user allowed writes in the main checkout for one response.".into());
                                    Ok("The user allowed writes in the main checkout for one response.".into())
                                }
                                ConversationMove => Err("Conversation moves use their own approval path.".into()),
                            }
                        })().map_err(|error: String| match request.kind {
                            WorktreeCreate => format!("Worktree creation failed: {error}"),
                            MainCheckoutWrite => format!("Main-checkout write access failed: {error}"),
                            ConversationMove => format!("Conversation move failed: {error}"),
                        });
                        requests.finish(id, action)
                    }
                }
            };
            // A move that succeeded leaves a link in the source, not a note.
            let moved = request.kind == discard_core::write_requests::WriteRequestKind::ConversationMove && request.state == "completed";
            if !moved {
                let message = discard_core::model::Message {
                    id: discard_core::model::MessageId::new(),
                    channel_id,
                    role: discard_core::model::Role::System,
                    blocks: vec![discard_core::model::Block::Markdown { text: request.result.clone().unwrap_or_else(|| request.state.clone()) }],
                };
                self.store.lock().unwrap_or_else(|e| e.into_inner()).insert_message(&message).map_err(|e| e.to_string())?;
                self.core.apply_channel(channel, discard_core::ui_state::Update::ConversationMarker(message));
            }
            self.core.refresh_branches_in_background(view);
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

    /// Before an approval takes a pending move request, refuse a target channel that is loaded and busy. The request
    /// stays pending, so the user can approve it again when the channel is idle.
    fn refuse_busy_move_target(&self, requests: &discard_core::write_requests::WriteRequests, source: &ChannelId, id: &str) -> Result<(), String> {
        use discard_core::write_requests::{MoveDetails, MoveTarget};
        let request = requests.list(source).into_iter().find(|request| request.id == id && request.state == "pending");
        let Some(MoveDetails { target: MoveTarget::Channel { channel_id, name }, .. }) = request.and_then(|request| request.details) else { return Ok(()) };
        if !self.core.has_channel(&channel_id) { return Ok(()); }
        match self.core.channel_busy_reason(&channel_id) {
            Some(reason) => Err(format!("#{name} is busy: {reason}")),
            None => Ok(()),
        }
    }
    /// Carry out an approved move. Every check that can fail runs before the first message of the target is saved. A
    /// new branch, its worktree, and its channel exist before that point by necessity. Returns the result text.
    fn approve_move(&self, view: &ViewId, workspace: &discard_core::workspace::GitWorkspace, source: &ChannelId, request: &discard_core::write_requests::WriteRequest) -> Result<String, String> {
        use discard_core::model::{Block, LinkDirection, Message, Role};
        let details = request.details.clone().ok_or("The request has no move details. Ask again.")?;
        let agent = AgentId(request.agent_id.clone());
        let source_channel = self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(source).map_err(|e| e.to_string())?;
        // The workspace takes the store lock itself.
        workspace.move_source(source, &agent, source_channel.project_id.as_ref())?;
        let (text, attachments) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let message = store.messages_for_channel(source).map_err(|e| e.to_string())?.into_iter()
                .find(|message| message.id.0 == details.source_message_id && message.role == Role::User)
                .ok_or("The message to move is no longer available. Ask again.")?;
            // The card says that the latest message moves. A newer user message makes the request stale.
            if store.last_user_message_id(source).map_err(|e| e.to_string())?.is_some_and(|latest| latest != message.id) { return Err("You sent another message after this request. Ask for the move again.".into()); }
            let text = message.blocks.iter().filter_map(|block| match block { Block::Markdown { text } => Some(text.as_str()), _ => None }).collect::<Vec<_>>().join("\n\n");
            // The handoff carries the files that the agent named. The user's own message does not move.
            let attachments = discard_core::write_requests::load_source_attachments(&store, source, &details.attachments)?;
            (text, attachments)
        };
        let project = source_channel.project_id.clone().ok_or("Conversation moves require this conversation's project.")?;
        let target = self.resolve_move_target(view, workspace, &source_channel, &project, &agent, &details)?;
        let (recipient, moderator) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let id = discard_core::mentions::recipient(&store, &target.id, &text)?;
            if store.agent_archived(&id).map_err(|e| e.to_string())? { return Err("This agent is archived. Restore it before sending a message.".into()); }
            let config = store.load_agent(&id).map_err(|e| e.to_string())?;
            let recipient = discard_core::model::MessageAuthor { agent_id: id, name: config.name, provider: config.provider, avatar: config.avatar };
            let config = store.load_agent(&agent).map_err(|e| e.to_string())?;
            (recipient, discard_core::model::MessageAuthor { agent_id: agent.clone(), name: config.name, provider: config.provider, avatar: config.avatar })
        };
        // The send runs after the inserts. Check it first, so a refused send leaves nothing in the target.
        self.core.can_send_in_background(&target.id.0, Some(&recipient))?;
        let link = Message { id: MessageId::new(), channel_id: target.id.clone(), role: Role::System, blocks: vec![Block::ChannelLink { direction: LinkDirection::MovedFrom, channel_id: source.0.clone(), channel_name: source_channel.name.clone() }] };
        let handoff = Message { id: MessageId::new(), channel_id: target.id.clone(), role: Role::Agent, blocks: std::iter::once(Block::Markdown { text: format!("Handoff from #{}\n\n{}", source_channel.name, details.handoff.markdown()) }).chain(attachments.into_iter().map(|attachment| Block::Attachment { attachment })).collect() };
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            store.insert_message(&link).map_err(|e| e.to_string())?;
            store.insert_authored_message(&handoff, &moderator).map_err(|e| e.to_string())?;
        }
        let handoff_id = handoff.id.clone();
        self.core.apply_channel(&target.id.0, discard_core::ui_state::Update::ConversationMarker(link));
        self.core.apply_channel(&target.id.0, discard_core::ui_state::Update::ConversationMarker(handoff));
        self.core.set_message_author(&target.id.0, handoff_id, moderator);
        // The user's message asked for the move. Do not copy it: the target agent would read it as a new request to
        // move. The handoff and its attachments carry the task.
        let note = format!("the conversation moved here from #{}. This channel is the new place for the work, so do not move again. The handoff above holds the latest request.", source_channel.name);
        self.core.continue_in_background(&target.id.0, recipient, note)?;
        let moved = Message { id: MessageId::new(), channel_id: source.clone(), role: Role::System, blocks: vec![Block::ChannelLink { direction: LinkDirection::MovedTo, channel_id: target.id.0.clone(), channel_name: target.name.clone() }] };
        // The target has the conversation now. A storage error on the link must not fail the move.
        match self.store.lock().unwrap_or_else(|e| e.into_inner()).insert_message(&moved) {
            Ok(()) => { self.core.apply_channel(&source.0, discard_core::ui_state::Update::ConversationMarker(moved)); }
            Err(error) => tracing::warn!(%error, "could not save the link of a moved conversation"),
        }
        Ok(format!("Moved to #{}.", target.name))
    }
    /// The loaded channel that a move goes to: an active channel of the project, or a new branch channel with its
    /// branch and worktree.
    fn resolve_move_target(&self, view: &ViewId, workspace: &discard_core::workspace::GitWorkspace, source: &Channel, project: &ProjectId, agent: &AgentId, details: &discard_core::write_requests::MoveDetails) -> Result<Channel, String> {
        use discard_core::write_requests::MoveTarget;
        match &details.target {
            MoveTarget::Channel { channel_id, .. } => {
                let found = self.store.lock().unwrap_or_else(|e| e.into_inner()).all_channels_for_project(project).map_err(|e| e.to_string())?
                    .into_iter().find(|(channel, archived)| channel.id.0 == *channel_id && channel.id != source.id && !archived).map(|(channel, _)| channel)
                    .ok_or("The target channel is no longer available.")?;
                self.start(&found)?;
                Ok(found)
            }
            MoveTarget::NewBranch { repository, repo_path, branch, base_commit, .. } => {
                let repo = std::path::Path::new(repo_path);
                let label = if repository.is_empty() { None } else { Some(self.repository(view, project, repository)?.label) };
                discard_core::branch_actions::validate_branch_name(repo, branch, None)?;
                let name = {
                    let existing = self.store.lock().unwrap_or_else(|e| e.into_inner()).channels_for_project(project).map_err(|e| e.to_string())?;
                    let base = label.map_or_else(|| branch.clone(), |label| format!("{label}/{branch}"));
                    let (mut name, mut suffix) = (base.clone(), 2);
                    while existing.iter().any(|channel| channel.name.eq_ignore_ascii_case(&name)) { name = format!("{base}-{suffix}"); suffix += 1; }
                    name
                };
                if !discard_core::branch_actions::commit_exists(repo, base_commit) { return Err("The base commit is no longer available. Ask again.".into()); }
                discard_core::branch_actions::create_branch_at(repo, branch, base_commit)?;
                let channel = Channel { id: ChannelId::new(), project_id: Some(project.clone()), name, agent_id: agent.clone(), workspace_mode: WorkspaceMode::Worktree };
                if let Err(error) = workspace.create_channel_with_worktree(repo, repository, &channel, branch) {
                    // A failed worktree leaves no branch behind.
                    if let Err(rollback) = discard_core::branch_actions::delete_branch(repo, branch, true) { tracing::warn!(error = %rollback, branch = %branch, "could not remove the branch of a failed conversation move"); }
                    return Err(error);
                }
                self.start(&channel)?;
                self.store.lock().unwrap_or_else(|e| e.into_inner()).save_channel_topic(&channel.id, 0, &details.handoff.markdown(), discard_core::model::TopicAuthorKind::Agent, Some(agent)).map_err(|e| e.to_string())?;
                Ok(channel)
            }
        }
    }

    /// Decide a request to add an agent to the project. Adding makes the agent a project member and continues the
    /// requesting agent in the channel; denying continues it with the note that the user declined. The channel must
    /// be the selected channel and idle, as for a write request, so a decision never saves without its follow-up.
    /// The transcript gets a note of the result.
    pub fn decide_agent_request(&self, requests: &discard_core::agent_requests::AgentRequests, channel: &str, id: &str, approve: bool) -> Result<discard_core::agent_requests::AgentAddRequest, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_continue_in(channel)?;
        let channel_id = ChannelId(channel.into());
        let decision = requests.decide(&self.store.lock().unwrap_or_else(|e| e.into_inner()), &channel_id, id, approve)?;
        let message = discard_core::model::Message {
            id: discard_core::model::MessageId::new(), channel_id,
            role: discard_core::model::Role::System,
            blocks: vec![discard_core::model::Block::Markdown { text: decision.request.result.clone().unwrap_or_else(|| decision.request.state.clone()) }],
        };
        // The note is a courtesy to the reader. A storage error must not leave the agent without its follow-up.
        match self.store.lock().unwrap_or_else(|e| e.into_inner()).insert_message(&message) {
            Ok(()) => { self.core.apply_channel(channel, Update::ConversationMarker(message)); }
            Err(error) => tracing::warn!(%error, "could not save the note of an agent request"),
        }
        if let Some(follow_up) = decision.follow_up {
            if follow_up.declined { self.core.continue_declined_in(channel, follow_up.agent, follow_up.note)?; }
            else { self.core.continue_in(channel, follow_up.agent, follow_up.note)?; }
        }
        Ok(decision.request)
    }

    pub fn continue_after_approval(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.continue_in(channel, agent, note)
    }

    pub fn send_message(&self, channel: &str, text: String) -> Result<(), String> {
        self.send_with_attachments(channel, text, Vec::new())
    }
    pub fn send_with_attachments(&self, channel: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>) -> Result<(), String> {
        self.send_with_options(channel, text, attachments, false)
    }
    /// The agent that answers `text` in `channel`: the mentioned agent, or the agent of the channel.
    fn recipient_of(&self, channel: &str, text: &str) -> Result<discard_core::model::MessageAuthor, String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let id = discard_core::mentions::recipient(&store, &ChannelId(channel.into()), text)?;
        if store.agent_archived(&id).map_err(|e| e.to_string())? { return Err("This agent is archived. Restore it before sending a message.".into()); }
        let config = store.load_agent(&id).map_err(|e| e.to_string())?;
        Ok(discard_core::model::MessageAuthor { agent_id: id, name: config.name, provider: config.provider, avatar: config.avatar })
    }
    /// Take the operation lock. A caller that holds it serialises with every channel operation, and posts with
    /// `send_to_channel`.
    pub fn operation_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.operation.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Send a user message to a channel the way the composer does. The guard proves that the caller holds the
    /// operation lock, as `send_with_options` does. The store lock must be free.
    pub fn send_to_channel(&self, _operation: &std::sync::MutexGuard<'_, ()>, channel: &str, text: String) -> Result<(), String> {
        let recipient = self.recipient_of(channel, &text)?;
        self.core.send_to_agent(Some(channel), text, Some(recipient))
    }
    /// Send a user message. In a thread, `also_post` also posts it to the parent channel. Anywhere else it is an error.
    pub fn send_with_options(&self, channel: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>, also_post: bool) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let thread_info = self.store.lock().unwrap_or_else(|e| e.into_inner()).thread_info(&ChannelId(channel.into())).map_err(|e| e.to_string())?;
        let thread = thread_info.is_some();
        if also_post && !thread { return Err("Only a message in a thread can be posted to a channel.".into()); }
        let recipient = Some(self.recipient_of(channel, &text)?);
        if thread {
            self.load_thread(&ChannelId(channel.into()))?;
            return self.core.send_in_thread(channel, text, attachments, also_post, recipient);
        }
        self.core.send_to_agent_with_attachments(Some(channel), text, recipient, attachments)
    }

    // The menu actions of the channel list: mute, archive, and the Git actions of branch and worktree rows. Each one
    // checks its own state again, so the webview state is never trusted. Each Git action starts a branch refresh.

    /// Resolve a repository of the current project. The webview names it by ID: empty for the project root.
    fn repository(&self, view: &ViewId, project: &ProjectId, id: &str) -> Result<discard_core::worktree::DiscoveredRepository, String> {
        if self.core.current_project(view).id != *project { return Err("Open this project first.".into()); }
        discard_core::worktree::discover_repositories(&self.project_dir(view)).map_err(|e| e.to_string())?
            .into_iter().find(|repository| repository.id == id).ok_or_else(|| "This repository is no longer available.".to_string())
    }
    fn branch_entries(&self, repository: &discard_core::worktree::DiscoveredRepository) -> Result<Vec<discard_core::worktree::BranchEntry>, String> {
        discard_core::worktree::list_branches(&repository.path).map_err(|e| e.to_string())
    }
    fn branch_channel(&self, project: &ProjectId, repository: &str, branch: &str) -> Result<Option<Channel>, String> {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).project_repository_branch_channel(project, repository, branch).map_err(|e| e.to_string())
    }
    /// Take the channels out of use for one action. Refused, with nothing held, while any of them is busy.
    fn hold(&self, ids: &[ChannelId]) -> Result<Vec<String>, String> {
        let mut held: Vec<String> = Vec::new();
        for id in ids {
            if held.contains(&id.0) { continue; }
            if let Err(error) = self.core.begin_channel_action(&id.0) {
                self.release(held);
                return Err(error);
            }
            held.push(id.0.clone());
        }
        Ok(held)
    }
    fn release(&self, held: Vec<String>) { for id in held { self.core.end_channel_change(&id); } }
    /// Show the new Git state: the branch list and the rows that depend on it.
    fn after_git(&self, view: &ViewId) { self.core.refresh_branches_in_background(view); }
    /// The channels that use a working tree. The first list holds every channel with a saved worktree record at its
    /// path, archived or not. The second holds the active ones among them and the branch channel of the branch that the
    /// tree holds.
    fn worktree_users(&self, project: &ProjectId, repository: &str, entry: &discard_core::worktree::BranchEntry, root: &std::path::Path) -> Result<(Vec<ChannelId>, Vec<ChannelId>), String> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let canonical = root.canonicalize().ok();
        let same = |path: &str| { let path = std::path::Path::new(path); path == root || (canonical.is_some() && path.canonicalize().ok() == canonical) };
        let (mut bound, mut active) = (Vec::new(), Vec::new());
        for owner in store.list_projects().map_err(|e| e.to_string())? {
            for (channel, archived) in store.all_channels_for_project(&owner.id).map_err(|e| e.to_string())? {
                if store.worktree_for(&channel.id).map_err(|e| e.to_string())?.is_some_and(|record| same(&record.path)) {
                    bound.push(channel.id.clone());
                    if !archived { active.push(channel.id); }
                }
            }
        }
        if !entry.detached && let Some(channel) = store.project_repository_branch_channel(project, repository, &entry.branch).map_err(|e| e.to_string())? && !active.contains(&channel.id) {
            active.push(channel.id);
        }
        Ok((bound, active))
    }
    /// Resolve a working tree of a repository: its entry, and the main checkout path that Git commands run in.
    fn working_tree(&self, repository: &discard_core::worktree::DiscoveredRepository, worktree: &str) -> Result<(discard_core::worktree::BranchEntry, PathBuf, PathBuf), String> {
        let entries = self.branch_entries(repository)?;
        let root = PathBuf::from(worktree);
        let entry = discard_core::branch_actions::entry_for_worktree(&entries, &root).cloned().ok_or("This working tree is no longer available.")?;
        let main = entries.iter().find(|candidate| candidate.is_main_checkout).and_then(|candidate| candidate.worktree.clone()).unwrap_or_else(|| repository.path.clone());
        let root = entry.worktree.clone().unwrap_or(root);
        Ok((entry, root, main))
    }

    /// Mute or unmute a channel. A thread cannot be muted. `#general` can.
    pub fn set_channel_muted(&self, project: &ProjectId, channel: &ChannelId, muted: bool) -> Result<(), String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let value = store.load_channel(channel).map_err(|e| e.to_string())?;
            if value.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
            store.set_channel_muted(channel, muted).map_err(|e| e.to_string())?;
        }
        self.core.set_channel_muted(&channel.0, muted);
        Ok(())
    }

    /// Archive the channel of a row with the rules, the confirm step and the store path of the menu bar item. Also
    /// refused for `#general`, for an archived channel, and for a busy channel. Returns the items that a forced archive
    /// would delete when the archive needs the second confirm. When the archived channel was open, `#general` opens.
    pub fn archive_channel(&self, view: &ViewId, workspace: &discard_core::workspace::GitWorkspace, project: &ProjectId, channel: &ChannelId, force: bool) -> Result<super::dto::ArchiveChannelDto, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let threads = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let value = store.load_channel(channel).map_err(|e| e.to_string())?;
            if value.project_id.as_ref() != Some(project) { return Err("Channel does not belong to this project.".into()); }
            if store.thread_info(channel).map_err(|e| e.to_string())?.is_some() { return Err("A thread is archived with its channel.".into()); }
            let metadata = store.project_channel_metadata(channel).map_err(|e| e.to_string())?.ok_or("Project channel metadata is unavailable")?;
            if metadata.kind == discard_core::model::ProjectChannelKind::General { return Err("#general cannot be archived.".into()); }
            if store.channel_archived(channel).map_err(|e| e.to_string())? { return Err("This channel is already archived.".into()); }
            store.threads_for_channel(channel).map_err(|e| e.to_string())?.into_iter().map(|thread| thread.channel_id.0).collect::<Vec<_>>()
        };
        self.core.begin_channel_archive(&channel.0)?;
        let outcome = workspace.archive(channel, force);
        self.core.end_channel_change(&channel.0);
        match outcome? {
            discard_core::workspace::ArchiveOutcome::NeedsConfirmation(items) => Ok(super::dto::ArchiveChannelDto { archived: false, items }),
            discard_core::workspace::ArchiveOutcome::Archived { .. } => {
                self.core.channel_read_only(&channel.0, true, true);
                for thread in threads.iter().filter(|thread| self.core.has_channel(thread)) { self.core.channel_read_only(thread, true, true); }
                // Every view that shows the channel opens `#general` of the project (decision 22).
                let moved: Vec<ViewId> = self.core.views_selecting(&channel.0).into_iter().filter(|shown| self.open_general(shown, project).is_ok()).collect();
                self.refresh_branches_of(view, &moved);
                Ok(super::dto::ArchiveChannelDto { archived: true, items: vec![] })
            }
        }
    }
    /// Open `#general` of a project.
    fn open_general(&self, view: &ViewId, project: &ProjectId) -> Result<(), String> {
        let general = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            store.all_channels_for_project(project).map_err(|e| e.to_string())?.into_iter().filter(|(_, archived)| !archived).map(|(channel, _)| channel)
                .find(|channel| store.project_channel_metadata(&channel.id).ok().flatten().is_some_and(|metadata| metadata.kind == discard_core::model::ProjectChannelKind::General))
        };
        let general = general.ok_or("This project has no #general channel.")?;
        self.select_inner(view, &general.id.0).map(|_| ())
    }

    /// Check that a branch can get a worktree, and find its channel. A branch with no channel gets one at creation.
    fn worktree_target(&self, view: &ViewId, project: &ProjectId, repository: &str, branch: &str) -> Result<(discard_core::worktree::DiscoveredRepository, Option<Channel>), String> {
        let repo = self.repository(view, project, repository)?;
        let entries = self.branch_entries(&repo)?;
        let entry = entries.iter().find(|entry| entry.branch == branch && !entry.detached).ok_or("This branch no longer exists.")?;
        if entry.worktree.is_some() { return Err("This branch is checked out in a working tree.".into()); }
        let channel = self.branch_channel(project, &repo.id, branch)?;
        if let Some(channel) = &channel && self.store.lock().unwrap_or_else(|e| e.into_inner()).worktree_for(&channel.id).map_err(|e| e.to_string())?.is_some() {
            return Err("This channel already has a worktree.".into());
        }
        Ok((repo, channel))
    }
    /// Where Create Worktree… puts the worktree. The path follows the rule of owned worktrees.
    pub fn worktree_plan(&self, view: &ViewId, workspace: &discard_core::workspace::GitWorkspace, project: &ProjectId, repository: &str, branch: &str) -> Result<super::dto::WorktreePlanDto, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (repo, channel) = self.worktree_target(view, project, repository, branch)?;
        let id = channel.map_or_else(ChannelId::new, |channel| channel.id);
        let path = workspace.target_for_branch(&repo.path, &id, branch)?;
        Ok(super::dto::WorktreePlanDto { path: path.to_string_lossy().into_owned(), channel_id: id.0 })
    }
    /// Create a linked worktree for a branch with `create_for_branch`. A branch with no channel gets a channel in the
    /// same transaction as the worktree record, under `new_channel` when that ID is free. A Git failure saves nothing.
    /// Returns the worktree path.
    pub fn create_worktree(&self, view: &ViewId, workspace: &discard_core::workspace::GitWorkspace, project: &ProjectId, repository: &str, branch: &str, new_channel: Option<&str>) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let (repo, channel) = self.worktree_target(view, project, repository, branch)?;
        let record = match channel {
            Some(channel) => workspace.create_for_branch(&repo.path, &channel.id, branch)?,
            None => {
                let (agent, name) = {
                    let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                    let agent = store.project_main_agent(project).map_err(|e| e.to_string())?.ok_or("This project has no main agent. Open the project first.")?;
                    let channels = store.channels_for_project(project).map_err(|e| e.to_string())?;
                    let base = if repo.id.is_empty() { branch.to_owned() } else { format!("{}/{}", repo.label, branch) };
                    let (mut name, mut suffix) = (base.clone(), 2);
                    while channels.iter().any(|existing| existing.name.eq_ignore_ascii_case(&name)) { name = format!("{base}-{suffix}"); suffix += 1; }
                    (agent, name)
                };
                // The webview shows the path of this ID. Use it when it is well formed and free.
                let id = new_channel.filter(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                    .map(|id| ChannelId(id.to_owned()))
                    .filter(|id| self.store.lock().unwrap_or_else(|e| e.into_inner()).load_channel(id).is_err())
                    .unwrap_or_default();
                let channel = Channel { id, project_id: Some(project.clone()), name, agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
                workspace.create_channel_with_worktree(&repo.path, &repo.id, &channel, branch)?
            }
        };
        self.after_git(view);
        Ok(record.path)
    }

    /// Rename a local branch and move the channel binding to the new name. A checked-out branch works. The channel
    /// must be idle. When Git succeeds and the store fails, the branch gets its old name back. Returns the new name.
    pub fn rename_branch(&self, view: &ViewId, project: &ProjectId, repository: &str, branch: &str, new_name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        if !self.branch_entries(&repo)?.iter().any(|entry| entry.branch == branch && !entry.detached) { return Err("This branch no longer exists.".into()); }
        let channel = self.branch_channel(project, &repo.id, branch)?;
        let held = self.hold(&channel.iter().map(|channel| channel.id.clone()).collect::<Vec<_>>())?;
        let result = (|| {
            let new = discard_core::branch_actions::validate_branch_name(&repo.path, new_name, Some(branch))?;
            discard_core::branch_actions::rename_branch(&repo.path, branch, &new)?;
            if let Some(channel) = &channel {
                let saved = self.store.lock().unwrap_or_else(|e| e.into_inner()).rename_project_branch(project, &channel.id, &new).map_err(|e| e.to_string());
                if let Err(error) = saved {
                    return Err(match discard_core::branch_actions::rename_branch(&repo.path, &new, branch) {
                        Ok(()) => format!("Could not save the new branch name: {error}. The branch has its old name again."),
                        Err(undo) => format!("Could not save the new branch name: {error}. Could not restore the old name `{branch}`: {undo}"),
                    });
                }
            }
            Ok(new)
        })();
        self.release(held);
        if result.is_ok() { self.after_git(view); }
        result
    }

    /// The unmerged commits that Delete Branch… would drop, or the reason the branch cannot be deleted. The dialog
    /// reads this when it opens.
    pub fn branch_deletion_info(&self, view: &ViewId, project: &ProjectId, repository: &str, branch: &str) -> Result<usize, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        if let Some(channel) = self.branch_channel(project, &repo.id, branch)? && let Some(reason) = self.core.channel_busy_reason(&channel.id.0) { return Err(reason.into()); }
        discard_core::branch_actions::deletion_plan(&repo.path, branch)
    }
    /// Delete a local branch. The count of unmerged commits is read again here. Unmerged commits need `force`.
    pub fn delete_branch(&self, view: &ViewId, project: &ProjectId, repository: &str, branch: &str, force: bool) -> Result<super::dto::BranchDeletionDto, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        let channel = self.branch_channel(project, &repo.id, branch)?;
        let held = self.hold(&channel.iter().map(|channel| channel.id.clone()).collect::<Vec<_>>())?;
        let result = discard_core::branch_actions::delete_branch(&repo.path, branch, force);
        self.release(held);
        let deletion = match result? {
            discard_core::branch_actions::BranchDeletion::Deleted => super::dto::BranchDeletionDto { deleted: true, unmerged: 0 },
            discard_core::branch_actions::BranchDeletion::NeedsForce { unmerged } => super::dto::BranchDeletionDto { deleted: false, unmerged },
        };
        if deletion.deleted { self.after_git(view); }
        Ok(deletion)
    }

    /// Start a new branch at the HEAD of a working tree and switch it to the branch. Uncommitted changes stay. No
    /// channel that uses the tree may be busy. Returns the new name.
    pub fn create_branch(&self, view: &ViewId, project: &ProjectId, repository: &str, worktree: &str, name: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        let (entry, root, _) = self.working_tree(&repo, worktree)?;
        let (_, users) = self.worktree_users(project, &repo.id, &entry, &root)?;
        let held = self.hold(&users)?;
        let result = (|| {
            let name = discard_core::branch_actions::validate_branch_name(&repo.path, name, None)?;
            discard_core::branch_actions::create_branch(&root, &name)?;
            Ok(name)
        })();
        self.release(held);
        if result.is_ok() { self.after_git(view); }
        result
    }

    /// The files that Remove Worktree… would discard, or the reason the working tree cannot be removed.
    pub fn worktree_removal_info(&self, view: &ViewId, project: &ProjectId, repository: &str, worktree: &str) -> Result<super::dto::WorktreeFilesDto, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        let (entry, root, main) = self.working_tree(&repo, worktree)?;
        if entry.is_main_checkout { return Err("The main checkout cannot be removed.".into()); }
        let (_, users) = self.worktree_users(project, &repo.id, &entry, &root)?;
        if let Some(reason) = users.iter().find_map(|id| self.core.channel_busy_reason(&id.0)) { return Err(reason.into()); }
        let manager = WorktreeManager::new(&self.project_dir(view));
        let files = manager.status(&discard_core::worktree::Worktree { root, branch: entry.branch, base_commit: String::new(), repo: main }).map_err(|e| e.to_string())?.files();
        Ok(super::dto::WorktreeFilesDto::of(files))
    }
    /// Remove a linked working tree with `git worktree remove`. The branch and its commits stay. The status is read
    /// again here. Files that would be lost need `force`. The saved worktree records of the channels that were bound to
    /// the path go. The channels stay.
    pub fn remove_worktree(&self, view: &ViewId, project: &ProjectId, repository: &str, worktree: &str, force: bool) -> Result<super::dto::WorktreeRemovalDto, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        let repo = self.repository(view, project, repository)?;
        let (entry, root, main) = self.working_tree(&repo, worktree)?;
        if entry.is_main_checkout { return Err("The main checkout cannot be removed.".into()); }
        let (bound, users) = self.worktree_users(project, &repo.id, &entry, &root)?;
        let held = self.hold(&users)?;
        let manager = WorktreeManager::new(&self.project_dir(view));
        let outcome = manager.remove_checked(&discard_core::worktree::Worktree { root, branch: entry.branch, base_commit: String::new(), repo: main }, force).map_err(|e| e.to_string());
        let result = match outcome {
            Ok(discard_core::worktree::RemoveOutcome::Removed) => self.store.lock().unwrap_or_else(|e| e.into_inner()).delete_worktrees(&bound).map_err(|e| e.to_string())
                .map(|()| super::dto::WorktreeRemovalDto { removed: true, files: vec![], more: 0 }),
            Ok(discard_core::worktree::RemoveOutcome::NeedsForce(files)) => { let listed = super::dto::WorktreeFilesDto::of(files); Ok(super::dto::WorktreeRemovalDto { removed: false, files: listed.files, more: listed.more }) }
            Err(error) => Err(error),
        };
        self.release(held);
        if result.as_ref().is_ok_and(|removal| removal.removed) { self.after_git(view); }
        result
    }
    pub fn open_branch(&self, view: &ViewId, key: &str) -> Result<String, String> {
        let _operation = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        self.core.can_switch(view)?;
        let project_id = self.core.current_project(view).id;
        if project_id.0.is_empty() { return Err("Select a project first.".into()); }
        let repository_id = if key.starts_with('[') {
            let parts: Vec<String> = serde_json::from_str(key).map_err(|_| "Invalid repository row.")?;
            if !(2..=3).contains(&parts.len()) { return Err("Invalid repository row.".into()); }
            parts[0].clone()
        } else { String::new() };
        let repository = discard_core::worktree::discover_repositories(&self.project_dir(view)).map_err(|e| e.to_string())?
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
            self.select_inner(view, &channel.id.0)?;
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
        self.create_inner(view, name, &row, &repository.id, &repository.path)
    }
}

/// The view that `open_project_view` returns. `opened` is the result of `open_view`: false when the view existed. That
/// view can close before the check, so a view that was not added and is not there is an error, not a view to use.
fn view_after_open(view: ViewId, opened: bool, exists: bool) -> Result<ViewId, String> {
    if opened || exists { Ok(view) } else { Err(super::core::WINDOW_CLOSED.into()) }
}
/// The engine of a running sub-agent thread ended without a final state: the task fails.
fn sub_agent_stream_closed(core: &AppCore, store: &Arc<Mutex<Store>>, thread: &str) {
    use discard_core::model::ThreadState;
    let error = "The sub-agent session ended before it finished.";
    let _ = store.lock().unwrap_or_else(|e| e.into_inner()).finish_sub_agent_thread(&ChannelId(thread.to_owned()), ThreadState::Failed);
    core.apply_channel(thread, Update::Error(error.into()));
    core.apply_channel(thread, Update::SubAgentFinished { state: ThreadState::Failed, text: String::new() });
    // The updates reach the registry only while the runtime is loaded. The task ends here in any case.
    core.sub_agents().end(thread, ThreadState::Failed, "");
}

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().trim_start_matches('#').trim().to_lowercase();
    if name.is_empty() || name.chars().count() > 64 { return Err("Use a channel name with 1–64 characters.".into()); }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')) { return Err("Use letters, numbers, dashes, underscores, dots, or slashes.".into()); }
    Ok(name)
}

/// Wait until background branch refreshes finish.
#[cfg(test)]
async fn settle(core: &AppCore) {
    for _ in 0..500 {
        if !core.branch_refresh_pending(&ViewId::main()) { return; }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("a branch refresh did not finish");
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::{agent::AgentConfig, capability::ApprovalPolicy, model::{AgentId, Project}, ui_state::UiState};
    use tokio::sync::mpsc;

    fn configured_agent(store: &Store) -> AgentId {
        store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Project agent".into(), provider: "codex".into(), model: Some("model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap()
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
        core.configure_targets(&ViewId::main(), channel.project_id.as_ref().expect("test channel has a project").0.clone(), dir, channel.agent_id.0.clone());
        core.configure_read_marker(&ViewId::main(), dir, &channel.id.0);
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
                core.configure_targets(&ViewId::main(), project.id.0.clone(), &repo, agent.0.clone());
                let channels = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
                assert!(channels.decide_write_request(&ViewId::main(), &requests, &workspace, &grants, "other", &request.id, true).is_err());
                core.send_to(Some(&channel.id.0), "initial request".into()).unwrap();
                assert!(matches!(received.try_recv(), Ok(Command::Send(_))));
                assert_eq!(channels.decide_write_request(&ViewId::main(), &requests, &workspace, &grants, &channel.id.0, &request.id, true).unwrap_err(), "Wait for the current response to finish.");
                core.apply(&ViewId::main(), Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
                let result = channels.decide_write_request(&ViewId::main(), &requests, &workspace, &grants, &channel.id.0, &request.id, approve).unwrap();
                assert_eq!(result.state, if approve { "completed" } else { "denied" });
                if approve {
                    assert!(matches!(received.try_recv(), Ok(Command::Continue { agent: recipient, .. }) if recipient == agent));
                    if kind == WriteRequestKind::WorktreeCreate { assert!(store.lock().unwrap().worktree_for(&channel.id).unwrap().unwrap().owned); }
                    else { assert!(grants.take(&channel.id, &agent)); assert!(!grants.take(&channel.id, &agent)); }
                }
                assert!(received.try_recv().is_err());
                core.apply(&ViewId::main(), Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
                assert!(channels.decide_write_request(&ViewId::main(), &requests, &workspace, &grants, &channel.id.0, &request.id, true).is_err());
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
        let second_general = service.activate_project(&ViewId::main(), &second.id).unwrap().1;
        core.end_project_change();
        assert_eq!(core.current_project(&ViewId::main()).path, second.path);
        assert_eq!(core.active_channel_id(&ViewId::main()), second_general, "project activation opens its general channel");
        let direct = service.create_conversation(&ViewId::main(), &agent, None, "Direct chat").unwrap();
        let saved_second = store.lock().unwrap().last_project_channel(&second.id).unwrap().unwrap().id;
        service.select(&ViewId::main(), &direct).unwrap();
        assert_eq!(store.lock().unwrap().last_project_channel(&second.id).unwrap().unwrap().id, saved_second, "direct chat selection does not change the saved project channel");
        core.begin_project_change().unwrap();
        let restored_first = service.activate_project(&ViewId::main(), &first.id).unwrap().1;
        core.end_project_change();
        assert_eq!(restored_first, workspace.channel.id.0, "returning to A restores its saved channel");
        assert_eq!(core.active_channel_id(&ViewId::main()), workspace.channel.id.0);
        assert_eq!(core.current_project(&ViewId::main()).path, first.path);
        let store = store.lock().unwrap();
        store.hide_project(&second.id).unwrap();
        assert_eq!(store.visible_projects().unwrap().len(), 1);
        let channels = store.channels_for_project(&second.id).unwrap();
        assert_eq!(channels.len(), 1, "project activation creates only the general channel");
        assert_eq!(store.import_project_folder(second_dir.path()).unwrap().id, second.id);
        assert_eq!(store.channels_for_project(&second.id).unwrap().len(), 1);
        assert_eq!(store.visible_projects().unwrap().len(), 2);
        core.clear_project(&ViewId::main());
        assert!(core.current_project(&ViewId::main()).id.0.is_empty());
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
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        let rows: Vec<_> = core.snapshot(&ViewId::main()).branches.into_iter().filter(|row| row.detached).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, rows[1].label);
        assert_ne!(rows[0].key, rows[1].key);
        // A real branch with the same short SHA must retain its own channel.
        git(&["branch", &rows[0].label]);
        let branch_channel = service.open_branch(&ViewId::main(), &rows[0].label).unwrap();
        assert!(core.snapshot(&ViewId::main()).branches.iter().filter(|row| row.current).all(|row| !row.detached));
        let first = service.open_branch(&ViewId::main(), &rows[0].key).unwrap();
        let second = service.open_branch(&ViewId::main(), &rows[1].key).unwrap();
        assert_ne!(first, second);
        assert_ne!(first, branch_channel);
        assert_eq!(service.open_branch(&ViewId::main(), &rows[0].key).unwrap(), first);
        let selected: Vec<_> = core.snapshot(&ViewId::main()).branches.into_iter().filter(|row| row.current).collect();
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
        assert_eq!(restored.open_branch(&ViewId::main(), &rows[1].key).unwrap(), second);
        // A stale key cannot silently open the same path on a different checkout.
        let path = rows[0].worktree.as_ref().unwrap();
        let output = discard_core::process::spawn_std(std::process::Command::new("git").args(["-C", path, "checkout", "-b", "changed"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
        assert!(output.status.success());
        assert!(restored.open_branch(&ViewId::main(), &rows[0].key).is_err());
        assert!(workspace.prepare(&repo, &ChannelId(first), &rows[0].label).is_err());
        // The main checkout can also have detached HEAD, including in a nested repository.
        git(&["checkout", "--detach"]);
        let nested_project = store.lock().unwrap().import_project_folder(dir.path()).unwrap();
        service.activate_project(&ViewId::main(), &nested_project.id).unwrap();
        settle(&core).await;
        let main = core.snapshot(&ViewId::main()).branches.into_iter().find(|row| row.detached && row.kind == "main").unwrap();
        assert!(!main.repository.is_empty());
        let main_channel = service.open_branch(&ViewId::main(), &main.key).unwrap();
        assert_eq!(workspace.conversation_directory(dir.path(), &ChannelId(main_channel.clone())).unwrap(), repo.canonicalize().unwrap());
        assert_eq!(service.open_branch(&ViewId::main(), &main.key).unwrap(), main_channel);
        assert_eq!(core.snapshot(&ViewId::main()).branches.iter().filter(|row| row.current).count(), 1);
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
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        let snapshot = core.snapshot(&ViewId::main());
        assert_eq!(snapshot.repositories.len(), 2);
        let main_rows: Vec<_> = snapshot.branches.iter().filter(|row| row.label == "main").collect();
        assert_eq!(main_rows.len(), 2);
        let first = service.open_branch(&ViewId::main(), &main_rows[0].key).unwrap();
        let second = service.open_branch(&ViewId::main(), &main_rows[1].key).unwrap();
        assert_ne!(first, second);
        assert_eq!(service.open_branch(&ViewId::main(), &main_rows[0].key).unwrap(), first);
        assert_eq!(core.snapshot(&ViewId::main()).branches.iter().filter(|row| row.current).count(), 1);
        assert_eq!(core.snapshot(&ViewId::main()).branches.iter().find(|row| row.current).unwrap().key, main_rows[0].key);
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
        let feature_id = service.open_branch(&ViewId::main(), &feature.key).unwrap();
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
        assert!(service.open_branch(&ViewId::main(), &unknown).is_err());
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
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        let snapshot = core.snapshot(&ViewId::main());
        assert_eq!(snapshot.branches.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), vec!["main", "feature"]);
        assert_eq!(snapshot.channel, "general");
        assert_eq!(store.lock().unwrap().channels_for_project(&project.id).unwrap().len(), 1);

        // A conversation's branch source must not override the browsed project.
        core.configure_branch_source(&ViewId::main(), store, ChannelId("elsewhere".into()), WorkspaceMode::MainCheckout, other.path().into());
        core.refresh_branches(&ViewId::main(), core.request_branch_refresh(&ViewId::main()));
        assert_eq!(core.snapshot(&ViewId::main()).branches.len(), 2);
        assert!(core.snapshot(&ViewId::main()).branches.iter().all(|row| !row.current));
        let stale = core.request_branch_refresh(&ViewId::main());
        service.activate_project(&ViewId::main(), &other_project.id).unwrap();
        settle(&core).await;
        core.refresh_branches(&ViewId::main(), stale);
        assert!(core.snapshot(&ViewId::main()).branches.is_empty());
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        assert_eq!(core.snapshot(&ViewId::main()).branches.len(), 2);
        assert!(core.snapshot(&ViewId::main()).branches.iter().all(|row| !row.current), "the general channel does not highlight the main checkout");
        service.open_branch(&ViewId::main(), "feature").unwrap();
        assert!(core.snapshot(&ViewId::main()).branches.iter().any(|row| row.key == "feature" && row.current), "canonical branch metadata highlights its branch without a worktree");
        core.select_sidebar_project(&ViewId::main(), None);
        core.refresh_branches(&ViewId::main(), core.request_branch_refresh(&ViewId::main()));
        assert!(core.snapshot(&ViewId::main()).branches.is_empty());
    }

    fn git_in(path: &std::path::Path, args: &[&str]) {
        let output = discard_core::process::spawn_std(std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args).current_dir(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[tokio::test]
    async fn selecting_an_existing_channel_marks_its_row_without_a_git_refresh() {
        let repo = tempfile::tempdir().unwrap();
        for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-m", "initial"], vec!["branch", "feature"]] { git_in(repo.path(), &args); }
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(repo.path()).unwrap();
        configured_agent(&store);
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit {
            state: UiState::new(ChannelId(String::new()), vec![]), commands,
            project: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
        }));
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store, Handle::current(), launcher());
        let general = service.activate_project(&ViewId::main(), &project.id).unwrap().1;
        settle(&core).await;
        let feature = service.open_branch(&ViewId::main(), "feature").unwrap();
        settle(&core).await;
        assert_eq!(core.snapshot(&ViewId::main()).branches.len(), 2);
        // Git is unavailable now. A refresh would replace the rows with an empty list.
        std::fs::rename(repo.path().join(".git"), repo.path().join(".git-hidden")).unwrap();
        service.select(&ViewId::main(), &general).unwrap();
        let snapshot = core.snapshot(&ViewId::main());
        assert_eq!(snapshot.channel_id, general);
        assert_eq!(snapshot.branches.len(), 2, "selecting a channel keeps the rows");
        assert!(snapshot.branches.iter().all(|row| !row.current));
        service.select(&ViewId::main(), &feature).unwrap();
        settle(&core).await;
        let snapshot = core.snapshot(&ViewId::main());
        assert_eq!(snapshot.channel_id, feature);
        assert_eq!(snapshot.branches.len(), 2, "selecting a channel does not reload the rows");
        assert!(snapshot.branches.iter().any(|row| row.key == "feature" && row.current), "the selected row updates at once");
        assert!(!snapshot.repositories_loading);
    }

    #[tokio::test]
    async fn selecting_a_branch_without_a_channel_refreshes_in_the_background() {
        let repo = tempfile::tempdir().unwrap();
        for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-m", "initial"]] { git_in(repo.path(), &args); }
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(repo.path()).unwrap();
        configured_agent(&store);
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit {
            state: UiState::new(ChannelId(String::new()), vec![]), commands,
            project: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
        }));
        let store = Arc::new(Mutex::new(store));
        let service = Channels::new(core.clone(), store, Handle::current(), launcher());
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        assert_eq!(core.snapshot(&ViewId::main()).branches.len(), 1);
        git_in(repo.path(), &["branch", "later"]);
        service.open_branch(&ViewId::main(), "later").unwrap();
        settle(&core).await;
        let snapshot = core.snapshot(&ViewId::main());
        assert!(snapshot.branches.iter().any(|row| row.key == "later" && row.current), "creating a channel refreshes the rows");
    }

    #[tokio::test]
    async fn creation_switching_and_restore_use_saved_channels_and_distinct_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels.db");
        let store = Store::open(&path).unwrap();
        let project = Project { id: ProjectId::new(), name: "Project".into(), path: dir.path().to_string_lossy().into_owned() };
        store.insert_project(&project).unwrap();
        let agent = AgentId::new();
        store.insert_agent(&agent, &AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: Some("agent-model".into()), avatar: None, system_prompt: "Review changes".into(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "general".into(), agent_id: agent.clone(), workspace_mode: WorkspaceMode::MainCheckout };
        store.create_named_channel(&first, None, None).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        let created = service.activate_project(&ViewId::main(), &project.id).unwrap().1;
        assert_eq!(created, first.id.0);
        assert_eq!(core.snapshot(&ViewId::main()).channel, "general");
        assert!(service.create(&ViewId::main(), "Tests", "main_checkout", &created).unwrap_err().contains("Open a Git branch"));
        assert!(service.create(&ViewId::main(), "old-ui", "main_checkout", &first.id.0).unwrap_err().contains("Open a Git branch"));
        service.select(&ViewId::main(), &first.id.0).unwrap();
        assert_eq!(core.snapshot(&ViewId::main()).channel, "general");
        let reopened = Store::open(&path).unwrap();
        let rows = reopened.channels_for_project(&project.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(reopened.load_agent(&agent).unwrap().model.as_deref(), Some("agent-model"));
        let restored = tests::core(&first, dir.path());
        let service = Channels::new(restored.clone(), Arc::new(Mutex::new(reopened)), Handle::current(), launcher());
        service.restore(&ViewId::main()).unwrap();
        service.select(&ViewId::main(), &created).unwrap();
        assert_eq!(restored.snapshot(&ViewId::main()).channel, "general");
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
        store.insert_agent(&agent, &AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Builder".into(), provider: "codex".into(), model: None, avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let first = Channel { id: ChannelId::new(), project_id: Some(project.id.clone()), name: "old".into(), agent_id: agent, workspace_mode: WorkspaceMode::Worktree };
        let manager = WorktreeManager::new(dir.path());
        let prepared = manager.recorded(dir.path(), dir.path(), "old").unwrap();
        store.create_named_channel(&first, Some("old"), Some(&WorktreeRecord { channel_id: first.id.clone(), path: dir.path().to_string_lossy().into_owned(), branch: "old".into(), base_commit: prepared.worktree.base_commit, repo: project.path.clone(), owned: false })).unwrap();
        store.set_project_main_agent(&project.id, &first.agent_id).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&first, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        let direct_agent = store.lock().unwrap().create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Direct".into(), provider: "codex".into(), model: Some("direct-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        service.open_agent(&ViewId::main(), &direct_agent).unwrap();
        git(&["checkout", "-b", "new"]);
        let new = service.open_branch(&ViewId::main(), "new").unwrap();
        assert_ne!(new, first.id.0, "the changed checkout must not reuse the old session");
        assert_eq!(store.lock().unwrap().load_channel(&ChannelId(new.clone())).unwrap().agent_id, first.agent_id, "branch channels use the project main agent");
        assert!(store.lock().unwrap().channel_agents(&ChannelId(new.clone())).unwrap().contains(&first.agent_id));
        service.open_agent(&ViewId::main(), &direct_agent).unwrap();
        assert_eq!(service.open_branch(&ViewId::main(), "new").unwrap(), new, "a compatible binding is reused");
        let old = service.open_branch(&ViewId::main(), "old").unwrap();
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

        assert_eq!(service.activate_project(&ViewId::main(), &project_id).unwrap().1, general_id.0);
        let unrelated = store.lock().unwrap().create_agent(&AgentConfig {
            workspace_write: false, sandbox_autorun: false, delegation: false, native_resume: false,
            reasoning_effort: None,
            name: "Direct agent".into(),
            provider: "codex".into(),
            model: Some("direct-model".into()),
            avatar: None,
            system_prompt: String::new(),
            policy: ApprovalPolicy::Ask,
            plugin_write: false,
        }).unwrap();
        service.open_agent(&ViewId::main(), &unrelated).unwrap();
        assert_eq!(service.open_branch(&ViewId::main(), "feature/persisted").unwrap(), branch_id.0);
        assert_eq!(app_core.snapshot(&ViewId::main()).channel, "Feature");
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
        assert_eq!(reopened_service.activate_project(&ViewId::main(), &project_id).unwrap().1, branch_id.0);
        assert_eq!(reopened_core.active_channel_id(&ViewId::main()), branch_id.0);
        assert_eq!(reopened_core.snapshot(&ViewId::main()).channel, "Feature");
    }

    #[tokio::test]
    async fn project_activation_fails_without_a_configured_agent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let (commands, _) = mpsc::unbounded_channel();
        let core = Arc::new(AppCore::new(CoreInit { state: UiState::new(ChannelId(String::new()), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() }));
        let service = Channels::new(core, Arc::new(Mutex::new(store)), Handle::current(), launcher());
        let error = service.activate_project(&ViewId::main(), &project.id).unwrap_err();
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
        assert_eq!(service.select(&ViewId::main(), &branch.id.0).unwrap(), ViewId::main());
        assert_eq!(core.active_channel_id(&ViewId::main()), branch.id.0);
        assert_eq!(core.active_context(&ViewId::main()).0, agent);
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
    async fn text_channels_are_created_selected_and_listed_with_their_kind_and_messages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store);
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&general, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        let id = service.create_text_channel(&ViewId::main(), &project.id, "  Release Notes ").unwrap();
        assert_eq!(core.active_channel_id(&ViewId::main()), id, "the new channel opens");
        assert!(core.has_channel(&id));
        let channels = service.list_project_channels(&project.id).unwrap();
        let text = channels.iter().find(|channel| channel.id == id).unwrap();
        assert_eq!((text.kind.as_str(), text.name.as_str(), text.workspace_mode.as_str(), text.agent_id.as_str()), ("text", "release-notes", "main_checkout", agent.0.as_str()));
        assert!(!text.archived && !text.has_messages && text.branch.is_none() && text.worktree.is_none());
        assert_eq!(channels.iter().find(|channel| channel.id == general.id.0).unwrap().kind, "general");
        assert_eq!(service.create_text_channel(&ViewId::main(), &project.id, "RELEASE-notes").unwrap_err(), "A channel with this name already exists.");
        assert_eq!(service.create_text_channel(&ViewId::main(), &project.id, "General").unwrap_err(), "A channel with this name already exists.");
        assert!(service.create_text_channel(&ViewId::main(), &project.id, "bad/name").unwrap_err().starts_with("Use letters"));
        assert!(service.create_text_channel(&ViewId::main(), &ProjectId::new(), "other").unwrap_err().contains("Open this project"));
        assert_eq!(service.list_project_channels(&project.id).unwrap().len(), 2, "a refused name creates no channel");
        // A saved message makes the channel show in Archived after its branch goes. A channel with none does not.
        store.lock().unwrap().insert_message(&discard_core::model::Message { id: MessageId::new(), channel_id: ChannelId(id.clone()), role: discard_core::model::Role::User, blocks: vec![discard_core::model::Block::Markdown { text: "question".into() }] }).unwrap();
        let details = service.channel_details(&project.id, &ChannelId(id.clone())).unwrap();
        assert_eq!((details.channel.kind.as_str(), details.channel.has_messages), ("text", true));
        let channels = service.list_project_channels(&project.id).unwrap();
        assert!(channels.iter().find(|channel| channel.id == id).unwrap().has_messages);
        assert!(!channels.iter().find(|channel| channel.id == general.id.0).unwrap().has_messages);
    }

    #[tokio::test]
    async fn a_text_channel_renames_with_the_creation_checks_and_general_stays_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let agent = configured_agent(&store);
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&general, dir.path());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
        let plans = ChannelId(service.create_text_channel(&ViewId::main(), &project.id, "plans").unwrap());
        let notes = ChannelId(service.create_text_channel(&ViewId::main(), &project.id, "notes").unwrap());
        assert_eq!(service.rename_text_channel(&project.id, &plans, "  Release Notes ").unwrap(), "release-notes");
        assert_eq!(store.lock().unwrap().load_channel(&plans).unwrap().name, "release-notes");
        let listed = service.list_project_channels(&project.id).unwrap();
        assert_eq!(listed.iter().find(|channel| channel.id == plans.0).unwrap().name, "release-notes");
        assert_eq!(core.snapshot(&ViewId::main()).channels.iter().find(|channel| channel.id == plans.0).unwrap().name, "release-notes", "a loaded channel shows the new name");
        for name in ["NOTES", "General"] {
            assert_eq!(service.rename_text_channel(&project.id, &plans, name).unwrap_err(), "A channel with this name already exists.");
        }
        assert!(service.rename_text_channel(&project.id, &plans, "bad/name").unwrap_err().starts_with("Use letters"));
        assert!(service.rename_text_channel(&project.id, &general.id, "main").unwrap_err().contains("text channel"), "#general cannot be renamed");
        assert!(service.rename_text_channel(&ProjectId::new(), &notes, "x").is_err(), "the channel must belong to the project");
        assert_eq!(service.rename_text_channel(&project.id, &plans, "release-notes").unwrap(), "release-notes", "an unchanged name is not an error");
        assert_eq!(store.lock().unwrap().load_channel(&notes).unwrap().name, "notes");
    }

    #[tokio::test]
    async fn changing_the_main_agent_restarts_a_loaded_text_channel_with_the_new_agent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(dir.path()).unwrap();
        let old_agent = configured_agent(&store);
        let new_agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "New main".into(), provider: "claude".into(), model: Some("new-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        store.set_project_main_agent(&project.id, &old_agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &old_agent).unwrap();
        let store = Arc::new(Mutex::new(store));
        let core = core(&general, dir.path());
        let receivers = Arc::new(Mutex::new(Vec::new()));
        let kept: Launcher = {
            let receivers = receivers.clone();
            Arc::new(move |channel| {
                let (commands, receiver) = mpsc::unbounded_channel(); receivers.lock().unwrap().push(receiver);
                let (_, updates) = mpsc::unbounded_channel();
                Ok((CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "Project".into(), channel: channel.name.clone(), agent: "Builder".into(), provider: "codex".into() }, updates))
            })
        };
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), kept);
        let id = service.create_text_channel(&ViewId::main(), &project.id, "plans").unwrap();
        let unloaded = store.lock().unwrap().create_project_text_channel(&project.id, "unloaded").unwrap();
        service.change_project_main_agent(&project.id, &new_agent).unwrap();
        assert_eq!(store.lock().unwrap().load_channel(&ChannelId(id.clone())).unwrap().agent_id, new_agent);
        assert_eq!(store.lock().unwrap().load_channel(&unloaded.id).unwrap().agent_id, new_agent);
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, new_agent);
        assert_eq!(core.active_channel_id(&ViewId::main()), id);
        assert_eq!(core.active_context(&ViewId::main()).0, new_agent, "the open text channel runs the new agent");
        assert!(!core.has_channel(&unloaded.id.0), "a text channel that is not loaded starts when it opens");
        assert!(core.send_to(Some(&id), "new agent is ready".into()).is_ok());
        // The text channel now runs a response. The change waits, as it does for a general channel.
        assert!(service.change_project_main_agent(&project.id, &old_agent).unwrap_err().contains("Finish this channel's response"));
        assert_eq!(store.lock().unwrap().project_main_agent(&project.id).unwrap(), Some(new_agent));
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
        let new_agent = store.lock().unwrap().create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "New main".into(), provider: "claude".into(), model: Some("new-model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
        core.configure_targets(&ViewId::main(), project.id.0.clone(), dir.path(), old_agent.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher);
        assert!(core.begin_channel_change(&general.id.0).is_ok());
        assert!(!core.archive_started_for(&ViewId::main(), Some(&general.id.0)), "archive cannot start while channel identity is changing");
        core.end_channel_change(&general.id.0);
        assert!(core.archive_started_for(&ViewId::main(), Some(&general.id.0)));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("archive confirmation"), "channel identity cannot change while the archive worker is active");
        assert_eq!(store.lock().unwrap().project_main_agent(&project.id).unwrap(), Some(old_agent.clone()));
        core.archive_finished(&ViewId::main(), super::super::plan::ArchiveStep::NeedsConfirm(vec!["dirty worktree".into()]));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("archive confirmation"));
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, old_agent);
        assert!(core.archive_started_for(&ViewId::main(), Some(&general.id.0)), "accepting an archive confirmation can restart the archive worker");
        core.archive_finished(&ViewId::main(), super::super::plan::ArchiveStep::Failed("simulated archive failure".into()));
        assert!(service.change_project_main_agent(&project.id, &new_agent).unwrap_err().contains("simulated launcher failure"));
        assert_eq!(store.lock().unwrap().load_channel(&general.id).unwrap().agent_id, new_agent);
        assert!(core.snapshot(&ViewId::main()).notice.unwrap_or_default().contains("Select this channel again to retry"));
        assert!(core.send_to(Some(&general.id.0), "old agent must not accept this".into()).is_err());
        service.select(&ViewId::main(), &general.id.0).unwrap();
        assert_eq!(core.active_context(&ViewId::main()).0, new_agent);
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
            let first_replacement = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Replacement one".into(), provider: "codex".into(), model: Some("one".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
            let second_replacement = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Replacement two".into(), provider: "codex".into(), model: Some("two".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
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
        core.configure_targets(&ViewId::main(), first.id.0.clone(), first_dir.path(), old.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launch);
        service.select(&ViewId::main(), &second_general.id.0).unwrap();
        assert_eq!(core.active_channel_id(&ViewId::main()), second_general.id.0);
        assert!(core.has_channel(&first_general.id.0), "the first general remains parked");
        core.begin_agent_change(&old.0).unwrap();
        store.lock().unwrap().delete_agent(&old, discard_core::store::DeleteAgentOptions { delete_conversations: false, delete_private_memory: false }).unwrap();
        fail_second_replacement.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(service.reconcile_deleted_agent(&old.0).unwrap_err().contains("simulated replacement launch failure"));
        core.end_agent_change(&old.0);

        assert_eq!(store.lock().unwrap().load_channel(&first_general.id).unwrap().agent_id, first_replacement);
        assert_eq!(store.lock().unwrap().load_channel(&second_general.id).unwrap().agent_id, second_replacement);
        assert!(store.lock().unwrap().messages_for_channel(&first_general.id).unwrap().iter().any(|message| matches!(message.blocks.as_slice(), [discard_core::model::Block::Markdown { text }] if text == "retained general history")));
        assert_eq!(core.active_context(&ViewId::main()).0, second_replacement);
        assert!(!core.channel_session_closed(&first_general.id.0));
        assert!(core.channel_session_closed(&second_general.id.0), "failed replacement stays closed and retryable");
        assert_eq!(receivers.lock().unwrap().len(), 3, "the failed channel has no live replacement receiver");
        service.select(&ViewId::main(), &second_general.id.0).unwrap();
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
        service.select_canonical_branch(&ViewId::main(), &project.id, "feature/shared", &archived.id).unwrap();
        assert_eq!(core.active_channel_id(&ViewId::main()), archived.id.0);
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
        core.configure_targets(&ViewId::main(), project.id.0.clone(), dir.path(), agent.0.clone());
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launch);
        service.select_canonical_branch(&ViewId::main(), &project.id, "feature", &channel.id).unwrap();
        assert!(core.snapshot(&ViewId::main()).composer_disabled, "canonical selection preserves archived-agent read-only state");
        assert!(core.send_to(Some(&channel.id.0), "must not be sent".into()).is_err());
        assert!(matches!(receivers.lock().unwrap()[0].try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    struct RenameRig { repo: tempfile::TempDir, store: Arc<Mutex<Store>>, core: Arc<AppCore>, service: Channels, project: ProjectId, channels: Vec<(String, String)> }
    impl RenameRig {
        /// A repository with `main` and each named branch, and one branch channel for each named branch.
        async fn new(names: &[&str]) -> Self {
            let repo = tempfile::tempdir().unwrap();
            for args in [vec!["init", "-q", "-b", "main"], vec!["commit", "--allow-empty", "-m", "initial"]] { git_in(repo.path(), &args); }
            for name in names { git_in(repo.path(), &["branch", name]); }
            let store = Store::open_in_memory().unwrap();
            let project = store.import_project_folder(repo.path()).unwrap();
            configured_agent(&store);
            let (commands, _) = mpsc::unbounded_channel();
            let core = Arc::new(AppCore::new(CoreInit {
                state: UiState::new(ChannelId(String::new()), vec![]), commands,
                project: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
            }));
            let store = Arc::new(Mutex::new(store));
            let service = Channels::new(core.clone(), store.clone(), Handle::current(), launcher());
            service.activate_project(&ViewId::main(), &project.id).unwrap();
            settle(&core).await;
            let mut channels = Vec::new();
            for name in names {
                let key = core.snapshot(&ViewId::main()).branches.iter().find(|row| row.label == *name).unwrap().key.clone();
                channels.push(((*name).to_owned(), service.open_branch(&ViewId::main(), &key).unwrap()));
            }
            Self { repo, store, core, service, project: project.id, channels }
        }
        /// Open a branch channel for a branch that exists now.
        fn open(&mut self, name: &str) {
            self.refresh();
            let key = self.core.snapshot(&ViewId::main()).branches.iter().find(|row| row.label == name).unwrap().key.clone();
            self.channels.push((name.to_owned(), self.service.open_branch(&ViewId::main(), &key).unwrap()));
        }
        /// Run Git with a fixed committer date, so a reflog entry can be old.
        fn git_at(&self, date: &str, args: &[&str]) {
            let output = discard_core::process::spawn_std(std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
                .args(args).env("GIT_COMMITTER_DATE", date).current_dir(self.repo.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        fn id(&self, name: &str) -> String { self.channels.iter().find(|(n, _)| n == name).unwrap().1.clone() }
        fn git(&self, args: &[&str]) { git_in(self.repo.path(), args); }
        fn refresh(&self) { self.core.refresh_branches(&ViewId::main(), self.core.request_branch_refresh(&ViewId::main())); }
        fn repository(&self) -> String { discard_core::worktree::discover_repositories(self.repo.path()).unwrap().remove(0).id }
        fn bound(&self, name: &str) -> Option<String> {
            self.store.lock().unwrap().project_repository_branch_channel(&self.project, &self.repository(), name).unwrap().map(|channel| channel.id.0)
        }
        fn notes(&self, id: &str) -> Vec<String> {
            self.store.lock().unwrap().messages_for_channel(&ChannelId(id.into())).unwrap().into_iter()
                .filter(|m| m.role == discard_core::model::Role::System)
                .flat_map(|m| m.blocks.into_iter().filter_map(|b| match b { discard_core::model::Block::Markdown { text } if text.starts_with("Branch renamed") => Some(text), _ => None }))
                .collect()
        }
    }

    #[tokio::test]
    async fn outside_rename_moves_the_binding_and_posts_the_note() {
        let rig = RenameRig::new(&["feature"]).await;
        let id = rig.id("feature");
        rig.git(&["branch", "-m", "feature", "topic"]);
        rig.refresh();
        assert_eq!(rig.bound("topic"), Some(id.clone()));
        assert_eq!(rig.bound("feature"), None);
        let last = rig.store.lock().unwrap().messages_for_channel(&ChannelId(id)).unwrap().pop().unwrap();
        assert_eq!((last.role, last.blocks), (discard_core::model::Role::System, vec![discard_core::model::Block::Markdown { text: "Branch renamed from `feature` to `topic`.".into() }]));
    }

    #[tokio::test]
    async fn outside_rename_chain_moves_once() {
        let rig = RenameRig::new(&["feature"]).await;
        let id = rig.id("feature");
        rig.git(&["branch", "-m", "feature", "middle"]);
        rig.git(&["branch", "-m", "middle", "final"]);
        rig.refresh();
        rig.refresh();
        assert_eq!(rig.bound("final"), Some(id.clone()));
        assert_eq!(rig.notes(&id), vec!["Branch renamed from `feature` to `final`.".to_owned()]);
    }

    #[tokio::test]
    async fn deleted_branch_keeps_the_binding_and_posts_nothing() {
        let rig = RenameRig::new(&["feature"]).await;
        let id = rig.id("feature");
        rig.git(&["branch", "-D", "feature"]);
        rig.refresh();
        assert_eq!(rig.bound("feature"), Some(id.clone()));
        assert!(rig.notes(&id).is_empty());
    }

    #[tokio::test]
    async fn busy_channel_is_skipped_then_moved_on_a_later_refresh() {
        let rig = RenameRig::new(&["feature"]).await;
        let id = rig.id("feature");
        rig.git(&["branch", "-m", "feature", "topic"]);
        rig.core.begin_channel_action(&id).unwrap();
        rig.refresh();
        assert_eq!(rig.bound("feature"), Some(id.clone()));
        assert!(rig.notes(&id).is_empty());
        rig.core.end_channel_change(&id);
        rig.refresh();
        assert_eq!(rig.bound("topic"), Some(id.clone()));
        assert_eq!(rig.notes(&id).len(), 1);
    }

    #[tokio::test]
    async fn rename_onto_a_name_with_an_active_channel_changes_nothing() {
        let rig = RenameRig::new(&["feature", "other"]).await;
        let (feature, other) = (rig.id("feature"), rig.id("other"));
        rig.git(&["branch", "-D", "other"]);
        rig.git(&["branch", "-m", "feature", "other"]);
        rig.refresh();
        assert_eq!(rig.bound("feature"), Some(feature.clone()));
        assert_eq!(rig.bound("other"), Some(other));
        assert!(rig.notes(&feature).is_empty());
    }

    #[tokio::test]
    async fn two_stranded_bindings_reaching_one_target_do_not_move() {
        let rig = RenameRig::new(&["a", "b"]).await;
        let (a, b) = (rig.id("a"), rig.id("b"));
        // The reflog of `c` holds a to b and b to c, so both bound names reach `c`.
        rig.git(&["branch", "-m", "b", "tmp"]);
        rig.git(&["branch", "-m", "a", "b"]);
        rig.git(&["branch", "-D", "tmp"]);
        rig.git(&["branch", "-m", "b", "c"]);
        rig.refresh();
        assert_eq!(rig.bound("a"), Some(a.clone()));
        assert_eq!(rig.bound("b"), Some(b.clone()));
        assert!(rig.notes(&a).is_empty() && rig.notes(&b).is_empty());
    }

    #[tokio::test]
    async fn a_rename_older_than_the_channel_does_not_move_a_new_binding() {
        let mut rig = RenameRig::new(&[]).await;
        rig.git(&["branch", "wip"]);
        rig.git_at("2020-01-01T00:00:00 +0000", &["branch", "-m", "wip", "feature-x"]);
        rig.git(&["branch", "wip"]);
        rig.open("wip");
        let id = rig.id("wip");
        rig.git(&["branch", "-D", "wip"]);
        rig.refresh();
        assert_eq!(rig.bound("wip"), Some(id.clone()));
        assert_eq!(rig.bound("feature-x"), None);
        assert!(rig.notes(&id).is_empty());
    }

    #[tokio::test]
    async fn outside_rename_with_a_sibling_at_the_same_commit_moves_to_the_renamed_branch() {
        let rig = RenameRig::new(&["a", "c"]).await;
        let (a, c) = (rig.id("a"), rig.id("c"));
        rig.git(&["branch", "-m", "a", "b"]);
        rig.refresh();
        assert_eq!(rig.bound("b"), Some(a));
        assert_eq!(rig.bound("c"), Some(c));
    }
}

/// The menu actions of the channel list, against temporary repositories and an in-memory store.
#[cfg(test)]
mod navigation_action_tests {
    use super::*;
    use discard_core::{agent::AgentConfig, capability::ApprovalPolicy, model::{AgentId, Block, Message, Role}, ui_state::{Command, UiState}, workspace::GitWorkspace};
    use tokio::sync::mpsc;

    struct Rig {
        dir: tempfile::TempDir,
        repo: PathBuf,
        store: Arc<Mutex<Store>>,
        core: Arc<AppCore>,
        service: Channels,
        workspace: GitWorkspace,
        project: discard_core::model::Project,
        general: Channel,
        agent: AgentId,
        _receivers: Arc<Mutex<Vec<mpsc::UnboundedReceiver<Command>>>>,
        /// The channel of each receiver, in the same order.
        launched: Arc<Mutex<Vec<String>>>,
        requests: discard_core::write_requests::WriteRequests,
        grants: discard_core::workspace_grants::MainCheckoutGrants,
    }

    fn run(dir: &std::path::Path, args: &[&str]) -> String {
        let output = discard_core::process::spawn_std(std::process::Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args).current_dir(dir).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())).and_then(|child| child.wait_with_output()).unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// A project whose folder is a temporary repository with one commit on `main`. Nothing touches another path.
    async fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-q", "-m", "init"]);
        let store = Store::open_in_memory().unwrap();
        let project = store.import_project_folder(&repo).unwrap();
        let agent = store.create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Project agent".into(), provider: "codex".into(), model: Some("model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        store.set_project_main_agent(&project.id, &agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &agent).unwrap();
        let store = Arc::new(Mutex::new(store));
        let receivers = Arc::new(Mutex::new(Vec::new()));
        let launched = Arc::new(Mutex::new(Vec::new()));
        let launch: Launcher = {
            let receivers = receivers.clone();
            let launched = launched.clone();
            Arc::new(move |channel| {
                let (commands, receiver) = mpsc::unbounded_channel();
                receivers.lock().unwrap().push(receiver);
                launched.lock().unwrap().push(channel.id.0.clone());
                let (_, updates) = mpsc::unbounded_channel();
                Ok((CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "Project".into(), channel: channel.name.clone(), agent: "Builder".into(), provider: "codex".into() }, updates))
            })
        };
        let (init, _) = launch(&general).unwrap();
        let core = Arc::new(AppCore::new(init));
        core.configure_targets(&ViewId::main(), project.id.0.clone(), &repo, agent.0.clone());
        core.configure_read_marker(&ViewId::main(), dir.path(), &general.id.0);
        let service = Channels::new(core.clone(), store.clone(), Handle::current(), launch);
        service.activate_project(&ViewId::main(), &project.id).unwrap();
        settle(&core).await;
        let workspace = GitWorkspace::new(&dir.path().join("worktrees"), store.clone());
        Rig { dir, repo, store, core, service, workspace, project, general, agent, _receivers: receivers, launched, requests: Default::default(), grants: Default::default() }
    }
    impl Rig {
        fn git(&self, args: &[&str]) -> String { run(&self.repo, args) }
        fn branches(&self) -> Vec<String> { self.git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"]).lines().map(str::to_owned).collect() }
        fn busy(&self, channel: &str) {
            self.core.send_to(Some(channel), "work".into()).unwrap();
            assert!(self.core.channel_busy_reason(channel).is_some());
        }
        fn idle(&self, channel: &str) { self.core.apply_channel(channel, Update::TurnFinished(discard_core::driver::TurnOutcome::Completed)); }
        fn branch_channel(&self, branch: &str) -> Option<Channel> { self.store.lock().unwrap().project_branch_channel(&self.project.id, branch).unwrap() }
        fn worktree_record(&self, channel: &ChannelId) -> Option<WorktreeRecord> { self.store.lock().unwrap().worktree_for(channel).unwrap() }
        fn create_worktree(&self, branch: &str) -> String { self.service.create_worktree(&ViewId::main(), &self.workspace, &self.project.id, "", branch, None).unwrap() }
        /// The commands that the engine of a channel received since the last call.
        fn received(&self, channel: &ChannelId) -> Vec<Command> {
            let Some(index) = self.launched.lock().unwrap().iter().rposition(|id| *id == channel.0) else { return Vec::new() };
            let mut receivers = self._receivers.lock().unwrap();
            std::iter::from_fn(|| receivers[index].try_recv().ok()).collect()
        }
        fn messages(&self, channel: &ChannelId) -> Vec<Message> { self.store.lock().unwrap().messages_for_channel(channel).unwrap() }
        /// A user message in #general.
        fn say(&self, blocks: Vec<Block>) -> MessageId {
            let message = Message { id: MessageId::new(), channel_id: self.general.id.clone(), role: Role::User, blocks };
            self.store.lock().unwrap().insert_message(&message).unwrap();
            message.id
        }
        /// A text channel. #general stays the open channel.
        fn text_channel(&self, name: &str) -> ChannelId {
            let id = ChannelId(self.service.create_text_channel(&ViewId::main(), &self.project.id, name).unwrap());
            self.service.select(&ViewId::main(), &self.general.id.0).unwrap();
            id
        }
        fn handoff() -> discard_core::write_requests::Handoff {
            discard_core::write_requests::Handoff { goal: "Ship the importer".into(), constraints: vec!["Keep the CLI".into()], decisions: vec![], open_questions: vec!["Which delimiter?".into()] }
        }
        /// A pending move request of the project main agent in #general.
        fn move_request(&self, target: discard_core::write_requests::MoveTarget, source_message_id: &MessageId) -> discard_core::write_requests::WriteRequest {
            let details = discard_core::write_requests::MoveDetails { target, handoff: Self::handoff(), source_message_id: source_message_id.0.clone(), attachments: vec![] };
            self.requests.request_move(&self.general.id, &self.agent, details).unwrap()
        }
        fn to_channel(&self, id: &ChannelId, name: &str) -> discard_core::write_requests::MoveTarget { discard_core::write_requests::MoveTarget::Channel { channel_id: id.0.clone(), name: name.into() } }
        fn to_branch(&self, branch: &str, base_commit: &str) -> discard_core::write_requests::MoveTarget {
            discard_core::write_requests::MoveTarget::NewBranch { repository: String::new(), repo_path: self.repo.to_string_lossy().into_owned(), branch: branch.into(), base: "main".into(), base_commit: base_commit.into() }
        }
        fn decide(&self, id: &str, approve: bool) -> Result<discard_core::write_requests::WriteRequest, String> {
            self.service.decide_write_request(&ViewId::main(), &self.requests, &self.workspace, &self.grants, &self.general.id.0, id, approve)
        }
        fn channel_count(&self) -> usize { self.store.lock().unwrap().channels_for_project(&self.project.id).unwrap().len() }
        /// A move starts the target with a continue note. It does not copy the user's message.
        fn assert_continued(&self, target: &ChannelId) {
            let received = self.received(target);
            assert!(matches!(&received[..], [Command::Continue { note, .. }] if note.contains("moved here from #general")), "{received:?}");
        }
        /// The two messages that a move posts in the target, before the user message.
        fn assert_handoff_posted(&self, target: &ChannelId) {
            let messages = self.messages(target);
            assert_eq!(messages.len(), 2, "{messages:?}");
            assert_eq!((messages[0].role, &messages[0].blocks[..]), (Role::System, &[Block::ChannelLink { direction: discard_core::model::LinkDirection::MovedFrom, channel_id: self.general.id.0.clone(), channel_name: "general".into() }][..]));
            assert_eq!(messages[1].role, Role::Agent);
            assert_eq!(messages[1].blocks, vec![Block::Markdown { text: format!("Handoff from #general\n\n{}", Self::handoff().markdown()) }]);
            let author = self.store.lock().unwrap().message_author(&messages[1].id).unwrap().expect("the handoff has an author");
            assert_eq!((author.agent_id, author.name), (self.agent.clone(), "Project agent".to_string()));
        }
        /// An approval that fails after the checks: the request fails, the source gets the note, and the targets get nothing.
        fn assert_failed(&self, id: &str, error: &str, targets: &[&ChannelId]) {
            let done = self.decide(id, true).unwrap();
            let text = format!("Conversation move failed: {error}");
            assert_eq!((done.state.as_str(), done.result.as_deref()), ("failed", Some(text.as_str())));
            let last = self.messages(&self.general.id).pop().unwrap();
            assert_eq!((last.role, last.blocks), (Role::System, vec![Block::Markdown { text }]));
            for target in targets {
                assert!(self.messages(target).is_empty(), "a failed move posts nothing in the target");
                assert!(self.received(target).is_empty(), "a failed move sends nothing to the target");
            }
            assert!(self.received(&self.general.id).is_empty());
        }
        fn assert_moved_to(&self, target: &ChannelId, name: &str) {
            let last = self.messages(&self.general.id).pop().unwrap();
            assert_eq!((last.role, last.blocks), (Role::System, vec![Block::ChannelLink { direction: discard_core::model::LinkDirection::MovedTo, channel_id: target.0.clone(), channel_name: name.into() }]));
        }
    }

    #[tokio::test]
    async fn mute_follows_the_store_the_listing_and_the_unread_marker() {
        let r = rig().await;
        let plans = ChannelId(r.service.create_text_channel(&ViewId::main(), &r.project.id, "plans").unwrap());
        let muted = |r: &Rig, id: &ChannelId| r.service.list_project_channels(&r.project.id).unwrap().into_iter().find(|c| c.id == id.0).unwrap().muted;
        assert!(!muted(&r, &plans));
        r.service.set_channel_muted(&r.project.id, &plans, true).unwrap();
        assert!(muted(&r, &plans) && r.store.lock().unwrap().channel_muted(&plans).unwrap());
        assert!(r.core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == plans.0).unwrap().muted);
        r.service.set_channel_muted(&r.project.id, &plans, false).unwrap();
        assert!(!muted(&r, &plans));
        // #general can be muted.
        r.service.set_channel_muted(&r.project.id, &r.general.id, true).unwrap();
        assert!(muted(&r, &r.general.id));
        // A thread cannot, and the channel must belong to the project.
        let anchor = { let store = r.store.lock().unwrap(); let message = Message { id: MessageId::new(), channel_id: plans.clone(), role: Role::Agent, blocks: vec![Block::Markdown { text: "answer".into() }] }; store.insert_message(&message).unwrap(); message.id };
        let thread = r.store.lock().unwrap().create_thread(&plans, &anchor, "Why", &r.agent).unwrap();
        assert!(r.service.set_channel_muted(&r.project.id, &thread.id, true).unwrap_err().contains("thread"));
        assert!(r.service.set_channel_muted(&discard_core::model::ProjectId::new(), &plans, true).is_err());
    }

    #[tokio::test]
    async fn a_muted_channel_loads_muted_after_the_service_restarts_it() {
        let r = rig().await;
        let plans = ChannelId(r.service.create_text_channel(&ViewId::main(), &r.project.id, "plans").unwrap());
        r.service.set_channel_muted(&r.project.id, &plans, true).unwrap();
        r.service.select(&ViewId::main(), &r.general.id.0).unwrap();
        // Forget the loaded state and load the channel again from storage.
        r.core.set_channel_muted(&plans.0, false);
        r.service.select(&ViewId::main(), &plans.0).unwrap();
        assert!(r.core.snapshot(&ViewId::main()).channels.iter().find(|c| c.id == plans.0).unwrap().muted, "selection copies the saved flag");
    }

    #[tokio::test]
    async fn archive_from_a_row_refuses_general_archived_and_busy_channels() {
        let r = rig().await;
        let notes = ChannelId(r.service.create_text_channel(&ViewId::main(), &r.project.id, "notes").unwrap());
        assert_eq!(r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &r.general.id, false).unwrap_err(), "#general cannot be archived.");
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &discard_core::model::ProjectId::new(), &notes, false).is_err(), "the channel must belong to the project");
        r.busy(&notes.0);
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &notes, false).unwrap_err().contains("A turn runs"));
        assert!(!r.store.lock().unwrap().channel_archived(&notes).unwrap(), "a refusal changes nothing");
        r.idle(&notes.0);
        // The open channel archives, and #general opens.
        assert_eq!(r.core.active_channel_id(&ViewId::main()), notes.0);
        let outcome = r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &notes, false).unwrap();
        assert_eq!(outcome, super::super::dto::ArchiveChannelDto { archived: true, items: vec![] });
        assert!(r.store.lock().unwrap().channel_archived(&notes).unwrap());
        assert_eq!(r.core.active_channel_id(&ViewId::main()), r.general.id.0, "the app opens #general of the same project");
        assert_eq!(r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &notes, false).unwrap_err(), "This channel is already archived.");
    }

    #[tokio::test]
    async fn archiving_a_channel_that_is_not_open_leaves_the_open_channel_alone() {
        let r = rig().await;
        let notes = ChannelId(r.service.create_text_channel(&ViewId::main(), &r.project.id, "notes").unwrap());
        r.service.select(&ViewId::main(), &r.general.id.0).unwrap();
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &notes, false).unwrap().archived);
        assert_eq!(r.core.active_channel_id(&ViewId::main()), r.general.id.0);
        assert!(r.service.select(&ViewId::main(), &notes.0).is_ok(), "an archived channel stays readable");
    }

    #[tokio::test]
    async fn a_dirty_owned_worktree_needs_the_second_archive_confirm() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let channel = r.branch_channel("feature").expect("the worktree gave the branch a channel");
        std::fs::write(std::path::Path::new(&path).join("wip.txt"), "wip").unwrap();
        let first = r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &channel.id, false).unwrap();
        assert!(!first.archived && first.items.iter().any(|item| item.contains("wip.txt")), "{first:?}");
        assert!(std::path::Path::new(&path).exists() && !r.store.lock().unwrap().channel_archived(&channel.id).unwrap());
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &r.project.id, &channel.id, true).unwrap().archived);
        assert!(!std::path::Path::new(&path).exists());
        assert!(r.branches().contains(&"feature".to_string()), "the branch is kept");
        assert!(r.worktree_record(&channel.id).is_none());
    }

    #[tokio::test]
    async fn create_worktree_plans_a_path_then_creates_the_channel_and_record_together() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let plan = r.service.worktree_plan(&ViewId::main(), &r.workspace, &r.project.id, "", "feature").unwrap();
        assert!(plan.path.starts_with(r.dir.path().join("worktrees").to_str().unwrap()) && plan.path.contains("feature"), "{}", plan.path);
        let path = r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "feature", Some(&plan.channel_id)).unwrap();
        assert_eq!(path, plan.path, "the path of the dialog is the path of the worktree");
        let channel = r.branch_channel("feature").unwrap();
        assert_eq!(channel.id.0, plan.channel_id);
        let record = r.worktree_record(&channel.id).unwrap();
        assert!(record.owned && record.path == path && record.branch == "feature");
        assert!(std::path::Path::new(&path).join("a.txt").exists());
        let created = std::path::Path::new(&path).canonicalize().unwrap();
        settle(&r.core).await;
        assert!(r.core.snapshot(&ViewId::main()).branches.iter().any(|row| row.worktree.as_deref().is_some_and(|w| std::path::Path::new(w).canonicalize().ok().as_ref() == Some(&created))), "a branch refresh shows the new worktree");
        // The branch is checked out now, so a second request is refused.
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "feature", None).unwrap_err().contains("checked out"));
    }

    #[tokio::test]
    async fn create_worktree_is_refused_for_a_checked_out_branch_a_bound_channel_and_a_missing_branch() {
        let r = rig().await;
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "main", None).unwrap_err().contains("checked out"));
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "nope", None).unwrap_err().contains("no longer exists"));
        r.git(&["branch", "bound"]);
        let channel = Channel { id: ChannelId::new(), project_id: Some(r.project.id.clone()), name: "bound".into(), agent_id: r.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let record = WorktreeRecord { channel_id: channel.id.clone(), path: "/elsewhere".into(), branch: "bound".into(), base_commit: "abc".into(), repo: r.repo.to_string_lossy().into(), owned: false };
        r.store.lock().unwrap().create_named_repository_channel(&channel, "", Some("bound"), Some(&record)).unwrap();
        assert_eq!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "bound", None).unwrap_err(), "This channel already has a worktree.");
        assert_eq!(r.worktree_record(&channel.id).unwrap().path, "/elsewhere", "a refusal saves no record");
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &discard_core::model::ProjectId::new(), "", "bound", None).is_err());
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "missing-repo", "bound", None).unwrap_err().contains("repository"));
    }

    #[tokio::test]
    async fn a_git_failure_in_create_worktree_saves_no_channel_and_no_record() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let plan = r.service.worktree_plan(&ViewId::main(), &r.workspace, &r.project.id, "", "feature").unwrap();
        // The target folder exists, so Git cannot add the worktree.
        std::fs::create_dir_all(&plan.path).unwrap();
        std::fs::write(std::path::Path::new(&plan.path).join("keep"), "keep").unwrap();
        assert!(r.service.create_worktree(&ViewId::main(), &r.workspace, &r.project.id, "", "feature", Some(&plan.channel_id)).is_err());
        assert!(r.branch_channel("feature").is_none());
        assert!(r.store.lock().unwrap().load_channel(&ChannelId(plan.channel_id)).is_err());
        assert_eq!(std::fs::read_to_string(std::path::Path::new(&plan.path).join("keep")).unwrap(), "keep");
    }

    #[tokio::test]
    async fn rename_branch_checks_the_name_moves_the_binding_and_keeps_the_history() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let channel = ChannelId(r.service.open_branch(&ViewId::main(), "feature").unwrap());
        let message = Message { id: MessageId::new(), channel_id: channel.clone(), role: Role::User, blocks: vec![Block::Markdown { text: "keep".into() }] };
        r.store.lock().unwrap().insert_message(&message).unwrap();
        r.git(&["branch", "taken"]);
        for (name, reason) in [("", "Enter a branch name"), ("feature", "not the current name"), ("taken", "already exists"), ("bad name", "not a valid branch name")] {
            assert!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", name).unwrap_err().contains(reason), "{name}");
        }
        assert_eq!(r.branches(), ["feature", "main", "taken"], "a refusal changes nothing");
        assert_eq!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", " feature-2 ").unwrap(), "feature-2");
        assert_eq!(r.branches(), ["feature-2", "main", "taken"]);
        let moved = r.branch_channel("feature-2").unwrap();
        assert_eq!(moved.id, channel);
        assert!(r.branch_channel("feature").is_none());
        assert_eq!(r.store.lock().unwrap().messages_for_channel(&channel).unwrap().len(), 1);
        assert!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "gone", "other").unwrap_err().contains("no longer exists"));
    }

    #[tokio::test]
    async fn rename_branch_works_for_a_checked_out_branch_and_updates_the_worktree_record() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let channel = r.branch_channel("feature").unwrap();
        assert_eq!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", "feature-2").unwrap(), "feature-2");
        assert_eq!(run(std::path::Path::new(&path), &["rev-parse", "--abbrev-ref", "HEAD"]), "feature-2");
        assert_eq!(r.worktree_record(&channel.id).unwrap().branch, "feature-2");
        assert_eq!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "main", "trunk").unwrap(), "trunk");
        assert_eq!(r.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "trunk");
    }

    #[tokio::test]
    async fn rename_branch_is_refused_while_its_channel_is_busy() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let channel = ChannelId(r.service.open_branch(&ViewId::main(), "feature").unwrap());
        r.busy(&channel.0);
        assert!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", "other").unwrap_err().contains("A turn runs"));
        assert_eq!(r.branches(), ["feature", "main"]);
        r.idle(&channel.0);
        assert!(r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", "other").is_ok());
    }

    #[tokio::test]
    async fn a_store_failure_after_the_git_rename_renames_the_branch_back() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let channel = ChannelId(r.service.open_branch(&ViewId::main(), "feature").unwrap());
        // A stale channel still holds the name `taken` in the store, but Git has no such branch.
        let stale = Channel { id: ChannelId::new(), project_id: Some(r.project.id.clone()), name: "taken".into(), agent_id: r.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        r.store.lock().unwrap().create_named_repository_channel(&stale, "", Some("taken"), None).unwrap();
        let error = r.service.rename_branch(&ViewId::main(), &r.project.id, "", "feature", "taken").unwrap_err();
        assert!(error.contains("old name again"), "{error}");
        assert_eq!(r.branches(), ["feature", "main"], "Git has the old name again");
        assert_eq!(r.store.lock().unwrap().channel_branch(&channel).unwrap().as_deref(), Some("feature"));
    }

    #[tokio::test]
    async fn delete_branch_counts_unmerged_commits_and_needs_the_force_choice() {
        let r = rig().await;
        r.git(&["branch", "merged"]);
        r.git(&["checkout", "-q", "-b", "wip"]);
        r.git(&["commit", "-q", "--allow-empty", "-m", "one"]);
        r.git(&["commit", "-q", "--allow-empty", "-m", "two"]);
        r.git(&["checkout", "-q", "main"]);
        assert_eq!(r.service.branch_deletion_info(&ViewId::main(), &r.project.id, "", "wip").unwrap(), 2);
        assert_eq!(r.service.branch_deletion_info(&ViewId::main(), &r.project.id, "", "merged").unwrap(), 0);
        let refused = r.service.delete_branch(&ViewId::main(), &r.project.id, "", "wip", false).unwrap();
        assert_eq!((refused.deleted, refused.unmerged), (false, 2));
        assert!(r.branches().contains(&"wip".to_string()), "the backend refuses without the force choice");
        assert!(r.service.delete_branch(&ViewId::main(), &r.project.id, "", "merged", false).unwrap().deleted);
        // A commit that lands after the dialog opened is counted again at confirm time.
        r.git(&["checkout", "-q", "-b", "late"]);
        r.git(&["checkout", "-q", "main"]);
        assert_eq!(r.service.branch_deletion_info(&ViewId::main(), &r.project.id, "", "late").unwrap(), 0);
        r.git(&["checkout", "-q", "late"]);
        r.git(&["commit", "-q", "--allow-empty", "-m", "late commit"]);
        r.git(&["checkout", "-q", "main"]);
        assert!(!r.service.delete_branch(&ViewId::main(), &r.project.id, "", "late", false).unwrap().deleted);
        let forced = r.service.delete_branch(&ViewId::main(), &r.project.id, "", "wip", true).unwrap();
        assert_eq!((forced.deleted, forced.unmerged), (true, 0));
        assert_eq!(r.branches(), ["late", "main"]);
    }

    #[tokio::test]
    async fn delete_branch_is_refused_for_the_default_a_checked_out_branch_and_a_busy_channel() {
        let r = rig().await;
        assert!(r.service.delete_branch(&ViewId::main(), &r.project.id, "", "main", true).unwrap_err().contains("default branch"));
        assert!(r.service.branch_deletion_info(&ViewId::main(), &r.project.id, "", "main").is_err());
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        assert!(r.service.delete_branch(&ViewId::main(), &r.project.id, "", "feature", true).unwrap_err().contains("checked out"));
        assert!(std::path::Path::new(&path).exists());
        r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, false).unwrap();
        let channel = r.branch_channel("feature").unwrap();
        r.service.select(&ViewId::main(), &channel.id.0).unwrap();
        r.busy(&channel.id.0);
        assert!(r.service.delete_branch(&ViewId::main(), &r.project.id, "", "feature", true).unwrap_err().contains("A turn runs"));
        assert!(r.service.branch_deletion_info(&ViewId::main(), &r.project.id, "", "feature").unwrap_err().contains("A turn runs"));
        r.idle(&channel.id.0);
        assert!(r.service.delete_branch(&ViewId::main(), &r.project.id, "", "feature", false).unwrap().deleted);
        assert!(r.store.lock().unwrap().load_channel(&channel.id).is_ok(), "the channel stays. The list moves it to Archived when it has a conversation");
    }

    #[tokio::test]
    async fn create_branch_switches_the_working_tree_and_keeps_uncommitted_changes() {
        let r = rig().await;
        std::fs::write(r.repo.join("a.txt"), "edited\n").unwrap();
        let head = r.git(&["rev-parse", "HEAD"]);
        let root = r.repo.to_string_lossy().into_owned();
        for (name, reason) in [("", "Enter a branch name"), ("main", "already exists"), ("two words", "not a valid branch name")] {
            assert!(r.service.create_branch(&ViewId::main(), &r.project.id, "", &root, name).unwrap_err().contains(reason), "{name}");
        }
        assert_eq!(r.service.create_branch(&ViewId::main(), &r.project.id, "", &root, "topic/new").unwrap(), "topic/new");
        assert_eq!(r.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "topic/new");
        assert_eq!(r.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(std::fs::read_to_string(r.repo.join("a.txt")).unwrap(), "edited\n");
        settle(&r.core).await;
        assert!(r.core.snapshot(&ViewId::main()).branches.iter().any(|row| row.label == "topic/new"), "a branch refresh lists the new branch");
        assert!(r.service.create_branch(&ViewId::main(), &r.project.id, "", "/not/a/worktree", "x").unwrap_err().contains("no longer available"));
    }

    #[tokio::test]
    async fn create_branch_is_refused_while_a_channel_that_uses_the_tree_is_busy() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let channel = r.branch_channel("feature").unwrap();
        r.service.select(&ViewId::main(), &channel.id.0).unwrap();
        r.busy(&channel.id.0);
        assert!(r.service.create_branch(&ViewId::main(), &r.project.id, "", &path, "topic").unwrap_err().contains("A turn runs"));
        assert_eq!(run(std::path::Path::new(&path), &["rev-parse", "--abbrev-ref", "HEAD"]), "feature");
        r.idle(&channel.id.0);
        assert_eq!(r.service.create_branch(&ViewId::main(), &r.project.id, "", &path, "topic").unwrap(), "topic");
    }

    #[tokio::test]
    async fn remove_worktree_keeps_the_branch_and_the_channel_and_drops_the_record() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let channel = r.branch_channel("feature").unwrap();
        assert_eq!(r.service.worktree_removal_info(&ViewId::main(), &r.project.id, "", &path).unwrap().files, Vec::<String>::new());
        let removal = r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, false).unwrap();
        assert!(removal.removed && removal.files.is_empty());
        assert!(!std::path::Path::new(&path).exists());
        assert!(r.branches().contains(&"feature".to_string()), "the branch stays");
        assert!(r.worktree_record(&channel.id).is_none(), "the record goes");
        assert!(r.store.lock().unwrap().load_channel(&channel.id).is_ok(), "the channel stays");
        assert!(r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, false).unwrap_err().contains("no longer available"));
    }

    #[tokio::test]
    async fn remove_worktree_lists_at_most_ten_files_and_needs_the_force_choice() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let root = std::path::Path::new(&path);
        for index in 0..12 { std::fs::write(root.join(format!("new-{index:02}.txt")), "x").unwrap(); }
        std::fs::write(root.join("a.txt"), "edited\n").unwrap();
        let info = r.service.worktree_removal_info(&ViewId::main(), &r.project.id, "", &path).unwrap();
        assert_eq!((info.files.len(), info.more), (10, 3));
        let refused = r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, false).unwrap();
        assert!(!refused.removed);
        assert_eq!((refused.files.len(), refused.more), (10, 3), "the refusal returns the file list");
        assert!(root.exists(), "the backend refuses without the force choice");
        assert!(r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, true).unwrap().removed);
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn remove_worktree_is_refused_for_the_main_checkout_and_a_busy_channel() {
        let r = rig().await;
        let main = r.repo.to_string_lossy().into_owned();
        assert!(r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &main, true).unwrap_err().contains("main checkout"));
        assert!(r.service.worktree_removal_info(&ViewId::main(), &r.project.id, "", &main).unwrap_err().contains("main checkout"));
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let channel = r.branch_channel("feature").unwrap();
        r.service.select(&ViewId::main(), &channel.id.0).unwrap();
        r.busy(&channel.id.0);
        assert!(r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, true).unwrap_err().contains("A turn runs"));
        assert!(r.service.worktree_removal_info(&ViewId::main(), &r.project.id, "", &path).unwrap_err().contains("A turn runs"));
        assert!(std::path::Path::new(&path).exists() && r.worktree_record(&channel.id).is_some());
    }

    #[tokio::test]
    async fn remove_worktree_drops_the_record_of_every_channel_bound_to_the_path() {
        let r = rig().await;
        r.git(&["branch", "feature"]);
        let path = r.create_worktree("feature");
        let owner = r.branch_channel("feature").unwrap();
        // Another channel borrows the same working tree, so it holds its own record.
        let borrower = Channel { id: ChannelId::new(), project_id: Some(r.project.id.clone()), name: "borrower".into(), agent_id: r.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let record = WorktreeRecord { channel_id: borrower.id.clone(), path: path.clone(), branch: "feature".into(), base_commit: "abc".into(), repo: r.repo.to_string_lossy().into(), owned: false };
        r.store.lock().unwrap().create_named_channel(&borrower, None, Some(&record)).unwrap();
        assert!(r.service.remove_worktree(&ViewId::main(), &r.project.id, "", &path, false).unwrap().removed);
        assert!(r.worktree_record(&owner.id).is_none() && r.worktree_record(&borrower.id).is_none());
        assert!(r.store.lock().unwrap().load_channel(&borrower.id).is_ok());
    }

    // Project views: a project view is fixed to its project (decisions 17 and 17b to 22 and 24 to 26).

    /// A second project with its own `#general`, in the store of the rig.
    fn second_project(r: &Rig) -> (discard_core::model::Project, Channel) {
        let folder = r.dir.path().join("other");
        std::fs::create_dir(&folder).unwrap();
        let store = r.store.lock().unwrap();
        let project = store.import_project_folder(&folder).unwrap();
        store.set_project_main_agent(&project.id, &r.agent).unwrap();
        let general = store.ensure_project_general_channel(&project.id, &r.agent).unwrap();
        (project, general)
    }
    /// How many engines the launcher started.
    fn launches(r: &Rig) -> usize { r._receivers.lock().unwrap().len() }
    /// Wait until the background branch refreshes of a view finish.
    async fn settle_view(core: &AppCore, view: &ViewId) {
        for _ in 0..500 {
            if !core.branch_refresh_pending(view) { return; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("a branch refresh did not finish");
    }

    #[tokio::test]
    async fn a_target_outside_the_project_routes_to_the_main_view() {
        let r = rig().await;
        let (q, q_general) = second_project(&r);
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        assert_eq!(r.core.route(&p_view, Some(&p)), p_view);
        assert_eq!(r.core.route(&p_view, Some(&q.id)), ViewId::main());
        assert_eq!(r.core.route(&p_view, None), ViewId::main(), "a direct message");
        // A channel of its own project stays in the project view.
        let notes = r.service.create_text_channel(&p_view, &p, "notes").unwrap();
        assert_eq!((r.core.active_channel_id(&p_view), r.core.active_channel_id(&ViewId::main())), (notes.clone(), r.general.id.0.clone()));
        assert_eq!(r.service.select(&p_view, &r.general.id.0).unwrap(), p_view);
        assert_eq!(r.core.active_channel_id(&p_view), r.general.id.0);
        // A channel of another project changes the main view only.
        assert_eq!(r.service.select(&p_view, &q_general.id.0).unwrap(), ViewId::main());
        assert_eq!(r.core.active_channel_id(&ViewId::main()), q_general.id.0);
        assert_eq!(r.core.active_channel_id(&p_view), r.general.id.0, "the project view did not move");
        // So does a direct message.
        let direct = r.store.lock().unwrap().create_conversation(&r.agent, None, "Direct").unwrap();
        assert_eq!(r.service.select(&p_view, &direct.id.0).unwrap(), ViewId::main());
        assert_eq!(r.core.active_channel_id(&ViewId::main()), direct.id.0);
        assert_eq!(r.core.active_channel_id(&p_view), r.general.id.0);
        assert_eq!(r.core.current_project(&p_view).id, p, "the project of the view does not change");
    }

    #[tokio::test]
    async fn no_selection_path_takes_a_project_view_out_of_its_project() {
        let r = rig().await;
        let (q, q_general) = second_project(&r);
        let p_view = r.service.open_project_view(&r.project.id).unwrap();
        let stays = |r: &Rig| assert_eq!((r.core.active_channel_id(&p_view), r.core.current_project(&p_view).id), (r.general.id.0.clone(), r.project.id.clone()));
        // A direct message of an agent, and a new direct message.
        assert_eq!(r.service.open_agent(&p_view, &r.agent).unwrap(), r.core.active_channel_id(&ViewId::main()));
        stays(&r);
        let created = r.service.create_conversation(&p_view, &r.agent, None, "Direct").unwrap();
        assert_eq!(r.core.active_channel_id(&ViewId::main()), created);
        stays(&r);
        // The general channel of another project.
        let general = r.service.create_conversation(&p_view, &r.agent, Some(&q.id), "general").unwrap();
        assert_eq!((general.as_str(), r.core.active_channel_id(&ViewId::main())), (q_general.id.0.as_str(), q_general.id.0.clone()));
        stays(&r);
        // A thread of another project opens beside its parent in the main view.
        let anchor = { let store = r.store.lock().unwrap(); let message = Message { id: MessageId::new(), channel_id: q_general.id.clone(), role: Role::Agent, blocks: vec![Block::Markdown { text: "answer".into() }] }; store.insert_message(&message).unwrap(); message.id };
        let thread = r.store.lock().unwrap().create_thread(&q_general.id, &anchor, "Why", &r.agent).unwrap();
        r.service.select(&ViewId::main(), &r.general.id.0).unwrap();
        assert_eq!(r.service.select_thread(&p_view, &thread.id).unwrap(), (ViewId::main(), q_general.id.0.clone()));
        assert_eq!(r.core.active_channel_id(&ViewId::main()), q_general.id.0);
        assert_eq!(r.core.open_thread_id(&ViewId::main()), Some(thread.id.0.clone()));
        assert_eq!(r.core.open_thread_id(&p_view), None);
        stays(&r);
        assert_eq!(r.service.select(&p_view, &thread.id.0).unwrap(), ViewId::main());
        stays(&r);
        // A project change that asks for another project.
        assert_eq!(r.service.activate_project(&p_view, &q.id).unwrap(), (ViewId::main(), q_general.id.0.clone()));
        stays(&r);
        assert_eq!(r.core.current_project(&ViewId::main()).id, q.id);
        // The store keeps the project of the main view.
        assert_eq!(r.store.lock().unwrap().selected_project().unwrap().unwrap().id, q.id);
        assert_eq!(r.service.activate_project(&p_view, &r.project.id).unwrap(), (p_view.clone(), r.general.id.0.clone()));
        assert_eq!(r.store.lock().unwrap().selected_project().unwrap().unwrap().id, q.id, "a project view does not select the project of the store");
        assert_eq!(r.core.current_project(&ViewId::main()).id, q.id);
    }

    #[tokio::test]
    async fn a_project_view_lists_only_its_project_in_a_service_rig() {
        let r = rig().await;
        let (q, q_general) = second_project(&r);
        let p_view = r.service.open_project_view(&r.project.id).unwrap();
        r.service.select(&ViewId::main(), &q_general.id.0).unwrap();
        let projects = |view: &ViewId| r.core.snapshot(view).channels.into_iter().map(|c| c.project_id).collect::<Vec<_>>();
        assert_eq!(projects(&p_view), vec![Some(r.project.id.0.clone())]);
        let all = projects(&ViewId::main());
        assert!(all.contains(&Some(r.project.id.0.clone())) && all.contains(&Some(q.id.0.clone())), "{all:?}");
    }

    #[tokio::test]
    async fn opening_a_project_view_selects_the_saved_channel_and_starts_its_branch_refresh() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (loaded, started) = (r.core.loaded_channel_count(), launches(&r));
        let view = r.service.open_project_view(&p).unwrap();
        assert_eq!(view, ViewId::project(&p));
        assert_eq!(r.core.view_ids(), vec![ViewId::main(), view.clone()]);
        assert_eq!(r.core.active_channel_id(&view), r.general.id.0);
        assert_eq!((r.core.loaded_channel_count(), launches(&r)), (loaded, started), "the channel is in the pool: no second session");
        settle_view(&r.core, &view).await;
        let snapshot = r.core.snapshot(&view);
        assert!(!snapshot.repositories_loading && snapshot.branches.iter().any(|row| row.label == "main"), "the new view got its branches");
        assert_eq!((snapshot.project_id.as_str(), snapshot.channel_id.as_str()), (p.0.as_str(), r.general.id.0.as_str()));
        // A view that exists comes back as it is.
        let notes = r.service.create_text_channel(&view, &p, "notes").unwrap();
        assert_eq!(r.service.open_project_view(&p).unwrap(), view);
        assert_eq!(r.core.active_channel_id(&view), notes);
        assert_eq!(r.core.view_ids().len(), 2);
        // The next view opens on the channel that the store saved, whichever view selected it.
        r.core.drop_view(&view);
        let again = r.service.open_project_view(&p).unwrap();
        assert_eq!(r.core.active_channel_id(&again), notes);
        // An unknown project opens no view.
        assert!(r.service.open_project_view(&discard_core::model::ProjectId::new()).is_err());
        assert_eq!(r.core.view_ids().len(), 2);
    }

    #[tokio::test]
    async fn two_views_that_select_one_channel_share_one_session() {
        let r = rig().await;
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        let notes = r.service.create_text_channel(&ViewId::main(), &p, "notes").unwrap();
        r.service.select(&ViewId::main(), &r.general.id.0).unwrap();
        let (loaded, started) = (r.core.loaded_channel_count(), launches(&r));
        r.service.select(&p_view, &notes).unwrap();
        r.service.select(&ViewId::main(), &notes).unwrap();
        assert_eq!((r.core.active_channel_id(&p_view), r.core.active_channel_id(&ViewId::main())), (notes.clone(), notes.clone()));
        assert_eq!(r.core.loaded_channel_count(), loaded, "the pool did not grow");
        assert_eq!(launches(&r), started, "the launcher did not run again for the channel");
        // A channel that the pool holds starts no session, whoever asks.
        r.service.start(&Channel { id: ChannelId(notes.clone()), project_id: Some(p), name: "notes".into(), agent_id: r.agent.clone(), workspace_mode: WorkspaceMode::Worktree }).unwrap();
        assert_eq!((r.core.loaded_channel_count(), launches(&r)), (loaded, started));
    }

    #[tokio::test]
    async fn an_unavailable_channel_falls_back_in_every_view() {
        let r = rig().await;
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        let notes = ChannelId(r.service.create_text_channel(&ViewId::main(), &p, "notes").unwrap());
        r.service.select(&p_view, &notes.0).unwrap();
        assert_eq!((r.core.active_channel_id(&ViewId::main()), r.core.active_channel_id(&p_view)), (notes.0.clone(), notes.0.clone()));
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &p, &notes, false).unwrap().archived);
        assert_eq!(r.core.active_channel_id(&ViewId::main()), r.general.id.0);
        assert_eq!(r.core.active_channel_id(&p_view), r.general.id.0, "the project view opens #general too");
        // A view that does not show the channel stays where it is, and one that does is moved from another view.
        let plans = ChannelId(r.service.create_text_channel(&p_view, &p, "plans").unwrap());
        assert_eq!((r.core.active_channel_id(&ViewId::main()), r.core.active_channel_id(&p_view)), (r.general.id.0.clone(), plans.0.clone()));
        assert!(r.service.archive_channel(&ViewId::main(), &r.workspace, &p, &plans, false).unwrap().archived);
        assert_eq!((r.core.active_channel_id(&ViewId::main()), r.core.active_channel_id(&p_view)), (r.general.id.0.clone(), r.general.id.0.clone()));
    }

    #[tokio::test]
    async fn a_canonical_branch_change_moves_every_view_that_selected_the_old_channel() {
        let r = rig().await;
        let p = r.project.id.clone();
        let archived = Channel { id: ChannelId::new(), project_id: Some(p.clone()), name: "Feature old".into(), agent_id: r.agent.clone(), workspace_mode: WorkspaceMode::Worktree };
        let current = Channel { id: ChannelId::new(), name: "Feature current".into(), ..archived.clone() };
        {
            let store = r.store.lock().unwrap();
            store.create_named_channel(&archived, Some("feature/shared"), None).unwrap();
            store.create_named_channel(&current, Some("feature/shared"), None).unwrap();
        }
        let p_view = r.service.open_project_view(&p).unwrap();
        r.service.select(&ViewId::main(), &current.id.0).unwrap();
        r.service.select(&p_view, &current.id.0).unwrap();
        r.service.select_canonical_branch(&ViewId::main(), &p, "feature/shared", &archived.id).unwrap();
        assert_eq!((r.core.active_channel_id(&ViewId::main()), r.core.active_channel_id(&p_view)), (archived.id.0.clone(), archived.id.0.clone()));
        assert_eq!(r.core.loaded_channel_count(), r.core.snapshot(&ViewId::main()).channels.len(), "every loaded channel is listed once");
    }

    #[tokio::test]
    async fn a_project_view_does_not_change_saved_window_state() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (q, _) = second_project(&r);
        let panels = discard_core::window_state::StateStore::open(&r.dir.path().join("state.toml")).state().panels;
        let kept = r.service.create_text_channel(&ViewId::main(), &p, "kept").unwrap();
        let saved = r.core.saved_conversation();
        assert_eq!(saved.as_deref(), Some(kept.as_str()), "the main view saves its selection");
        let p_view = r.service.open_project_view(&p).unwrap();
        let notes = r.service.create_text_channel(&p_view, &p, "notes").unwrap();
        r.core.set_panels(&p_view, 300.0, 310.0).unwrap();
        r.core.set_member_list_visible(&p_view, false).unwrap();
        r.service.select(&p_view, &r.general.id.0).unwrap();
        r.service.select(&p_view, &notes).unwrap();
        assert_eq!(r.core.saved_conversation(), saved, "the project view does not save selected_conversation");
        assert_eq!(discard_core::window_state::StateStore::open(&r.dir.path().join("state.toml")).state().panels, panels);
        assert_eq!(r.core.snapshot(&ViewId::main()).panels.channel_list, panels.channel_list);
        assert!(r.core.snapshot(&ViewId::main()).member_list_visible);
        let shown = r.core.snapshot(&p_view);
        assert_eq!((shown.panels.channel_list, shown.panels.member_list, shown.member_list_visible), (300.0, 310.0, false));
        // The selected channel of the project is saved, and the selected project of the store is not.
        assert_eq!(r.store.lock().unwrap().last_project_channel(&p).unwrap().unwrap().id.0, notes);
        r.store.lock().unwrap().select_project(&q.id).unwrap();
        r.service.select(&p_view, &r.general.id.0).unwrap();
        assert_eq!(r.store.lock().unwrap().selected_project().unwrap().unwrap().id, q.id);
    }

    #[tokio::test]
    async fn a_view_dropped_during_navigation_stays_dropped() { // Review Focus 3
        let r = rig().await;
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        let before = r.core.active_channel_id(&ViewId::main());
        r.core.begin_project_navigation(&p_view).unwrap();
        r.core.drop_view(&p_view);
        assert_eq!(r.service.activate_project(&p_view, &p).unwrap_err(), "This window is closed.");
        // A target outside the project does not fall back to the main view either.
        let (q, _) = second_project(&r);
        assert_eq!(r.service.activate_project(&p_view, &q.id).unwrap_err(), "This window is closed.");
        assert_eq!(r.service.select(&p_view, &r.general.id.0).unwrap_err(), "This window is closed.");
        r.core.end_project_navigation(&p_view);
        assert_eq!(r.core.view_ids(), vec![ViewId::main()]);
        assert_eq!(r.core.active_channel_id(&ViewId::main()), before, "the main view did not change");
        assert_eq!(r.core.current_project(&ViewId::main()).id, p);
    }

    #[tokio::test]
    async fn a_project_view_opens_on_the_saved_channel_of_the_project() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (_, q_general) = second_project(&r);
        // The saved channel of project "p" is "notes". The main view then selects a channel of project "q".
        let notes = r.service.create_text_channel(&ViewId::main(), &p, "notes").unwrap();
        r.service.select(&ViewId::main(), &q_general.id.0).unwrap();
        assert_eq!(r.store.lock().unwrap().last_project_channel(&p).unwrap().unwrap().id.0, notes);
        let main_before = (r.core.active_channel_id(&ViewId::main()), r.core.current_project(&ViewId::main()).id);
        let selected = r.store.lock().unwrap().selected_project().unwrap().map(|project| project.id);
        let view = r.service.open_project_view(&p).unwrap();
        assert_eq!(r.core.active_channel_id(&view), notes);
        assert_eq!(r.core.current_project(&view).id, p);
        assert_eq!((r.core.active_channel_id(&ViewId::main()), r.core.current_project(&ViewId::main()).id), main_before, "the main view is unchanged");
        assert_eq!(r.core.active_channel_id(&ViewId::main()), q_general.id.0);
        assert_eq!(r.store.lock().unwrap().selected_project().unwrap().map(|project| project.id), selected, "the project of the store is unchanged");
        // A second open returns the same view and changes no selection.
        r.service.select(&view, &r.general.id.0).unwrap();
        assert_eq!(r.service.open_project_view(&p).unwrap(), view);
        assert_eq!(r.core.active_channel_id(&view), r.general.id.0);
        assert_eq!(r.core.active_channel_id(&ViewId::main()), q_general.id.0);
        assert_eq!(r.core.view_ids(), vec![ViewId::main(), view]);
    }

    #[tokio::test]
    async fn the_project_name_of_every_view_follows_a_rename() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (q, _) = second_project(&r);
        let q_view = r.service.open_project_view(&q.id).unwrap();
        let p_view = r.service.open_project_view(&p).unwrap();
        let before = r.core.current_project(&p_view).name;
        r.core.update_project_name(&p.0, "Mission Control");
        assert_eq!(r.core.current_project(&p_view).name, "Mission Control");
        assert_eq!(r.core.current_project(&ViewId::main()).name, "Mission Control", "the main view shows the project too");
        assert_ne!(before, "Mission Control");
        assert_eq!(r.core.current_project(&q_view).name, q.name, "a view of another project keeps its name");
        assert_eq!(r.core.snapshot(&p_view).project, "Mission Control");
    }

    #[test]
    fn view_after_open_accepts_an_added_or_existing_view_and_refuses_a_vanished_one() {
        let view = ViewId::project(&discard_core::model::ProjectId("p".into()));
        assert_eq!(view_after_open(view.clone(), true, true), Ok(view.clone()));
        // Another open made the view first: it is there, so it is returned.
        assert_eq!(view_after_open(view.clone(), false, true), Ok(view.clone()));
        // `open_view` added nothing and the view is not there (a close ran in between): no view.
        assert_eq!(view_after_open(view, false, false), Err("This window is closed.".into()));
    }

    #[tokio::test]
    async fn views_that_moved_are_refreshed_when_another_view_cannot_move() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (q, _) = second_project(&r);
        let q_view = r.service.open_project_view(&q.id).unwrap();
        let p_view = r.service.open_project_view(&p).unwrap();
        let notes = r.service.create_text_channel(&p_view, &p, "notes").unwrap();
        r.service.select(&ViewId::main(), &notes).unwrap();
        for view in [&ViewId::main(), &p_view, &q_view] { settle_view(&r.core, view).await; }
        // The main view sorts first and cannot move: its navigation runs. The project view moves after it.
        r.core.begin_project_navigation(&ViewId::main()).unwrap();
        r.git(&["branch", "late"]);
        let has_late = |view: &ViewId| r.core.snapshot(view).branches.iter().any(|row| row.label == "late");
        assert!(!has_late(&p_view) && !has_late(&q_view));
        // The caller is a third view, so only the refresh of the moved views can show the new branch.
        let error = r.service.move_views(&q_view, &notes, &r.general.id.0).unwrap_err();
        assert!(error.contains("Finish the open confirmation"), "{error}");
        assert_eq!((r.core.active_channel_id(&p_view), r.core.active_channel_id(&ViewId::main())), (r.general.id.0.clone(), notes.clone()));
        r.core.end_project_navigation(&ViewId::main());
        for view in [&ViewId::main(), &p_view, &q_view] { settle_view(&r.core, view).await; }
        assert!(has_late(&p_view), "the view that moved was refreshed");
        assert!(!has_late(&ViewId::main()), "the view that did not move was not refreshed");
    }

    #[tokio::test]
    async fn a_folder_change_reaches_every_view_of_the_project() {
        let r = rig().await;
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        for view in [&ViewId::main(), &p_view] { settle_view(&r.core, view).await; }
        let moved = r.dir.path().join("moved");
        std::fs::create_dir(&moved).unwrap();
        run(&moved, &["init", "-q", "-b", "trunk2"]);
        std::fs::write(moved.join("b.txt"), "two\n").unwrap();
        run(&moved, &["add", "-A"]);
        run(&moved, &["commit", "-q", "-m", "init"]);
        let labels = |view: &ViewId| r.core.snapshot(view).branches.into_iter().map(|row| row.label).collect::<Vec<_>>();
        assert!(labels(&p_view).contains(&"main".to_owned()) && !labels(&p_view).contains(&"trunk2".to_owned()));
        r.store.lock().unwrap().change_project_folder(&p, &moved).unwrap();
        let path = r.store.lock().unwrap().visible_projects().unwrap().into_iter().find(|project| project.id == p).unwrap().path;
        assert_ne!(path, r.repo.to_string_lossy());
        // The main view calls, as `choose_project_folder` does from the main window.
        r.core.begin_project_change().unwrap();
        let reloaded = r.service.reload_project(&p);
        r.core.end_project_change();
        reloaded.unwrap();
        for view in [&ViewId::main(), &p_view] { settle_view(&r.core, view).await; }
        for view in [&ViewId::main(), &p_view] {
            assert_eq!(r.core.current_project(view).path, path, "{view:?} has the new path");
            assert!(labels(view).contains(&"trunk2".to_owned()), "{view:?} lists the branches of the new folder: {:?}", labels(view));
            assert!(!labels(view).contains(&"main".to_owned()), "{view:?} no longer lists the old folder");
        }
        assert_eq!(r.core.active_channel_id(&p_view), r.general.id.0, "the project view keeps its channel");
    }

    #[tokio::test]
    async fn removing_a_project_moves_the_main_view_and_no_project_view() {
        let r = rig().await;
        let p = r.project.id.clone();
        let (q, q_general) = second_project(&r);
        let p_view = r.service.open_project_view(&p).unwrap();
        let q_view = r.service.open_project_view(&q.id).unwrap();
        // A project that the main view does not show: nothing moves.
        r.service.leave_removed_project(&q.id).unwrap();
        assert_eq!(r.core.current_project(&ViewId::main()).id, p);
        // The main view leaves p for the next project. The view of p keeps its project and channel until its window closes.
        r.service.leave_removed_project(&p).unwrap();
        assert_eq!(r.core.current_project(&ViewId::main()).id, q.id);
        assert_eq!(r.core.active_channel_id(&ViewId::main()), q_general.id.0);
        assert_eq!((r.core.current_project(&p_view).id, r.core.active_channel_id(&p_view)), (p.clone(), r.general.id.0.clone()));
        assert_eq!((r.core.current_project(&q_view).id, r.core.active_channel_id(&q_view)), (q.id.clone(), q_general.id.0.clone()));
    }

    #[tokio::test]
    async fn removing_the_last_project_leaves_the_main_view_with_no_project() {
        let r = rig().await;
        let p = r.project.id.clone();
        let p_view = r.service.open_project_view(&p).unwrap();
        r.service.leave_removed_project(&p).unwrap();
        assert_eq!(r.core.current_project(&ViewId::main()).id.0, "");
        assert_eq!((r.core.current_project(&p_view).id, r.core.active_channel_id(&p_view)), (p, r.general.id.0.clone()), "a project view never moves to no project");
    }

    // Conversation moves: the approval of a request that the project main agent made.

    #[tokio::test]
    async fn move_to_an_existing_channel_posts_the_handoff_and_starts_the_target() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let topic = r.store.lock().unwrap().channel_topic(&feature).unwrap();
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        r.received(&feature);
        let done = r.decide(&request.id, true).unwrap();
        assert_eq!((done.state.as_str(), done.result.as_deref()), ("completed", Some("Moved to #feature.")));
        r.assert_handoff_posted(&feature);
        r.assert_continued(&feature);
        r.assert_moved_to(&feature, "feature");
        assert_eq!(r.messages(&r.general.id).len(), 2, "the source holds its user message and the link only");
        assert!(r.received(&r.general.id).is_empty(), "no response starts in the source");
        assert_eq!(r.core.active_channel_id(&ViewId::main()), r.general.id.0);
        assert_eq!(r.store.lock().unwrap().channel_topic(&feature).unwrap(), topic);
        assert_eq!(r.requests.list(&r.general.id)[0].state, "completed");
    }

    #[tokio::test]
    async fn move_carries_the_named_attachments_in_the_handoff_and_nothing_else() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let file = |id: &str, name: &str| discard_core::attachments::Attachment {
            id: id.into(),
            name: name.into(),
            media_type: "text/plain".into(),
            size: 3,
            path: r.dir.path().join(name),
            text: Some("abc".into()),
        };
        let (kept, left) = (file("att-1", "notes.txt"), file("att-2", "other.txt"));
        r.say(vec![
            Block::Attachment {
                attachment: kept.clone(),
            },
            Block::Attachment { attachment: left },
        ]);
        let message = r.say(vec![Block::Markdown {
            text: "lets move this and make the changes".into(),
        }]);
        let details = discard_core::write_requests::MoveDetails {
            target: r.to_channel(&feature, "feature"),
            handoff: Rig::handoff(),
            source_message_id: message.0.clone(),
            attachments: vec![discard_core::write_requests::MoveAttachment {
                id: "att-1".into(),
                name: "notes.txt".into(),
            }],
        };
        let request = r
            .requests
            .request_move(&r.general.id, &r.agent, details)
            .unwrap();
        r.received(&feature);
        assert_eq!(r.decide(&request.id, true).unwrap().state, "completed");
        let handoff = &r.messages(&feature)[1];
        assert_eq!(handoff.role, Role::Agent);
        assert_eq!(
            handoff.blocks[1..],
            [Block::Attachment { attachment: kept }]
        );
        assert_eq!(handoff.blocks.len(), 2, "only the named file moves");
        r.assert_continued(&feature);
        assert!(
            r.messages(&feature).iter().all(|m| m.role != Role::User),
            "the user's message is not copied"
        );
    }

    #[tokio::test]
    async fn a_move_fails_before_posting_when_a_named_attachment_is_gone() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown {
            text: "move it".into(),
        }]);
        let details = discard_core::write_requests::MoveDetails {
            target: r.to_channel(&feature, "feature"),
            handoff: Rig::handoff(),
            source_message_id: message.0.clone(),
            attachments: vec![discard_core::write_requests::MoveAttachment {
                id: "gone".into(),
                name: "gone.txt".into(),
            }],
        };
        let request = r
            .requests
            .request_move(&r.general.id, &r.agent, details)
            .unwrap();
        assert_eq!(r.decide(&request.id, true).unwrap().state, "failed");
        assert!(
            r.messages(&feature).is_empty(),
            "a failed move posts nothing"
        );
    }

    #[tokio::test]
    async fn move_to_a_new_branch_creates_the_branch_worktree_channel_and_topic() {
        let r = rig().await;
        let base = r.git(&["rev-parse", "HEAD"]);
        r.git(&["commit", "--allow-empty", "-qm", "second"]);
        assert_ne!(r.git(&["rev-parse", "HEAD"]), base);
        let message = r.say(vec![Block::Markdown { text: "Start the importer".into() }]);
        let request = r.move_request(r.to_branch("importer", &base), &message);
        let done = r.decide(&request.id, true).unwrap();
        assert_eq!((done.state.as_str(), done.result.as_deref()), ("completed", Some("Moved to #importer.")));
        assert_eq!(r.git(&["rev-parse", "refs/heads/importer"]), base);
        let channel = r.branch_channel("importer").expect("the branch has a channel");
        assert_eq!((channel.name.as_str(), channel.workspace_mode, &channel.agent_id), ("importer", WorkspaceMode::Worktree, &r.agent));
        let record = r.worktree_record(&channel.id).expect("the channel owns a worktree");
        assert!(record.owned && record.branch == "importer" && record.base_commit == base, "{record:?}");
        assert!(std::path::Path::new(&record.path).join("a.txt").exists());
        let topic = r.store.lock().unwrap().channel_topic_history(&channel.id).unwrap();
        assert_eq!((topic.len(), topic[0].version, &topic[0].text, topic[0].author_kind, topic[0].author_id.as_ref()), (1, 1, &Rig::handoff().markdown(), discard_core::model::TopicAuthorKind::Agent, Some(&r.agent)));
        r.assert_handoff_posted(&channel.id);
        r.assert_continued(&channel.id);
        r.assert_moved_to(&channel.id, "importer");
        assert!(r.core.has_channel(&channel.id.0));
        assert_eq!(r.core.active_channel_id(&ViewId::main()), r.general.id.0);
        assert!(r.received(&r.general.id).is_empty());
    }

    #[tokio::test]
    async fn a_new_branch_channel_takes_a_free_name() {
        let r = rig().await;
        r.text_channel("importer");
        let base = r.git(&["rev-parse", "HEAD"]);
        let message = r.say(vec![Block::Markdown { text: "Start".into() }]);
        let request = r.move_request(r.to_branch("importer", &base), &message);
        assert_eq!(r.decide(&request.id, true).unwrap().result.as_deref(), Some("Moved to #importer-2."));
        assert_eq!(r.branch_channel("importer").unwrap().name, "importer-2");
    }

    #[tokio::test]
    async fn a_denied_move_posts_a_note_and_changes_nothing() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let base = r.git(&["rev-parse", "HEAD"]);
        let message = r.say(vec![Block::Markdown { text: "Start the importer".into() }]);
        let request = r.move_request(r.to_branch("importer", &base), &message);
        let channels = r.channel_count();
        r.received(&feature);
        let done = r.decide(&request.id, false).unwrap();
        assert_eq!((done.state.as_str(), done.result.as_deref()), ("denied", Some("Conversation move denied.")));
        let last = r.messages(&r.general.id).pop().unwrap();
        assert_eq!((last.role, last.blocks), (Role::System, vec![Block::Markdown { text: "Conversation move denied.".into() }]));
        assert!(!r.branches().contains(&"importer".to_string()));
        assert_eq!(r.channel_count(), channels);
        assert!(r.messages(&feature).is_empty());
        assert!(r.received(&feature).is_empty() && r.received(&r.general.id).is_empty());
    }

    #[tokio::test]
    async fn a_move_waits_for_the_source_and_the_target() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        // A turn runs in the source.
        r.busy(&r.general.id.0);
        assert_eq!(r.decide(&request.id, true).unwrap_err(), "Wait for the current response to finish.");
        r.idle(&r.general.id.0);
        // A turn runs in the target. Open it to start the turn, then open the source again.
        r.service.select(&ViewId::main(), &feature.0).unwrap();
        r.busy(&feature.0);
        r.service.select(&ViewId::main(), &r.general.id.0).unwrap();
        assert_eq!(r.decide(&request.id, true).unwrap_err(), "#feature is busy: A turn runs in this channel. Stop it first.");
        assert_eq!(r.requests.list(&r.general.id)[0].state, "pending");
        assert!(r.messages(&feature).is_empty(), "a refusal posts nothing");
        assert!(!r.core.project_change_running(), "a refusal releases the guard");
        r.idle(&feature.0);
        r.received(&feature);
        assert_eq!(r.decide(&request.id, true).unwrap().state, "completed");
        r.assert_handoff_posted(&feature);
        r.assert_continued(&feature);
    }

    #[tokio::test]
    async fn a_move_is_approved_once() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        assert_eq!(r.decide(&request.id, true).unwrap().state, "completed");
        assert_eq!(r.decide(&request.id, true).unwrap_err(), "This request is no longer pending.");
        r.assert_handoff_posted(&feature);
        assert_eq!(r.received(&feature).len(), 1);
    }

    #[tokio::test]
    async fn a_move_fails_cleanly_when_state_changed() {
        let gate = "Only this project's Primary / Moderator agent can move a conversation. Suggest the move in your reply.";
        // The main agent changed.
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Move this".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        let other = r.store.lock().unwrap().create_agent(&AgentConfig { workspace_write: false, sandbox_autorun: false, native_resume: false, delegation: false, reasoning_effort: None, name: "Other".into(), provider: "codex".into(), model: Some("model".into()), avatar: None, system_prompt: String::new(), policy: ApprovalPolicy::Ask, plugin_write: false }).unwrap();
        r.store.lock().unwrap().set_project_main_agent(&r.project.id, &other).unwrap();
        r.assert_failed(&request.id, gate, &[&feature]);
        // The target channel was archived.
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Move this".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        r.store.lock().unwrap().archive_channel(&feature).unwrap();
        r.assert_failed(&request.id, "The target channel is no longer available.", &[&feature]);
        // The source message was deleted: the request names no message.
        let r = rig().await;
        let feature = r.text_channel("feature");
        let request = r.move_request(r.to_channel(&feature, "feature"), &MessageId::new());
        r.assert_failed(&request.id, "The message to move is no longer available. Ask again.", &[&feature]);
        // The new branch name was taken.
        let r = rig().await;
        let base = r.git(&["rev-parse", "HEAD"]);
        let message = r.say(vec![Block::Markdown { text: "Move this".into() }]);
        let request = r.move_request(r.to_branch("x", &base), &message);
        r.git(&["branch", "x"]);
        let channels = r.channel_count();
        r.assert_failed(&request.id, "A branch named `x` already exists.", &[]);
        assert_eq!(r.channel_count(), channels);
    }

    #[tokio::test]
    async fn a_move_is_refused_after_a_newer_user_message() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        r.say(vec![Block::Markdown { text: "Wait, one more thing".into() }]);
        r.assert_failed(&request.id, "You sent another message after this request. Ask for the move again.", &[&feature]);
        assert_eq!(r.requests.list(&r.general.id)[0].state, "failed");
    }

    #[tokio::test]
    async fn a_move_fails_cleanly_when_the_base_commit_is_gone() {
        let r = rig().await;
        let message = r.say(vec![Block::Markdown { text: "Start the importer".into() }]);
        let request = r.move_request(r.to_branch("importer", &"0".repeat(40)), &message);
        let channels = r.channel_count();
        r.assert_failed(&request.id, "The base commit is no longer available. Ask again.", &[]);
        assert!(!r.branches().contains(&"importer".to_string()), "{:?}", r.branches());
        assert_eq!(r.channel_count(), channels);
        assert!(r.branch_channel("importer").is_none());
    }

    #[tokio::test]
    async fn move_to_an_existing_channel_that_is_not_loaded_starts_it() {
        let r = rig().await;
        // The store holds the channel, but the pool does not.
        let feature = r.store.lock().unwrap().create_project_text_channel(&r.project.id, "feature").unwrap().id;
        assert!(!r.core.has_channel(&feature.0));
        assert!(!r.launched.lock().unwrap().contains(&feature.0));
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        let done = r.decide(&request.id, true).unwrap();
        assert_eq!((done.state.as_str(), done.result.as_deref()), ("completed", Some("Moved to #feature.")));
        assert!(r.launched.lock().unwrap().contains(&feature.0), "the approval launched the channel");
        assert!(r.core.has_channel(&feature.0));
        r.assert_handoff_posted(&feature);
        r.assert_continued(&feature);
        r.assert_moved_to(&feature, "feature");
    }

    #[tokio::test]
    async fn a_move_to_a_target_that_cannot_take_a_send_posts_nothing() {
        let r = rig().await;
        let feature = r.text_channel("feature");
        let message = r.say(vec![Block::Markdown { text: "Please ship the importer".into() }]);
        let request = r.move_request(r.to_channel(&feature, "feature"), &message);
        r.core.channel_read_only(&feature.0, true, false);
        r.assert_failed(&request.id, "This channel cannot send a message right now.", &[&feature]);
        assert_eq!(r.requests.list(&r.general.id)[0].state, "failed");
    }

    #[tokio::test]
    async fn a_failed_worktree_leaves_no_branch_and_no_channel() {
        let r = rig().await;
        let base = r.git(&["rev-parse", "HEAD"]);
        let message = r.say(vec![Block::Markdown { text: "Move this".into() }]);
        let request = r.move_request(r.to_branch("importer", &base), &message);
        std::fs::write(r.dir.path().join("worktrees"), "not a folder").unwrap();
        let channels = r.channel_count();
        let done = r.decide(&request.id, true).unwrap();
        assert_eq!(done.state, "failed");
        assert!(done.result.as_deref().unwrap().starts_with("Conversation move failed: "), "{done:?}");
        assert!(!r.branches().contains(&"importer".to_string()), "{:?}", r.branches());
        assert_eq!(r.channel_count(), channels);
        assert!(r.branch_channel("importer").is_none());
        let last = r.messages(&r.general.id).pop().unwrap();
        assert_eq!((last.role, last.blocks), (Role::System, vec![Block::Markdown { text: done.result.unwrap() }]));
        assert!(r.received(&r.general.id).is_empty());
    }
}
