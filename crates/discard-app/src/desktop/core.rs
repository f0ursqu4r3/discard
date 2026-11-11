//! State shared between engine updates and desktop commands.
use super::dto::*;
use crate::plugin_memory::PluginMemory;
use super::threads::ThreadRecord;
use discard_core::model::{AgentId, Message, PostedSource, ThreadKind, ThreadState};
use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::timeline::ApprovalView;
use discard_core::ui_state::{Command, SlotLabels, UiState, Update, new_divider_anchor, startup_marker};
use discard_lua::{PluginService, LoadReport};
use discard_core::branch_tree::{TreeRow, RowKind, current_row};
use discard_core::window_state::{Bounds, PanelWidths, StateStore};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc::UnboundedSender};

fn display_project_path(path: &str, home: Option<&Path>) -> String {
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty())
        && let Ok(relative) = Path::new(path).strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() { "~".into() }
            else { Path::new("~").join(relative).to_string_lossy().into_owned() };
    }
    path.to_owned()
}

/// The approval prompts of a conversation, with the always-allow rules its session profile allows.
fn prompt_dtos(ui: &UiState) -> Vec<PromptDto> {
    ui.prompts()
        .iter()
        .map(|p| PromptDto {
            kind: p.kind.as_str().into(),
            rule_words: matches!(ui.session_profile, discard_core::ui_state::SessionProfile::Workspace { .. }).then(|| p.rule_words.clone()).flatten(),
            default_cut: matches!(ui.session_profile, discard_core::ui_state::SessionProfile::Workspace { .. }).then(|| p.rule_words.as_ref().map(|words| discard_core::always_allow::default_cut(words))).flatten(),
            rule_host: matches!(ui.session_profile, discard_core::ui_state::SessionProfile::Workspace { .. }).then(|| p.rule_host.clone()).flatten(),
            always_allow_note: if matches!(ui.session_profile, discard_core::ui_state::SessionProfile::Workspace { .. }) {
                if p.kind == discard_core::approval::ApprovalKind::SandboxEscape && p.tool == "command" { Some("Always allow is unavailable for Codex write sessions.".into()) }
                else if p.kind == discard_core::approval::ApprovalKind::ProviderTool && p.rule_words.is_none() { Some("This command uses shell syntax that Always allow cannot match.".into()) }
                else { None }
            } else { None },
            always_allow: match ui.session_profile {
                discard_core::ui_state::SessionProfile::Workspace { .. } if matches!(p.kind, discard_core::approval::ApprovalKind::ProviderTool | discard_core::approval::ApprovalKind::NetworkDomain) && (p.rule_words.is_some() || p.rule_host.is_some()) => "available",
                discard_core::ui_state::SessionProfile::Coding if p.kind == discard_core::approval::ApprovalKind::ProviderTool => "available",
                _ => "unavailable",
            }.into(),
            approval_id: p.approval_id.to_string(),
            tool: p.tool.clone(),
            summary: p.summary.clone(),
            view: match p.view {
                ApprovalView::Pending => "pending",
                ApprovalView::Approved => "approved",
                ApprovalView::Denied => "denied",
                ApprovalView::Stale => "stale",
            }
            .into(),
        })
        .collect()
}

fn channel_state<'a>(s: &'a State, id: &str) -> Option<&'a ChannelState> {
    if s.active.channel_id == id { Some(&s.active) } else { s.parked.get(id) }
}

fn agent_status_name(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::AwaitingApproval => "awaiting_approval",
        AgentStatus::Offline => "offline",
    }
}

/// The transcript of one channel. A parent transcript carries the thread chips and the posted-from links. A
/// thread transcript marks the messages that are also posted to the parent.
fn channel_messages(s: &State, channel: &ChannelState) -> Vec<MessageDto> {
    let mut messages = history_dto_with_authors(channel.ui.history(), channel.ui.authors());
    if let Some(parent) = channel.thread_parent.as_deref() {
        let posted: HashSet<&str> = channel_state(s, parent).into_iter().flat_map(|p| p.posted.values())
            .filter(|source| source.thread_channel_id.0 == channel.channel_id).map(|source| source.source_message_id.0.as_str()).collect();
        for message in &mut messages { message.posted_to_channel = posted.contains(message.id.as_str()); }
        return messages;
    }
    // Chips come in creation order: the thread map is ordered by thread ID, which is time-ordered.
    let mut chips: HashMap<&str, ThreadChipDto> = HashMap::new();
    let mut sub_agent_chips: HashMap<&str, Vec<ThreadChipDto>> = HashMap::new();
    for record in s.threads.values().filter(|r| r.parent == channel.channel_id && r.visible() && !r.anchor.is_empty()) {
        let chip = record.chip(s.window.last_read(&record.id), thread_approvals(s, &record.id));
        match record.kind {
            ThreadKind::Question => { chips.insert(record.anchor.as_str(), chip); }
            ThreadKind::SubAgent => sub_agent_chips.entry(record.anchor.as_str()).or_default().push(chip),
        }
    }
    for message in &mut messages {
        message.thread = chips.get(message.id.as_str()).cloned();
        message.sub_agent_threads = sub_agent_chips.get(message.id.as_str()).cloned().unwrap_or_default();
        message.posted_from = channel.posted.get(&discard_core::model::MessageId(message.id.clone()))
            .map(|source| PostedFromDto { source_message_id: source.source_message_id.0.clone(), thread_channel_id: source.thread_channel_id.0.clone(), thread_name: source.thread_name.clone() });
    }
    messages
}

/// Approvals that wait in a loaded thread.
fn thread_approvals(s: &State, thread: &str) -> usize { s.parked.get(thread).map_or(0, |c| c.ui.pending_approvals()) }

/// The followed threads of a channel with live state, oldest first.
fn followed_links(s: &State, channel: &str) -> Vec<ThreadLinkDto> {
    s.threads.values().filter(|r| r.parent == channel && r.followed && r.visible()).map(|r| {
        let live = s.parked.get(&r.id);
        r.link(s.window.last_read(&r.id), live.is_some_and(|c| c.ui.turn_running()), live.map_or(0, |c| c.ui.pending_approvals()))
    }).collect()
}

/// The sub-agent threads of the running turn of `channel`: they have no anchor yet.
fn live_sub_agent_chips(s: &State, channel: &str) -> Vec<ThreadChipDto> {
    s.threads.values().filter(|r| r.parent == channel && r.kind == ThreadKind::SubAgent && r.anchor.is_empty())
        .map(|r| r.chip(s.window.last_read(&r.id), thread_approvals(s, &r.id))).collect()
}

/// Every loaded thread with an approval that waits, followed or not.
fn pending_thread_approvals(s: &State) -> Vec<ThreadApprovalDto> {
    s.threads.values().filter_map(|r| {
        let pending = thread_approvals(s, &r.id);
        (pending > 0).then(|| ThreadApprovalDto { thread_id: r.id.clone(), parent_channel_id: r.parent.clone(), name: r.name.clone(), kind: r.kind.as_str().into(), agent_name: r.agent_name.clone(), pending_approvals: pending })
    }).collect()
}

/// A thread is read-only when it is, or its parent channel is, read-only or archived.
fn thread_blocked(s: &State, thread: &ChannelState) -> bool {
    thread.read_only || thread.archived || thread.thread_parent.as_deref().and_then(|parent| channel_state(s, parent)).is_some_and(|parent| parent.archived)
}

fn open_thread_dto(s: &State) -> Option<OpenThreadDto> {
    let id = s.open_thread.as_ref()?;
    let channel = s.parked.get(id)?;
    let record = s.threads.get(id)?;
    let ui = &channel.ui;
    let blocked = thread_blocked(s, channel);
    Some(OpenThreadDto {
        channel_id: id.clone(), parent_channel_id: record.parent.clone(), name: record.name.clone(), followed: record.followed,
        kind: record.kind.as_str().into(), state: record.state.map(|state| state.as_str().into()), agent_name: record.agent_name.clone(),
        anchor_message_id: Some(record.anchor.clone()).filter(|anchor| !anchor.is_empty()), history_rev: s.thread_rev,
        agent_id: channel.agent_id.clone(), agent: channel.agent.clone(), provider: channel.provider.clone(),
        responder: ui.responder().cloned(), agent_status: agent_status_name(ui.agent_status()).into(), turn_running: ui.turn_running(),
        notice: ui.notice().map(str::to_owned), live_blocks: ui.live_blocks().to_vec(), prompts: prompt_dtos(ui),
        pending_approvals: ui.pending_approvals(), unread: record.unread(s.window.last_read(id)),
        composer_disabled: blocked,
        composer_disabled_reason: blocked.then(|| "read_only".into()),
        read_only_note: discard_core::workspace_session::ReadOnlyReason::Thread.note(),
    })
}

/// Rebuild a loaded thread's record from its transcript, then tell the views that show it.
fn refresh_thread(s: &mut State, thread: &str) {
    let Some(channel) = s.parked.get(thread) else { return };
    let Some(record) = s.threads.get_mut(thread) else { return };
    record.refresh(channel.ui.history(), channel.ui.authors());
    let parent = record.parent.clone();
    touch_thread(s, &parent, thread);
}

/// Tell the views that show a thread that its record changed: the parent transcript with the chip, the open panel.
fn touch_thread(s: &mut State, parent: &str, thread: &str) {
    if s.open_thread.as_deref() == Some(thread) { s.thread_rev += 1; }
    if s.active.channel_id == parent { s.history_rev += 1; }
}

/// Add a thread message to its parent channel as a reference row, once. Returns whether the parent changed.
fn post_into_parent(s: &mut State, thread: &str, reference: Message, source: PostedSource) -> bool {
    let Some(parent_id) = s.parked.get(thread).and_then(|t| t.thread_parent.clone()) else { return false };
    let author = s.parked.get(thread).and_then(|t| t.ui.authors().get(&source.source_message_id).cloned());
    let marker = s.window.last_read(&parent_id).map(str::to_owned);
    let is_active = s.active.channel_id == parent_id;
    let parent = if is_active { Some(&mut s.active) } else { s.parked.get_mut(&parent_id) };
    let Some(parent) = parent else { return false };
    parent.posted.insert(reference.id.clone(), source);
    let added = !parent.ui.history().iter().any(|m| m.id == reference.id);
    if added {
        if let Some(author) = author { parent.ui.set_author(reference.id.clone(), author); }
        parent.ui.apply(Update::ConversationMarker(reference));
        parent.new_anchor = new_divider_anchor(parent.ui.history(), marker.as_deref(), parent.new_anchor.as_deref(), false);
    }
    if is_active { s.history_rev += 1; }
    if s.open_thread.as_deref() == Some(thread) { s.thread_rev += 1; }
    added
}

/// The conversation a command for `channel` acts on: the selected channel, or a loaded thread.
fn command_target<'a>(s: &'a State, channel: Option<&str>) -> Option<&'a ChannelState> {
    match channel {
        None => Some(&s.active),
        Some(id) if id == s.active.channel_id => Some(&s.active),
        Some(id) => s.parked.get(id).filter(|c| c.thread_parent.is_some()),
    }
}

/// What an update means for the sub-agent tasks and thread records.
pub(crate) enum SubAgentNote {
    None,
    /// The text of an `Update::Error` or `Update::ResponseSaveFailed`: kept as the detail of a task that does not finish.
    Detail(String),
    /// The text of an `Update::SetupNeeded`: a detail, and the end of the turn.
    SetupNeeded(String),
    Finished { state: ThreadState, text: String },
    TurnEnded,
    Followed,
    Anchored(Vec<discard_core::model::ThreadInfo>),
}
impl SubAgentNote {
    fn of(update: &Update) -> Self {
        match update {
            Update::Error(text) | Update::ResponseSaveFailed(text) => Self::Detail(text.clone()),
            Update::SetupNeeded(text) => Self::SetupNeeded(text.clone()),
            Update::SubAgentFinished { state, text } => Self::Finished { state: *state, text: text.clone() },
            Update::TurnFinished(_) => Self::TurnEnded,
            Update::ThreadFollowed => Self::Followed,
            Update::ThreadsAnchored(infos) => Self::Anchored(infos.clone()),
            _ => Self::None,
        }
    }
}

pub struct CoreInit {
    pub state: UiState,
    pub commands: UnboundedSender<Command>,
    pub project: String,
    pub channel: String,
    pub agent: String,
    pub provider: String,
}
struct ChannelState {
    ui: UiState,
    runtime_generation: u64,
    commands: UnboundedSender<Command>,
    channel: String,
    project_name: String,
    agent: String,
    provider: String,
    channel_id: String,
    agent_id: String,
    project_id: Option<discard_core::model::ProjectId>,
    mode: discard_core::model::WorkspaceMode,
    new_anchor: Option<String>,
    archive: Option<String>,
    archived: bool,
    archive_busy: bool,
    read_only: bool,
    /// The parent channel when this state belongs to a thread. A thread is never the selected channel.
    thread_parent: Option<String>,
    /// Reference rows of this channel: the thread message each one shows.
    posted: HashMap<discard_core::model::MessageId, PostedSource>,
}
impl ChannelState {
    fn new(init: CoreInit) -> Self {
        let channel_id = init.state.channel_id().0.clone();
        Self { ui: init.state, runtime_generation: 0, commands: init.commands, channel: init.channel, project_name: init.project, agent: init.agent,
            provider: init.provider, channel_id, agent_id: String::new(), project_id: None,
            mode: discard_core::model::WorkspaceMode::MainCheckout, new_anchor: None,
            archive: None, archived: false, archive_busy: false, read_only: false, thread_parent: None, posted: HashMap::new() }
    }
}
struct State {
    project: String,
    project_busy: bool,
    changing_agents: HashSet<String>,
    changing_channels: HashSet<String>,
    runtime_generation: u64,
    active: ChannelState,
    parked: BTreeMap<String, ChannelState>,
    rev: u64,
    history_rev: u64,
    memory_rev: u64,
    /// Every thread the app knows, by thread channel ID. Loaded threads keep their record current.
    threads: BTreeMap<String, ThreadRecord>,
    /// The thread shown beside the selected channel.
    open_thread: Option<String>,
    thread_rev: u64,
    theme: ThemeDto,
    window: StateStore,
    branches: Vec<TreeRow>,
    branch_repositories: Vec<String>,
    repositories: Vec<RepositoryDto>,
    repositories_loading: bool,
    current_branch: Option<usize>,
    collapsed: HashSet<String>,
    project_id: String,
    project_path: String,
    confirm: Option<super::plan::Confirm>,
    plugin_notice: Option<String>,
}
#[derive(Clone)]
struct BranchSource {
    store: Arc<Mutex<discard_core::store::Store>>,
    project: PathBuf,
}
pub struct AppCore {
    state: Mutex<State>,
    plugins: Mutex<Option<Arc<PluginService>>>,
    load_report: Mutex<LoadReport>,
    slot_cache: Mutex<super::slots::SlotCache>,
    plugin_signature: Mutex<String>,
    branch_source: Mutex<Option<BranchSource>>,
    branch_generation: AtomicU64,
    /// Moves when a thread list that the webview reads from storage changed without a command of the webview.
    conversations_rev: AtomicU64,
    sub_agents: super::delegation::SubAgents,
    pub(crate) changed: Notify,
}
impl AppCore {
    pub fn new(init: CoreInit) -> Self {
        let project = init.project.clone();
        Self {
            state: Mutex::new(State {
                project,
                project_busy: false,
                changing_agents: HashSet::new(),
                changing_channels: HashSet::new(),
                runtime_generation: 0,
                active: ChannelState::new(init),
                parked: BTreeMap::new(),
                window: StateStore::open(Path::new("")),
                branches: vec![],
                branch_repositories: vec![],
                repositories: vec![],
                repositories_loading: true,
                current_branch: None,
                collapsed: HashSet::new(),
                project_id: String::new(),
                project_path: String::new(),
                confirm: None,
                plugin_notice: None,
                rev: 0,
                history_rev: 0,
                memory_rev: 0,
                threads: BTreeMap::new(),
                open_thread: None,
                thread_rev: 0,
                theme: ThemeDto {
                    mono_font: None,
                    mode: "dark".into(),
                    tokens: BTreeMap::new(),
                    radius: None,
                    font_scale: 1.0,
                    spacing_scale: 1.0,
                    protected_bg: "#1e1e1e".into(),
                    protected_fg: "#f0f0f0".into(),
                    protected_border: "#ffaa00".into(),
                    diff_colors: DiffColorsDto { added: "#77cc88".into(), removed: "#ff7777".into(), inline_added: "#77cc88".into(), inline_removed: "#ff7777".into() },
                },
            }),
            plugins: Mutex::new(None),
            load_report: Mutex::new(LoadReport::default()),
            slot_cache: Mutex::new(super::slots::SlotCache::default()),
            plugin_signature: Mutex::new(String::new()),
            branch_source: Mutex::new(None),
            branch_generation: AtomicU64::new(0),
            conversations_rev: AtomicU64::new(0),
            sub_agents: super::delegation::SubAgents::default(),
            changed: Notify::new(),
        }
    }
    /// The running sub-agent tasks.
    pub fn sub_agents(&self) -> &super::delegation::SubAgents { &self.sub_agents }
    /// Moves when the engine followed a thread or a followed sub-agent thread changed state. The pump tells the
    /// webview (`discard://conversations`) so lists that it reads from storage load again.
    pub fn conversations_rev(&self) -> u64 { self.conversations_rev.load(Ordering::SeqCst) }
    pub fn clear_project(&self) {
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let project = discard_core::model::Project { id: discard_core::model::ProjectId(String::new()), name: String::new(), path: String::new() };
        let channel = discard_core::model::Channel { id: discard_core::model::ChannelId(String::new()), project_id: None, name: String::new(), agent_id: AgentId(String::new()), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        self.replace_project(CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() }, &project, &channel);
        *self.branch_source.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.clear_slots();
    }
    /// Browse projects independently of the active conversation.
    pub fn select_sidebar_project(&self, project: Option<&discard_core::model::Project>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.project = project.map(|p| p.name.clone()).unwrap_or_default();
        s.project_id = project.map(|p| p.id.0.clone()).unwrap_or_default();
        s.project_path = project.map(|p| p.path.clone()).unwrap_or_default();
        s.branches.clear(); s.branch_repositories.clear(); s.repositories.clear(); s.repositories_loading = project.is_some();
        s.current_branch = None;
        s.collapsed.clear();
        self.branch_generation.fetch_add(1, Ordering::SeqCst);
        s.rev += 1;
        self.changed.notify_one();
    }

    pub fn update_project_name(&self, project_id: &str, name: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.project_id == project_id {
            s.project = name.to_owned();
            s.rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn saved_conversation(&self) -> Option<String> { self.state.lock().unwrap_or_else(|e| e.into_inner()).window.state().selected_conversation }
    /// Whether the channel has a turn that runs.
    pub fn channel_running(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::once(&s.active).chain(s.parked.values()).find(|c| c.channel_id == id).is_some_and(|c| c.ui.turn_running())
    }
    pub fn has_channel(&self, id: &str) -> bool { let s = self.state.lock().unwrap_or_else(|e| e.into_inner()); s.active.channel_id == id || s.parked.contains_key(id) }
    pub fn agent_channel_ids(&self, id: &str) -> Vec<String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::once(&s.active).chain(s.parked.values()).filter(|c| c.agent_id == id).map(|c| c.channel_id.clone()).collect()
    }
    pub fn channel_session_closed(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::once(&s.active).chain(s.parked.values()).find(|c| c.channel_id == id).is_some_and(|c| c.commands.is_closed())
    }
    /// Replace only a closed transport. Preserve transcript, notices and unsaved reply blocks.
    pub fn restart_closed_channel(&self, init: CoreInit, channel: &discard_core::model::Channel) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next_generation = s.runtime_generation + 1;
        let target = if s.active.channel_id == channel.id.0 { Some(&mut s.active) } else { s.parked.get_mut(&channel.id.0) };
        let Some(target) = target else { return Ok(false) };
        if !target.commands.is_closed() { return Ok(false); }
        if target.ui.turn_running() { return Err("Stop this conversation before restarting its session.".into()); }
        target.runtime_generation = next_generation;
        target.commands = init.commands; target.agent = init.agent; target.provider = init.provider;
        target.project_name = init.project; target.channel = init.channel;
        target.agent_id = channel.agent_id.0.clone();
        target.project_id = channel.project_id.clone();
        target.mode = channel.workspace_mode;
        s.runtime_generation = next_generation;
        s.rev += 1; self.changed.notify_one(); Ok(true)
    }
    pub fn begin_channel_change(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let channel = std::iter::once(&s.active).chain(s.parked.values()).find(|c| c.channel_id == id);
        if channel.is_some_and(|c| c.ui.turn_running() || c.archive_busy || c.archive.is_some()) || !s.changing_channels.insert(id.to_owned()) {
            return Err("Finish this channel's response or archive confirmation before changing its main agent or canonical status.".into());
        }
        Ok(())
    }
    pub fn end_channel_change(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.changing_channels.remove(id);
    }
    pub fn invalidate_idle_channel(&self, channel: &discard_core::model::Channel) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next_generation = s.runtime_generation + 1;
        let target = if s.active.channel_id == channel.id.0 { Some(&mut s.active) } else { s.parked.get_mut(&channel.id.0) };
        let Some(target) = target else { return Ok(false) };
        if target.ui.turn_running() || target.archive_busy { return Err("Stop this conversation before restarting its session.".into()); }
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        drop(receiver);
        target.runtime_generation = next_generation;
        target.commands = commands;
        target.agent_id = channel.agent_id.0.clone(); target.project_id = channel.project_id.clone(); target.mode = channel.workspace_mode;
        s.runtime_generation = next_generation;
        s.rev += 1; self.changed.notify_one(); Ok(true)
    }
    /// Invalidate only this project's idle transports. Retain all conversation UI state.
    /// The project-change guard blocks sends until fresh folder-bound runtimes are installed.
    pub fn invalidate_project_channels(&self, id: &discard_core::model::ProjectId) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.project_busy { return Err("Begin the project change before reloading its sessions.".into()); }
        if std::iter::once(&s.active).chain(s.parked.values()).any(|c| c.project_id.as_ref() == Some(id) && (c.ui.turn_running() || c.archive_busy)) {
            return Err("Stop this project's responses before restarting its sessions.".into());
        }
        let State { active, parked, runtime_generation, .. } = &mut *s;
        for channel in std::iter::once(active).chain(parked.values_mut()).filter(|c| c.project_id.as_ref() == Some(id)) {
            *runtime_generation += 1;
            channel.runtime_generation = *runtime_generation;
            let (commands, _) = tokio::sync::mpsc::unbounded_channel();
            channel.commands = commands;
        }
        s.rev += 1; self.changed.notify_one(); Ok(())
    }
    pub fn agent_sessions_closed(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::once(&s.active).chain(s.parked.values()).any(|c| c.agent_id == id && c.commands.is_closed())
    }
    pub fn begin_agent_change(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.changing_agents.contains(id) || std::iter::once(&s.active).chain(s.parked.values()).any(|c| (c.agent_id == id || c.ui.responder().is_some_and(|author| author.agent_id.0 == id)) && (c.ui.turn_running() || c.archive_busy)) {
            return Err("Stop this agent's responses before changing its lifecycle.".into());
        }
        s.changing_agents.insert(id.to_owned()); Ok(())
    }
    pub fn end_agent_change(&self, id: &str) { self.state.lock().unwrap_or_else(|e| e.into_inner()).changing_agents.remove(id); }
    pub fn refresh_agent(&self, id: &str, config: &discard_core::agent::AgentConfig, read_only: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let State { active, parked, .. } = &mut *s;
        for channel in std::iter::once(active).chain(parked.values_mut()).filter(|c| c.agent_id == id) {
            channel.agent = config.name.clone(); channel.provider = config.provider.clone(); channel.read_only = read_only;
        }
        s.rev += 1; drop(s); self.clear_slots(); self.changed.notify_one();
    }
    pub fn channel_read_only(&self, id: &str, read_only: bool, archived: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let channel = if s.active.channel_id == id { Some(&mut s.active) } else { s.parked.get_mut(id) };
        if let Some(channel) = channel { channel.read_only = read_only; channel.archived = archived; }
        s.rev += 1; self.changed.notify_one();
    }
    pub fn remove_agent_channels(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed: HashSet<String> = s.parked.iter().filter(|(_, c)| c.agent_id == id).map(|(channel, _)| channel.clone()).collect();
        s.parked.retain(|_, c| c.agent_id != id);
        let removed_active = s.active.agent_id == id;
        if removed_active { removed.insert(s.active.channel_id.clone()); }
        {
            let st = &mut *s;
            st.threads.retain(|thread, record| !removed.contains(thread) && !removed.contains(&record.parent));
            st.parked.retain(|thread, c| c.thread_parent.is_none() || st.threads.contains_key(thread));
        }
        if s.open_thread.as_ref().is_some_and(|thread| !s.parked.contains_key(thread)) { s.open_thread = None; s.thread_rev += 1; }
        if removed_active {
            let (commands, _) = tokio::sync::mpsc::unbounded_channel();
            s.active = ChannelState::new(CoreInit { state: UiState::new(discard_core::model::ChannelId(String::new()), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() });
            let _ = s.window.set_selected_conversation(None);
            s.history_rev += 1;
        }
        s.rev += 1; drop(s);
        if removed_active { self.clear_branch_source(); }
        self.changed.notify_one();
    }
    pub fn clear_branch_source(&self) {
        *self.branch_source.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.branch_generation.fetch_add(1, Ordering::SeqCst);
        self.set_branches(vec![], None);
    }
    pub fn current_project(&self) -> discard_core::model::Project {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        discard_core::model::Project { id: discard_core::model::ProjectId(s.project_id.clone()), name: s.project.clone(), path: s.project_path.clone() }
    }
    pub fn replace_project(&self, init: CoreInit, project: &discard_core::model::Project, channel: &discard_core::model::Channel) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.runtime_generation += 1;
        s.active = ChannelState::new(init);
        s.active.runtime_generation = s.runtime_generation;
        s.active.channel_id = channel.id.0.clone(); s.active.agent_id = channel.agent_id.0.clone(); s.active.mode = channel.workspace_mode; s.active.project_id = channel.project_id.clone();
        s.project = project.name.clone(); s.project_id = project.id.0.clone(); s.project_path = project.path.clone();
        s.parked.clear(); s.branches.clear(); s.branch_repositories.clear(); s.repositories.clear(); s.repositories_loading = false; s.collapsed.clear(); s.current_branch = None;
        s.threads.clear();
        if s.open_thread.take().is_some() { s.thread_rev += 1; }
        if let Some(marker) = startup_marker(s.active.ui.history(), s.window.last_read(&channel.id.0)) { let _ = s.window.set_last_read(&channel.id.0, &marker); }
        s.active.new_anchor = new_divider_anchor(s.active.ui.history(), s.window.last_read(&channel.id.0), None, false);
        s.rev += 1; s.history_rev += 1;
        self.branch_generation.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }
    pub fn begin_write_approval(&self, channel: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id != channel { return Err("Open this channel before approving.".into()); }
        if s.active.ui.turn_running() { return Err("Wait for the current response to finish.".into()); }
        if s.project_busy || s.active.archive_busy || s.active.archive.is_some() || s.confirm.is_some() || s.active.read_only || s.active.archived {
            return Err("Finish the open operation before approving.".into());
        }
        s.project_busy = true; s.rev += 1; self.changed.notify_one(); Ok(())
    }

    pub fn begin_project_change(&self) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.project_busy || s.active.archive_busy || s.active.archive.is_some() || s.confirm.is_some() || s.active.ui.turn_running() || s.parked.values().any(|c| c.ui.turn_running()) { return Err("Stop all agents and finish pending confirmations before changing projects.".into()); }
        s.project_busy = true; s.rev += 1; self.changed.notify_one(); Ok(())
    }

    /// Project selection preserves active and parked transports, including running turns.
    pub fn begin_project_navigation(&self) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.project_busy || s.active.archive_busy || s.active.archive.is_some() || s.confirm.is_some() {
            return Err("Finish the open confirmation before switching projects.".into());
        }
        s.project_busy = true;
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn end_project_change(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.project_busy = false; s.rev += 1; self.changed.notify_one();
    }
    pub fn active_channel_id(&self) -> String { self.state.lock().unwrap_or_else(|e| e.into_inner()).active.channel_id.clone() }
    pub fn active_context(&self) -> (AgentId, Option<discard_core::model::ProjectId>) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (AgentId(state.active.agent_id.clone()), state.active.project_id.clone())
    }
    pub fn active_agent_id(&self) -> String { self.state.lock().unwrap_or_else(|e| e.into_inner()).active.agent_id.clone() }
    pub fn any_running(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.active.ui.turn_running() || s.parked.values().any(|c| c.ui.turn_running())
    }
    /// Stop every running turn. Threads are loaded channels too, so this stops every sub-agent with its parent.
    pub fn cancel_all(&self) {
        let running = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            for channel in std::iter::once(&s.active).chain(s.parked.values()) { let _ = channel.commands.send(Command::Cancel); }
            std::iter::once(&s.active).chain(s.parked.values()).filter(|c| c.ui.turn_running()).map(|c| c.channel_id.clone()).collect::<Vec<_>>()
        };
        // Every loaded channel got its stop. A sub-agent that a running parent would still start must stop too.
        for parent in running { self.sub_agents.request_cancel_parent(&parent, true); }
    }
    pub fn add_channel(&self, init: CoreInit, channel: &discard_core::model::Channel) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id == channel.id.0 || s.parked.contains_key(&channel.id.0) { return; }
        s.runtime_generation += 1;
        let mut runtime = ChannelState::new(init);
        runtime.runtime_generation = s.runtime_generation;
        runtime.channel_id = channel.id.0.clone(); runtime.agent_id = channel.agent_id.0.clone(); runtime.mode = channel.workspace_mode; runtime.project_id = channel.project_id.clone();
        if let Some(marker) = startup_marker(runtime.ui.history(), s.window.last_read(&channel.id.0)) { let _ = s.window.set_last_read(&channel.id.0, &marker); }
        runtime.new_anchor = new_divider_anchor(runtime.ui.history(), s.window.last_read(&channel.id.0), None, false);
        s.parked.insert(channel.id.0.clone(), runtime);
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn can_switch(&self) -> Result<(), String> { self.can_switch_inner(false) }
    pub(crate) fn can_switch_for_project(&self) -> Result<(), String> { self.can_switch_inner(true) }
    fn can_switch_inner(&self, project_change: bool) -> Result<(), String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if (s.project_busy && !project_change) || s.active.archive_busy || s.active.archive.is_some() || s.confirm.is_some() { Err("Finish the open confirmation before switching channels.".into()) } else { Ok(()) }
    }
    pub fn switch_channel(&self, id: &str) -> Result<(), String> { self.switch_channel_inner(id, false) }
    pub(crate) fn switch_channel_for_project(&self, id: &str) -> Result<(), String> { self.switch_channel_inner(id, true) }
    fn switch_channel_inner(&self, id: &str, project_change: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id == id { return Ok(()); }
        if (s.project_busy && !project_change) || s.active.archive_busy || s.active.archive.is_some() || s.confirm.is_some() { return Err("Finish the open confirmation before switching channels.".into()); }
        if !s.parked.contains_key(id) || s.parked.get(id).is_some_and(|c| c.thread_parent.is_some()) { return Err("This channel is unavailable.".into()); }
        s.window.set_selected_conversation(Some(id))?;
        // The thread panel belongs to the channel that held its anchor.
        if s.open_thread.take().is_some() { s.thread_rev += 1; }
        let next = s.parked.remove(id).expect("channel checked under lock");
        let previous = std::mem::replace(&mut s.active, next);
        if !previous.channel_id.is_empty() { s.parked.insert(previous.channel_id.clone(), previous); }
        s.current_branch = None;
        s.rev += 1; s.history_rev += 1;
        self.branch_generation.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
        Ok(())
    }
    pub fn history_snapshot(&self) -> HistoryDto {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        HistoryDto { channel_id: s.active.channel_id.clone(), rev: s.history_rev, messages: channel_messages(&s, &s.active) }
    }
    pub fn snapshot(&self) -> SnapshotDto {
        let (mut snapshot, slot_state, context) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ui = &state.active.ui;
            let rev = state.rev;
            let history_rev = state.history_rev;
            let theme = state.theme.clone();
            let labels = SlotLabels {
                project: state.active.project_name.clone(),
                channel: state.active.channel.clone(),
                agent_id: state.active.agent_id.clone(),
                agent_name: state.active.agent.clone(),
                provider: state.active.provider.clone(),
            };
            let slot_state = ui.slot_state(&labels);
            let window = state.window.state();
            let branches = state.branches.iter().enumerate().map(|(index, row)| BranchRowDto {
                repository: state.branch_repositories.get(index).cloned().unwrap_or_default(),
                key: row.selection_key(state.branch_repositories.get(index).map(String::as_str).unwrap_or("")),
                label: row.name.clone(),
                kind: match row.kind { RowKind::Main => "main", RowKind::Worktree => "worktree", RowKind::Branch => "branch" }.into(),
                depth: row.depth(), current: state.current_branch.is_some_and(|i| std::ptr::eq(&state.branches[i], row)),
                collapsed: state.collapsed.contains(&row.name),
                detached: row.detached,
                worktree: row.worktree.as_ref().map(|path| path.to_string_lossy().into_owned()),
                has_children: state.branches.iter().enumerate().any(|(child_index, child)| state.branch_repositories.get(child_index) == state.branch_repositories.get(index) && child.parent.as_ref() == Some(&row.name)),
            }).collect();
            let mut channels: Vec<_> = std::iter::once(&state.active).chain(state.parked.values())
                    .filter(|channel| !channel.channel_id.is_empty() && channel.thread_parent.is_none()).map(|channel| ChannelDto {
                        threads: followed_links(&state, &channel.channel_id),
                        id: channel.channel_id.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.as_ref().map(|id| id.0.clone()), name: channel.channel.clone(), agent: channel.agent.clone(),
                        provider: channel.provider.clone(), workspace_mode: channel.mode.as_str().into(),
                        running: channel.ui.turn_running(), pending_approvals: channel.ui.pending_approvals(),
                        thread_pending_approvals: state.threads.values().filter(|r| r.parent == channel.channel_id).map(|r| thread_approvals(&state, &r.id)).sum(),
                        unread: super::windows::channel_indicators(channel.ui.history(), state.window.last_read(&channel.channel_id), channel.ui.pending_approvals()).0,
                    }).collect();
            channels.sort_by(|a, b| a.id.cmp(&b.id));

            let snapshot = SnapshotDto {
                rev,
                history_rev,
                memory_rev: state.memory_rev,
                project: state.project.clone(),
                project_id: state.project_id.clone(),
                project_path: state.project_path.clone(),
                project_display_path: display_project_path(&state.project_path, std::env::home_dir().as_deref()),
                active_project_id: state.active.project_id.as_ref().map(|id| id.0.clone()),
                composer_disabled: state.active.channel_id.is_empty() || state.active.read_only || state.active.archived,
                channel_id: state.active.channel_id.clone(),
                agent_id: state.active.agent_id.clone(),
                channel: state.active.channel.clone(),
                agent: state.active.agent.clone(),
                provider: state.active.provider.clone(),
                responder: ui.responder().cloned(),
                agent_status: agent_status_name(ui.agent_status()).into(),
                turn_running: ui.turn_running(),
                // The indicator describes a running response only, like the plugin slot state.
                autorun: if ui.turn_running() { AutorunDto::from_profile(&ui.session_profile) } else { None },
                notice: ui.notice().map(str::to_owned),
                live_blocks: ui.live_blocks().to_vec(),
                prompts: prompt_dtos(ui),
                pending_approvals: ui.pending_approvals(),
                unread: super::windows::channel_indicators(ui.history(), state.window.last_read(&state.active.channel_id), ui.pending_approvals()).0,
                new_anchor_id: state.active.new_anchor.clone(),
                archived: state.active.archived,
                archive_busy: state.project_busy || state.active.archive_busy,
                member_list_visible: window.member_list_visible,
                panels: PanelsDto {
                    channel_list: window.panels.channel_list,
                    member_list: window.panels.member_list,
                },
                channels,
                live_sub_agent_threads: live_sub_agent_chips(&state, &state.active.channel_id),
                thread_approvals: pending_thread_approvals(&state),
                open_thread: open_thread_dto(&state),
                branches,
                repositories: state.repositories.clone(),
                repositories_loading: state.repositories_loading,
                slots: BTreeMap::new(),
                banners: BannersDto {
                    review_count: 0,
                    review_names: vec![],
                    problem_lines: state.plugin_notice.iter().cloned().collect(),
                    confirm: state.confirm.map(|item| item.message().to_string()),
                    archive: state.active.archive.clone().map(|message| ArchiveNoticeDto { message }),
                    safe_mode: false,
                },
                theme,
            };
            (snapshot, slot_state, (AgentId(state.active.agent_id.clone()), state.active.project_id.clone()))
        };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots = BTreeMap::new();
        if let Some(plugins) = plugins.as_ref() {
            plugins.set_ui_state(slot_state.clone());
            let mut manager = plugins.manager().lock().unwrap_or_else(|e| e.into_inner());
            slots = self.slot_cache.lock().unwrap_or_else(|e| e.into_inner())
                .get_or_render(&slot_state, || { let (agent, project) = context; PluginMemory::for_context(agent, project, || super::slots::render_slots(&mut manager, &slot_state)) });
            let errors = manager.errors();
            drop(manager);
            let report = self.load_report.lock().unwrap_or_else(|e| e.into_inner());
            let pending = plugins.pending();
            snapshot.banners.review_count = pending.len();
            snapshot.banners.review_names = pending.iter().map(|review| review.name.clone()).collect();
            snapshot.banners.problem_lines.extend(super::problems::problem_lines(&errors, &plugins.statuses(), &report));
            snapshot.banners.safe_mode = report.safe_mode;
        }
        snapshot.slots = slots;
        snapshot
    }

    /// Load the saved layout and initial branch tree.
    pub fn configure_window(&self, data_dir: &Path, project_dir: &Path, channel_dir: Option<&Path>) {
        let rows = discard_core::worktree::list_branch_tree(project_dir).unwrap_or_default();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).window = StateStore::open(&data_dir.join("state.toml"));
        self.set_branches(rows, channel_dir);
    }
    pub fn set_branches(&self, rows: Vec<TreeRow>, channel_dir: Option<&Path>) {
        let current = channel_dir.and_then(|path| {
            let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            current_row(&rows, &path)
        });
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.branch_repositories = vec![String::new(); rows.len()];
        state.repositories.clear();
        state.repositories_loading = false;
        state.branches = rows;
        state.current_branch = current;
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn configure_branch_source(&self, store: Arc<Mutex<discard_core::store::Store>>, channel: discard_core::model::ChannelId, mode: discard_core::model::WorkspaceMode, project: PathBuf) {
        *self.branch_source.lock().unwrap_or_else(|e| e.into_inner()) = Some(BranchSource { store, project });
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active.channel_id = channel.0;
        state.active.mode = mode;
    }
    pub fn request_branch_refresh(&self) -> u64 { self.branch_generation.fetch_add(1, Ordering::SeqCst) + 1 }
    /// Reload repositories for the selected project, even without an active conversation.
    pub fn refresh_branches(&self, generation: u64) {
        let source = self.branch_source.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (project, channel, mode, active_project) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (PathBuf::from(&state.project_path), discard_core::model::ChannelId(state.active.channel_id.clone()), state.active.mode,
                state.active.project_id.as_ref().is_some_and(|id| id.0 == state.project_id))
        };
        if project.as_os_str().is_empty() { return; }
        let (canonical_branch, current) = source.filter(|source| active_project && source.project == project).map(|source| {
            let store = source.store.lock().unwrap_or_else(|e| e.into_inner());
            let metadata = store.project_channel_metadata(&channel).ok().flatten();
            let canonical_branch = metadata.as_ref().and_then(|m| {
                (m.kind == discard_core::model::ProjectChannelKind::Branch)
                    .then(|| m.branch.as_ref().map(|branch| (m.repository.clone(), branch.clone()))).flatten()
            });
            let legacy = metadata.as_ref().is_none_or(|metadata| metadata.kind == discard_core::model::ProjectChannelKind::Legacy);
            let worktree = legacy.then(|| store.worktree_for(&channel).ok().flatten().map(|record| PathBuf::from(record.path))).flatten();
            let current = legacy.then(|| super::windows::branch_current_directory(Some(mode), &project, worktree.as_deref())).flatten();
            (canonical_branch, current)
        }).unwrap_or((None, None));
        let discovered = discard_core::worktree::discover_repositories(&project).unwrap_or_default();
        let mut rows = Vec::new();
        let mut branch_repositories = Vec::new();
        let mut repositories = Vec::new();
        for repository in discovered {
            let repo_rows = discard_core::worktree::list_branch_tree(&repository.path).unwrap_or_default();
            branch_repositories.extend(std::iter::repeat_n(repository.id.clone(), repo_rows.len()));
            rows.extend(repo_rows);
            repositories.push(RepositoryDto {
                id: repository.id,
                label: repository.label,
                path: repository.path.to_string_lossy().into_owned(),
            });
        }
        let selected = canonical_branch.and_then(|(repository, branch)| {
            rows.iter().enumerate().position(|(index, row)| branch_repositories[index] == repository && !row.detached && row.name == branch)
        }).or_else(|| current.and_then(|path| current_row(&rows, &path.canonicalize().unwrap_or(path))));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.branch_generation.load(Ordering::SeqCst) == generation {
            state.branches = rows;
            state.branch_repositories = branch_repositories;
            state.repositories = repositories;
            state.repositories_loading = false;
            state.current_branch = selected;
            state.rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn configure_targets(&self, project_id: String, project_dir: &Path, agent_id: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.project_id = project_id;
        state.project_path = project_dir.to_string_lossy().into_owned();
        state.active.agent_id = agent_id;
        state.active.project_id = (!state.project_id.is_empty()).then(|| discard_core::model::ProjectId(state.project_id.clone()));
    }
    /// Initialize read markers for this channel, treating old history as read on first use.
    pub fn configure_read_marker(&self, data_dir: &Path, channel_id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.window = StateStore::open(&data_dir.join("state.toml"));
        s.active.channel_id = channel_id.to_owned();
        let marker = startup_marker(s.active.ui.history(), s.window.last_read(channel_id));
        if let Some(marker) = marker { let _ = s.window.set_last_read(channel_id, &marker); }
        s.active.new_anchor = new_divider_anchor(s.active.ui.history(), s.window.last_read(channel_id), None, false);
    }
    pub fn mark_read(&self, message_id: &str) -> Result<(), String> { self.mark_read_in(None, message_id) }
    pub fn mark_read_in(&self, channel: Option<&str>, message_id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(thread) = channel.filter(|id| *id != s.active.channel_id && s.parked.get(*id).is_some_and(|c| c.thread_parent.is_some())) {
            return self.mark_thread_read(&mut s, thread, message_id);
        }
        if s.active.channel_id.is_empty() || channel.is_some_and(|id| id != s.active.channel_id) { return Ok(()); }
        let Some(index) = s.active.ui.history().iter().position(|m| m.id.0 == message_id) else { return Ok(()) };
        let channel = s.active.channel_id.clone();
        if s.window.last_read(&channel).and_then(|id| s.active.ui.history().iter().position(|m| m.id.0 == id)).is_some_and(|previous| previous > index) { return Ok(()); }
        let changed = s.window.last_read(&channel) != Some(message_id);
        if changed { s.window.set_last_read(&channel, message_id)?; }
        let cleared = s.active.new_anchor.take().is_some();
        if changed || cleared { s.rev += 1; self.changed.notify_one(); }
        Ok(())
    }
    pub fn window_bounds(&self) -> Option<Bounds> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).window.state().bounds
    }
    pub fn save_bounds(&self, bounds: Bounds, screens: &[Bounds]) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        super::windows::save_restorable_bounds(&mut s.window, bounds, screens)
    }
    pub fn set_panels(&self, channel_list: f32, member_list: f32) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.window.set_panels(PanelWidths { channel_list, member_list })?;
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn set_member_list_visible(&self, visible: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.window.set_member_list_visible(visible)?;
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn toggle_branch(&self, key: String) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.collapsed.remove(&key) { s.collapsed.insert(key); }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Revert repeatedly failing plugins and refresh slots when their metadata changes.
    pub fn maintain_plugins(&self) {
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return };
        let reverted = plugins.maintain();
        let signature = {
            let manager = plugins.manager().lock().unwrap_or_else(|e| e.into_inner());
            let names = manager.names();
            format!("{:?}:{:?}:{:?}:{}", names, names.iter().map(|name| manager.info(name)).collect::<Vec<_>>(), manager.errors(), plugins.pending().len())
        };
        let mut old = self.plugin_signature.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *old != signature || !reverted.is_empty();
        *old = signature;
        if !reverted.is_empty() {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.plugin_notice = Some(format!("Reverted after repeated failures: {}", reverted.join(", ")));
        }
        if changed { self.clear_slots(); }
    }
    /// Read the current plugin notice.
    pub fn plugin_notice(&self) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).plugin_notice.clone()
    }
    /// Update the plugin notice in every window.
    pub fn set_plugin_notice(&self, notice: Option<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.plugin_notice = notice;
        state.rev += 1;
        self.changed.notify_one();
    }
    /// Record a plugin failure on the protected problem line.
    pub fn plugin_failure(&self, plugin: &str, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.plugin_notice = Some(super::plan::plugin_item_notice(plugin, error));
        state.rev += 1;
        self.changed.notify_one();
    }
    /// Run a plugin action, then discard any cached trees it could have changed.
    pub fn plugin_action(&self, plugin: &str, action: &str) -> Result<(), String> {
        let (agent, project) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); (AgentId(state.active.agent_id.clone()), state.active.project_id.clone()) };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Err("plugins are unavailable".into()) };
        let result = PluginMemory::for_context(agent, project, || plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_action(plugin, action).map_err(|e| e.to_string()));
        self.clear_slots();
        result
    }
    /// Discard the plugin trees after plugin state changes.
    pub fn clear_slots(&self) {
        self.slot_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn set_load_report(&self, report: LoadReport) {
        *self.load_report.lock().unwrap_or_else(|e| e.into_inner()) = report;
    }
    /// Show a notice in the main window.
    pub fn notice(&self, message: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active.ui.apply(Update::Notice(message));
        state.rev += 1;
        self.changed.notify_one();
    }
    /// Retain an existing recovery notice when an idle conversation cannot restart.
    pub fn append_channel_notice(&self, id: &str, message: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let channel = if state.active.channel_id == id { Some(&mut state.active) } else { state.parked.get_mut(id) };
        let Some(channel) = channel else { return };
        let message = match channel.ui.notice() {
            Some(existing) if existing.contains(&message) => return,
            Some(existing) => format!("{existing}\n\n{message}"),
            None => message,
        };
        channel.ui.apply(Update::Notice(message));
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn ask(&self, ending: super::plan::Confirm) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.confirm = Some(ending);
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn take_confirm(&self) -> Option<super::plan::Confirm> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ending = state.confirm.take();
        if ending.is_some() { state.rev += 1; self.changed.notify_one(); }
        ending
    }
    pub fn archive_started(&self) -> bool { self.archive_started_for(None) }
    pub fn archive_started_for(&self, expected_channel: Option<&str>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.channel_id.is_empty() || state.active.read_only || state.changing_agents.contains(&state.active.agent_id) || state.changing_channels.contains(&state.active.channel_id) || state.project_busy || expected_channel.is_some_and(|id| id != state.active.channel_id) || state.active.ui.turn_running() || state.active.archived || state.active.archive_busy { return false; }
        state.active.archive = None;
        state.active.archive_busy = true;
        state.rev += 1;
        self.changed.notify_one();
        true
    }
    pub fn archive_finished(&self, result: super::plan::ArchiveStep) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active.archive_busy = false;
        match result {
            super::plan::ArchiveStep::Done(message) => { state.active.archived = true; state.active.ui.apply(Update::Notice(message)); }
            super::plan::ArchiveStep::NeedsConfirm(items) => state.active.archive = Some(format!("Archiving will delete these items:\n{}", items.join("\n"))),
            super::plan::ArchiveStep::Failed(message) => state.active.ui.apply(Update::Notice(format!("Could not archive channel: {message}"))),
        }
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn cancel_archive(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.archive.take().is_some() { state.rev += 1; self.changed.notify_one(); }
    }
    pub fn has_pending_archive(&self) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).active.archive.is_some() }
    pub fn resolve_copy(&self, target: &discard_core::context_menu::ContextTarget, action: discard_core::action::Action) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        discard_core::context_menu::resolve_copy(state.active.ui.history(), &state.active.channel, target, action)
            .or_else(|| {
                let thread = state.open_thread.as_ref().and_then(|id| state.parked.get(id))?;
                discard_core::context_menu::resolve_copy(thread.ui.history(), &thread.channel, target, action)
            })
    }
    pub fn menu_context(&self) -> discard_core::action::MenuContext {
        use discard_core::action::{MenuContext, MenuTarget, PluginMenuItem};
        let (turn_running, archived) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); (state.active.ui.turn_running(), state.active.archived || state.active.read_only || state.active.channel_id.is_empty()) };
        let safe_mode = self.load_report.lock().unwrap_or_else(|e| e.into_inner()).safe_mode;
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return MenuContext { turn_running, archived, safe_mode, ..Default::default() } };
        let plugin_items = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).menu_items().into_iter().filter_map(|(plugin, index, item)| Some(PluginMenuItem { plugin, index, target: MenuTarget::parse(&item.target)?, label: item.label })).collect();
        MenuContext { turn_running, archived, safe_mode, pending_reviews: plugins.pending().len(), plugin_items }
    }
    pub fn run_plugin_menu(&self, plugin: &str, index: usize, target: Option<&discard_core::context_menu::ContextTarget>) -> Result<(), String> {
        let (agent, project) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); (AgentId(state.active.agent_id.clone()), state.active.project_id.clone()) };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Err("plugins are unavailable".into()) };
        let (kind, id) = target.map_or(("app".to_owned(), String::new()), |target| (target.kind().id().to_owned(), target.target_id()));
        let result = PluginMemory::for_context(agent, project, || plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_menu_item(plugin, index, &kind, &id).map_err(|e| e.to_string()));
        self.clear_slots();
        result
    }
    pub fn set_plugins(&self, plugins: Arc<PluginService>) {
        *self.plugins.lock().unwrap_or_else(|e| e.into_inner()) = Some(plugins);
        self.clear_slots();
    }
    pub fn set_theme(&self, theme: ThemeDto) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.theme != theme {
            s.theme = theme;
            s.rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn history(&self) -> Vec<MessageDto> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        channel_messages(&state, &state.active)
    }
    pub fn send(&self, text: String) { let _ = self.send_to(None, text); }
    pub fn send_to(&self, channel: Option<&str>, text: String) -> Result<(), String> {
        self.send_to_agent(channel, text, None)
    }
    pub fn send_to_agent(&self, channel: Option<&str>, text: String, recipient: Option<discard_core::model::MessageAuthor>) -> Result<(), String> {
        self.send_to_agent_with_attachments(channel, text, recipient, Vec::new())
    }
    pub fn send_to_agent_with_attachments(&self, channel: Option<&str>, text: String, recipient: Option<discard_core::model::MessageAuthor>, attachments: Vec<discard_core::attachments::Attachment>) -> Result<(), String> {
        if text.trim().is_empty() && attachments.is_empty() {
            return Ok(());
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if channel.is_some_and(|id| id != s.active.channel_id) { return Err("The channel changed. Send the message again.".into()); }
        if s.active.channel_id.is_empty() || s.active.read_only || s.changing_agents.contains(&s.active.agent_id) || s.changing_channels.contains(&s.active.channel_id) || s.project_busy || s.active.ui.turn_running() || s.active.archived || s.active.archive_busy || s.active.archive.is_some() {
            return Err("This channel cannot send a message right now.".into());
        }
        if recipient.as_ref().is_some_and(|agent| s.changing_agents.contains(&agent.agent_id.0)) {
            return Err("The mentioned agent is being updated. Try again.".into());
        }
        let command = if attachments.is_empty() { Command::Send(text.clone()) } else { Command::SendAttachments { text: text.clone(), attachments: attachments.clone() } };
        if s.active.commands.send(command).is_ok() {
            s.active.new_anchor = None;
            s.active.ui.user_sent_with_attachments(&text, &attachments);
            if let Some(author) = recipient { s.active.ui.apply(Update::Responding(author)); }
            s.rev += 1;
            s.history_rev += 1;
            self.changed.notify_one();
        } else { return Err("The agent session is unavailable.".into()); }
        Ok(())
    }
    pub fn continue_in(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String> {
        self.continue_with(channel, agent, note, false)
    }
    /// Like `continue_in`, after the user declined a request of the agent.
    pub fn continue_declined_in(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String> {
        self.continue_with(channel, agent, note, true)
    }
    /// Check what must hold before a decision may continue the agent of `channel`, so a decision is never saved
    /// when its follow-up cannot be sent.
    pub fn can_continue_in(&self, channel: &str) -> Result<(), String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::continue_blocked(&s, channel, None)
    }
    fn continue_blocked(s: &State, channel: &str, agent: Option<&AgentId>) -> Result<(), String> {
        if s.active.channel_id != channel { return Err("Open this channel first.".into()); }
        if s.active.ui.turn_running() { return Err("Wait for the current response to finish.".into()); }
        if s.active.read_only || s.active.archived || s.active.archive_busy || s.active.archive.is_some() || s.project_busy || s.changing_channels.contains(channel) || agent.is_some_and(|agent| s.changing_agents.contains(&agent.0)) {
            return Err("This channel cannot send a message right now.".into());
        }
        Ok(())
    }
    fn continue_with(&self, channel: &str, agent: AgentId, note: String, declined: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id != channel { return Err("The channel changed. Open the channel and try again.".into()); }
        Self::continue_blocked(&s, channel, Some(&agent))?;
        let command = if declined { Command::ContinueDeclined { agent, note } } else { Command::Continue { agent, note } };
        s.active.commands.send(command).map_err(|_| "The agent session is unavailable.")?;
        s.active.ui.response_started();
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }

    pub fn decide(&self, approval_id: String, decision: String) { self.decide_in(None, approval_id, decision, None); }
    pub fn decide_in(&self, channel: Option<&str>, approval_id: String, decision: String, rule_cut: Option<usize>) {
        let decision = match decision.as_str() {
            "approve" => Decision::Approve,
            "deny" => Decision::Deny,
            "always_allow" => Decision::AlwaysAllow,
            _ => return,
        };
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = command_target(&s, channel) else { return };
        if !target.ui.prompts().iter().any(|p| p.approval_id.to_string() == approval_id && p.view == ApprovalView::Pending) { return; }
        let _ = target.commands.send(Command::Decide {
            rule_cut,
            approval_id: ApprovalId(approval_id),
            decision,
        });
    }
    pub fn cancel(&self) { self.cancel_in(None); }
    /// Stop the turn of the selected channel or of a loaded thread. Stopping a parent channel also stops the
    /// sub-agents that its turn started. Stopping a sub-agent thread ends only that task.
    pub fn cancel_in(&self, channel: Option<&str>) {
        let parent = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(target) = command_target(&s, channel) else { return };
            let _ = target.commands.send(Command::Cancel);
            target.channel_id.clone()
        };
        self.cancel_sub_agents_of(&parent);
    }
    /// Stop every running sub-agent task of the turn of `parent`. While that turn still runs, the stop is also
    /// recorded, so a task that a `delegate` call starts later in the same turn stops too.
    pub fn cancel_sub_agents_of(&self, parent: &str) {
        let running = self.channel_running(parent);
        self.cancel_threads(self.sub_agents.request_cancel_parent(parent, running));
    }
    /// Stop every running sub-agent task of one `delegate` call.
    pub fn cancel_sub_agent_call(&self, call: u64) { self.cancel_threads(self.sub_agents.request_cancel_call(call)); }
    /// Stop the turn of one loaded thread.
    pub fn cancel_thread(&self, thread: &str) { self.cancel_threads(vec![thread.to_owned()]); }
    fn cancel_threads(&self, threads: Vec<String>) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for thread in threads { if let Some(channel) = s.parked.get(&thread).filter(|c| c.thread_parent.is_some()) { let _ = channel.commands.send(Command::Cancel); } }
    }
    /// Track a thread. Its state lives beside the parked channels and never becomes the selected channel.
    /// A thread starts with no read marker, so an agent message that nobody read counts as unread.
    pub fn add_thread(&self, init: CoreInit, channel: &discard_core::model::Channel, info: &discard_core::model::ThreadInfo) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id == channel.id.0 || s.parked.contains_key(&channel.id.0) { return; }
        s.runtime_generation += 1;
        let record = ThreadRecord::build(info, init.state.history(), init.state.authors());
        let mut runtime = ChannelState::new(init);
        runtime.runtime_generation = s.runtime_generation;
        runtime.channel_id = channel.id.0.clone(); runtime.agent_id = channel.agent_id.0.clone(); runtime.mode = channel.workspace_mode; runtime.project_id = channel.project_id.clone();
        runtime.thread_parent = Some(info.parent_channel_id.0.clone());
        s.parked.insert(channel.id.0.clone(), runtime);
        s.threads.insert(channel.id.0.clone(), record);
        if s.active.channel_id == info.parent_channel_id.0 { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Drop the runtime of a sub-agent thread whose task could not start, and stop its engine. The thread keeps its
    /// record, so its chip shows the final state. Opening the thread later loads it again from storage.
    pub fn drop_thread_runtime(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(runtime) = s.parked.get(id).filter(|c| c.thread_parent.is_some()) else { return };
        // The engine ends its turn on a stop, and its loop ends when the sender is dropped.
        let _ = runtime.commands.send(Command::Cancel);
        s.parked.remove(id);
        if s.open_thread.as_deref() == Some(id) { s.open_thread = None; s.thread_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Replace what the app knows about the threads of `parent`. A loaded thread keeps its live record.
    pub fn set_channel_threads(&self, parent: &str, records: Vec<ThreadRecord>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let st = &mut *s;
        let loaded: HashSet<String> = st.parked.iter().filter(|(_, c)| c.thread_parent.as_deref() == Some(parent)).map(|(id, _)| id.clone()).collect();
        st.threads.retain(|id, record| record.parent != parent || loaded.contains(id));
        for record in records.into_iter().filter(|record| !loaded.contains(&record.id)) { st.threads.insert(record.id.clone(), record); }
        if st.active.channel_id == parent { st.history_rev += 1; }
        st.rev += 1;
        self.changed.notify_one();
    }
    /// Set the reference rows of a loaded channel: the thread message each one shows.
    pub fn set_posted(&self, channel: &str, posted: HashMap<discard_core::model::MessageId, PostedSource>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let is_active = s.active.channel_id == channel;
        let target = if is_active { Some(&mut s.active) } else { s.parked.get_mut(channel) };
        let Some(target) = target else { return };
        target.posted = posted;
        if is_active { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn thread_parent(&self, id: &str) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).parked.get(id).and_then(|c| c.thread_parent.clone())
    }
    pub fn open_thread_id(&self) -> Option<String> { self.state.lock().unwrap_or_else(|e| e.into_inner()).open_thread.clone() }
    /// Show a loaded thread beside the selected channel. Only a thread of the selected channel can open.
    pub fn open_thread(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = s.threads.get(id) else { return Err("This thread is unavailable.".into()) };
        if record.parent != s.active.channel_id { return Err("Open this thread's channel first.".into()); }
        if !s.parked.get(id).is_some_and(|c| c.thread_parent.is_some()) { return Err("This thread is not loaded.".into()); }
        if s.open_thread.as_deref() == Some(id) { return Ok(()); }
        s.open_thread = Some(id.to_owned());
        s.thread_rev += 1; s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn close_thread(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.open_thread.take().is_some() { s.thread_rev += 1; s.rev += 1; self.changed.notify_one(); }
    }
    /// Forget a thread that never received its first message.
    pub fn remove_thread(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.parked.get(id).is_some_and(|c| c.thread_parent.is_some()) { return; }
        s.parked.remove(id);
        let parent = s.threads.remove(id).map(|record| record.parent);
        if s.open_thread.as_deref() == Some(id) { s.open_thread = None; s.thread_rev += 1; }
        if parent.is_some_and(|parent| parent == s.active.channel_id) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn set_thread_followed(&self, id: &str, followed: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = s.threads.get_mut(id) else { return };
        record.followed = followed;
        let parent = record.parent.clone();
        if s.active.channel_id == parent { s.history_rev += 1; }
        if s.open_thread.as_deref() == Some(id) { s.thread_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// The saved read marker of a channel or thread.
    pub fn last_read(&self, id: &str) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).window.last_read(id).map(str::to_owned)
    }
    /// Check what must hold before a thread row is created in `parent`.
    pub fn can_start_thread(&self, parent: &str) -> Result<(), String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.active.channel_id != parent { return Err("The channel changed. Start the thread again.".into()); }
        if s.active.read_only || s.active.archived || s.active.archive_busy || s.active.archive.is_some() || s.project_busy || s.changing_channels.contains(parent) {
            return Err("This channel cannot start a thread right now.".into());
        }
        Ok(())
    }
    /// Send a user message in a loaded thread. `also_post` also posts the saved message to the parent channel.
    pub fn send_in_thread(&self, thread: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>, also_post: bool, recipient: Option<discard_core::model::MessageAuthor>) -> Result<(), String> {
        if text.trim().is_empty() && attachments.is_empty() { return Ok(()); }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = s.parked.get(thread).filter(|c| c.thread_parent.is_some()) else { return Err("This thread is unavailable.".into()) };
        // A message in a sub-agent thread whose task runs steers the task. Any other running turn refuses it.
        let steering = s.threads.get(thread).is_some_and(|record| record.kind == ThreadKind::SubAgent && record.state == Some(ThreadState::Running)) && target.ui.turn_running();
        if thread_blocked(&s, target) || s.changing_agents.contains(&target.agent_id) || s.changing_channels.contains(thread) || s.project_busy || (target.ui.turn_running() && !steering) {
            return Err("This thread cannot send a message right now.".into());
        }
        if recipient.as_ref().is_some_and(|agent| s.changing_agents.contains(&agent.agent_id.0)) {
            return Err("The mentioned agent is being updated. Try again.".into());
        }
        // The engine refuses such a steer too. The refusal here keeps the draft in the composer.
        if steering && recipient.as_ref().is_some_and(|agent| agent.agent_id.0 != target.agent_id) {
            return Err("Wait for the sub-agent to finish before you mention another agent.".into());
        }
        let target = s.parked.get_mut(thread).expect("thread checked under lock");
        target.commands.send(Command::SendInThread { text: text.clone(), attachments: attachments.clone(), also_post }).map_err(|_| "The agent session is unavailable.")?;
        if steering {
            // The engine shows a steer after the partial response of the turn that it stops (`Update::UserMessageAdded`),
            // and follows the thread then. The running turn stays as it is until then.
            s.rev += 1;
            self.changed.notify_one();
            return Ok(());
        }
        target.ui.user_sent_with_attachments(&text, &attachments);
        if let Some(author) = recipient { target.ui.apply(Update::Responding(author)); }
        // The engine follows a thread when the user sends in it. The record follows too.
        if let Some(record) = s.threads.get_mut(thread) { record.followed = true; }
        refresh_thread(&mut s, thread);
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    /// Send the brief of a new sub-agent thread: the first and only request of its engine. The brief shows at once
    /// as a message of `author` (the delegating agent). The thread does not become followed or open.
    pub fn send_brief(&self, thread: &str, text: String, author: discard_core::model::MessageAuthor) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = s.parked.get_mut(thread).filter(|c| c.thread_parent.is_some()) else { return Err("This thread is unavailable.".into()) };
        target.commands.send(Command::SendBrief { text: text.clone(), author: author.clone() }).map_err(|_| "The agent session is unavailable.")?;
        target.ui.user_sent(&text);
        if let Some(local) = target.ui.unsaved_user().cloned() { target.ui.set_author(local, author); }
        refresh_thread(&mut s, thread);
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    /// Show a stored reference row in its parent channel. Posting again changes nothing.
    pub fn message_posted(&self, thread: &str, reference: Message, source: PostedSource) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        post_into_parent(&mut s, thread, reference, source);
        s.rev += 1;
        self.changed.notify_one();
    }
    /// The transcript of the open thread, with its anchor message.
    pub fn thread_history_snapshot(&self) -> Option<ThreadHistoryDto> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = s.open_thread.as_ref()?;
        let thread = s.parked.get(id)?;
        let record = s.threads.get(id)?;
        let anchor = channel_state(&s, &record.parent).and_then(|parent| {
            let message = parent.ui.history().iter().find(|m| m.id.0 == record.anchor)?;
            history_dto_with_authors(std::slice::from_ref(message), parent.ui.authors()).pop()
        });
        Some(ThreadHistoryDto { channel_id: id.clone(), parent_channel_id: record.parent.clone(), rev: s.thread_rev, anchor, messages: channel_messages(&s, thread) })
    }
    fn mark_thread_read(&self, s: &mut State, thread: &str, message_id: &str) -> Result<(), String> {
        let Some(channel) = s.parked.get(thread) else { return Ok(()) };
        let Some(index) = channel.ui.history().iter().position(|m| m.id.0 == message_id) else { return Ok(()) };
        let marker = s.window.last_read(thread);
        if marker.and_then(|id| channel.ui.history().iter().position(|m| m.id.0 == id)).is_some_and(|previous| previous > index) || marker == Some(message_id) { return Ok(()); }
        s.window.set_last_read(thread, message_id)?;
        // The chip and the channel bar show the thread's unread state. The parent's own state does not change.
        if s.threads.get(thread).is_some_and(|record| s.active.channel_id == record.parent) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }

    pub fn apply(&self, update: Update) -> bool { self.apply_channel(&self.active_channel_id(), update) }
    pub fn runtime_generation(&self) -> u64 { self.state.lock().unwrap_or_else(|e| e.into_inner()).active.runtime_generation }
    pub fn channel_runtime_generation(&self, id: &str) -> Option<u64> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::once(&s.active).chain(s.parked.values()).find(|c| c.channel_id == id).map(|c| c.runtime_generation)
    }
    pub fn apply_channel(&self, channel: &str, update: Update) -> bool { self.apply_runtime(channel, None, update) }
    pub fn apply_runtime(&self, channel: &str, generation: Option<u64>, update: Update) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let current = std::iter::once(&s.active).chain(s.parked.values()).find(|c| c.channel_id == channel).map(|c| c.runtime_generation);
        if generation.is_some_and(|generation| Some(generation) != current) {
            // The runtime of a thread is gone, but its task still waits for an end.
            if current.is_none() { drop(s); self.sub_agents.observe(channel, &SubAgentNote::of(&update)); }
            return false;
        }
        let memory_changed = matches!(&update, Update::MemoryChanges { .. });
        // A posted thread message is a message of the parent channel. The thread's own state has nothing to change.
        if let Update::MessagePosted { reference, source } = update {
            post_into_parent(&mut s, channel, reference, source);
            s.rev += 1;
            self.changed.notify_one();
            return false;
        }
        // The saved ID replaces the local ID of the user message, and a read marker on it follows.
        let saved = match &update {
            Update::UserMessageSaved(new) => channel_state(&s, channel).and_then(|c| c.ui.unsaved_user().cloned()).map(|old| (old, new.clone())),
            _ => None,
        };
        let thread_len = s.parked.get(channel).filter(|c| c.thread_parent.is_some()).map(|c| c.ui.history().len());
        let thread_pending = s.parked.get(channel).filter(|c| c.thread_parent.is_some()).map(|c| c.ui.pending_approvals());
        // What a sub-agent task or its parent needs from this update, read before the update moves into the state.
        let note = SubAgentNote::of(&update);
        let finished;
        if s.active.channel_id != channel {
            let marker = s.window.last_read(channel).map(str::to_owned);
            let Some(parked) = s.parked.get_mut(channel) else { drop(s); self.sub_agents.observe(channel, &note); return false };
            let was_running = parked.ui.turn_running();
            parked.ui.apply(update);
            parked.new_anchor = new_divider_anchor(parked.ui.history(), marker.as_deref(), parked.new_anchor.as_deref(), false);
            finished = was_running && !parked.ui.turn_running();
            if memory_changed { s.memory_rev += 1; }
            if !s.active.ui.turn_running() && !s.parked.values().any(|c| c.ui.turn_running()) { s.confirm = None; }
            s.rev += 1;
        } else {
            if memory_changed { s.memory_rev += 1; }
            let old_len = s.active.ui.history().len();
            let was_running = s.active.ui.turn_running();
            s.active.ui.apply(update);
            if super::windows::should_clear_confirm(s.active.ui.turn_running() || s.parked.values().any(|c| c.ui.turn_running()), s.confirm.is_some()) { s.confirm = None; }
            s.rev += 1;
            if old_len != s.active.ui.history().len() {
                s.active.new_anchor = new_divider_anchor(s.active.ui.history(), s.window.last_read(&s.active.channel_id), s.active.new_anchor.as_deref(), false);
                s.history_rev += 1;
            }
            finished = was_running && !s.active.ui.turn_running();
        }
        let id_replaced = saved.is_some();
        if let Some((old, new)) = saved {
            if s.window.last_read(channel) == Some(old.0.as_str()) { let _ = s.window.set_last_read(channel, &new.0); }
            if s.active.channel_id == channel { s.history_rev += 1; }
        }
        if let Some(before) = thread_len {
            let after = s.parked.get(channel).map(|c| c.ui.history().len());
            if after != Some(before) || id_replaced { refresh_thread(&mut s, channel); }
            // The chip shows the approvals that wait in its thread.
            else if thread_pending != s.parked.get(channel).map(|c| c.ui.pending_approvals()) && let Some(parent) = s.threads.get(channel).map(|r| r.parent.clone()) { touch_thread(&mut s, &parent, channel); }
        }
        let conversations = self.apply_thread_note(&mut s, channel, &note);
        drop(s);
        if conversations { self.conversations_rev.fetch_add(1, Ordering::SeqCst); }
        // Tell the task of a sub-agent thread, and stop the tasks of a parent whose turn ended while they ran.
        self.sub_agents.observe(channel, &note);
        if matches!(note, SubAgentNote::TurnEnded | SubAgentNote::SetupNeeded(_)) {
            self.cancel_sub_agents_of(channel);
            self.sub_agents.parent_turn_ended(channel);
        }
        self.changed.notify_one();
        finished
    }

    /// Apply what an update says about a thread record. Returns whether a list that the webview reads from storage changed.
    fn apply_thread_note(&self, s: &mut State, channel: &str, note: &SubAgentNote) -> bool {
        match note {
            SubAgentNote::Followed => {
                let Some(record) = s.threads.get_mut(channel) else { return false };
                let changed = !record.followed;
                record.followed = true;
                let parent = record.parent.clone();
                touch_thread(s, &parent, channel);
                changed
            }
            SubAgentNote::Finished { state, .. } => {
                let Some(record) = s.threads.get_mut(channel) else { return false };
                record.state = Some(*state);
                let (parent, followed) = (record.parent.clone(), record.followed);
                touch_thread(s, &parent, channel);
                followed
            }
            SubAgentNote::Anchored(infos) => {
                for info in infos {
                    match s.threads.get_mut(&info.channel_id.0) {
                        Some(record) => record.set_anchor(info),
                        // A thread that never loaded (its engine failed to start) still hangs under its response.
                        None => { s.threads.insert(info.channel_id.0.clone(), ThreadRecord::build(info, &[], &HashMap::new())); }
                    }
                    touch_thread(s, &info.parent_channel_id.0, &info.channel_id.0);
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use discard_core::model::{ChannelId, Message, MessageId, Role};

    fn approval_snapshot(profile: discard_core::ui_state::SessionProfile) -> PromptDto {
        let channel = ChannelId::new();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let mut ui = UiState::new(channel, vec![]);
        ui.apply(Update::SessionProfile(profile));
        ui.apply(Update::Event(Box::new(discard_core::event::Event {
            id: discard_core::event::EventId::new(),
            session_id: discard_core::event::SessionId::new(),
            seq: 1,
            raw: None,
            kind: discard_core::event::EventKind::ApprovalRequest {
                kind: discard_core::approval::ApprovalKind::ProviderTool,
                approval_id: ApprovalId::new(),
                call_id: "shell".into(),
                tool: "Bash".into(),
                input: serde_json::json!({"command":"cargo test -p x"}),
                paths: vec![],
            },
        })));
        AppCore::new(CoreInit { state: ui, commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() }).snapshot().prompts.remove(0)
    }
    #[test]
    fn read_only_approvals_have_no_always_allow_controls() {
        let prompt = approval_snapshot(discard_core::ui_state::SessionProfile::ReadOnly { reason: None });
        assert_eq!(prompt.always_allow, "unavailable");
        assert!(prompt.rule_words.is_none() && prompt.rule_host.is_none() && prompt.default_cut.is_none());
    }
    #[test]
    fn coding_approvals_offer_session_allow_without_prefix_controls() {
        let prompt = approval_snapshot(discard_core::ui_state::SessionProfile::Coding);
        assert_eq!(prompt.always_allow, "available");
        assert!(prompt.rule_words.is_none() && prompt.rule_host.is_none() && prompt.default_cut.is_none());
    }
    #[test]
    fn workspace_approvals_offer_prefix_controls() {
        let prompt = approval_snapshot(discard_core::ui_state::SessionProfile::Workspace { root: "/worktree".into(), autorun: discard_core::ui_state::AutorunState::Off });
        assert_eq!(prompt.always_allow, "available");
        assert_eq!(prompt.rule_words.unwrap(), ["cargo", "test", "-p", "x"]);
        assert_eq!(prompt.default_cut, Some(2));
        assert!(prompt.rule_host.is_none());
    }

    #[test]
    fn snapshot_reports_autorun_state() {
        use discard_core::ui_state::{AutorunState, SessionProfile};
        let workspace = |autorun| SessionProfile::Workspace { root: "/w".into(), autorun };
        let json = |profile: SessionProfile| serde_json::to_value(AutorunDto::from_profile(&profile)).unwrap();
        assert_eq!(json(workspace(AutorunState::On)), serde_json::json!({"state": "on", "reason": null}));
        assert_eq!(json(workspace(AutorunState::Unavailable("r".into()))), serde_json::json!({"state": "unavailable", "reason": "r"}));
        assert_eq!(AutorunDto::from_profile(&workspace(AutorunState::Off)), None);
        assert_eq!(AutorunDto::from_profile(&SessionProfile::ReadOnly { reason: None }), None);
        assert_eq!(AutorunDto::from_profile(&SessionProfile::Coding), None);
        // The snapshot carries the mapping.
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let mut ui = UiState::new(ChannelId::new(), vec![]);
        ui.apply(Update::SessionProfile(workspace(AutorunState::Unavailable("r".into()))));
        ui.response_started();
        let core = AppCore::new(CoreInit { state: ui, commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() });
        assert_eq!(serde_json::to_value(core.snapshot()).unwrap()["autorun"], serde_json::json!({"state": "unavailable", "reason": "r"}));
    }

    #[test]
    fn snapshot_has_no_autorun_after_the_turn_finishes() {
        use discard_core::ui_state::{AutorunState, SessionProfile};
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let mut ui = UiState::new(ChannelId::new(), vec![]);
        ui.apply(Update::SessionProfile(SessionProfile::Workspace { root: "/w".into(), autorun: AutorunState::On }));
        ui.response_started();
        ui.apply(Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
        assert!(!ui.turn_running());
        let core = AppCore::new(CoreInit { state: ui, commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() });
        assert!(core.snapshot().autorun.is_none());
        assert_eq!(serde_json::to_value(core.snapshot()).unwrap()["autorun"], serde_json::Value::Null);
    }

    #[test]
    fn direct_conversation_context_does_not_use_selected_sidebar_project() {
        let channel = ChannelId::new();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId(String::new()), vec![]), commands, project: "Selected sidebar project".into(), channel: "Direct".into(), agent: "A".into(), provider: "fake".into() });
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let direct = discard_core::model::Channel { id: channel.clone(), agent_id: AgentId::new(), project_id: None, name: "Direct".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_channel(CoreInit { state: UiState::new(channel.clone(), vec![]), commands, project: String::new(), channel: "Direct".into(), agent: "A".into(), provider: "fake".into() }, &direct);
        core.switch_channel(&channel.0).unwrap();
        assert_eq!(core.active_context(), (direct.agent_id, None));
    }

    #[test]
    fn marking_read_updates_snapshot_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let channel = ChannelId("channel".into());
        let history: Vec<_> = ["one", "two"].into_iter().map(|id| Message { id: MessageId(id.into()), channel_id: channel.clone(), role: Role::Agent, blocks: vec![] }).collect();
        let mut store = StateStore::open(&dir.path().join("state.toml"));
        store.set_last_read("channel", "one").unwrap();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, history), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.configure_read_marker(dir.path(), "channel");
        let before = core.snapshot();
        assert!(before.unread);
        assert_eq!(before.new_anchor_id.as_deref(), Some("two"));
        core.mark_read("two").unwrap();
        let after = core.snapshot();
        assert!(!after.unread);
        assert!(after.new_anchor_id.is_none());
        assert!(after.rev > before.rev);
    }

    #[test]
    fn marking_read_notifies_even_when_anchor_was_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let channel = ChannelId("channel".into());
        let history: Vec<_> = ["one", "two"].into_iter().map(|id| Message { id: MessageId(id.into()), channel_id: channel.clone(), role: Role::Agent, blocks: vec![] }).collect();
        let mut store = StateStore::open(&dir.path().join("state.toml"));
        store.set_last_read("channel", "one").unwrap();
        let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, history), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.configure_read_marker(dir.path(), "channel");
        core.send("hello".into());
        let before = core.snapshot();
        assert!(before.new_anchor_id.is_none());
        core.mark_read("two").unwrap();
        let after = core.snapshot();
        assert!(!after.unread);
        assert!(after.rev > before.rev);
    }

    #[test]
    fn stale_read_requests_do_not_move_the_marker_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let channel = ChannelId("channel".into());
        let history: Vec<_> = ["one", "two"].into_iter().map(|id| Message { id: MessageId(id.into()), channel_id: channel.clone(), role: Role::Agent, blocks: vec![] }).collect();
        let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, history), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.configure_read_marker(dir.path(), "channel");
        core.mark_read("two").unwrap();
        core.mark_read("one").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("two"));
        assert!(!core.snapshot().unread);
    }

    #[test]
    fn refreshed_branches_mark_the_channel_worktree_as_current() {
        use discard_core::branch_tree::{RowKind, TreeRow};
        let channel = ChannelId("channel".into());
        let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        let rows = vec![TreeRow { name: "main".into(), kind: RowKind::Main, worktree: Some("/repo".into()), detached: false, is_discard: false, parent: None }, TreeRow { name: "work".into(), kind: RowKind::Worktree, worktree: Some("/work".into()), detached: false, is_discard: true, parent: None }];
        core.set_branches(rows, Some(Path::new("/work/sub")));
        let snapshot = core.snapshot();
        assert!(!snapshot.branches[0].current);
        assert!(snapshot.branches[1].current);
    }

    #[test]
    fn project_display_paths_abbreviate_only_the_current_home_directory() {
        let home = Some(Path::new("/Users/example"));
        assert_eq!(display_project_path("/Users/example", home), "~");
        assert_eq!(display_project_path("/Users/example/git/project", home), "~/git/project");
        assert_eq!(display_project_path("/Users/example-other/project", home), "/Users/example-other/project");
        assert_eq!(display_project_path("/Users/other/project", home), "/Users/other/project");
        assert_eq!(display_project_path("/project", None), "/project");
        assert_eq!(display_project_path("", home), "");
    }

    #[test]
    fn read_marker_persists_for_current_channel() {
        let dir = tempfile::tempdir().unwrap();
        let channel = ChannelId("channel".into());
        let history = vec![Message { id: MessageId("one".into()), channel_id: channel.clone(), role: Role::Agent, blocks: vec![] }];
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, history), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.configure_read_marker(dir.path(), "channel");
        core.mark_read("one").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("one"));
        core.mark_read("unknown").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("one"));
    }
}
