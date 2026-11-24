//! State shared between engine updates and desktop commands.
use super::dto::*;
use crate::plugin_memory::PluginMemory;
use super::threads::ThreadRecord;
use super::view::ViewId;
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

/// The state of a channel a command can target: the active channel or a loaded thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandChannel { pub turn_running: bool, pub archived: bool, pub is_thread: bool }

fn channel_state<'a>(s: &'a State, id: &str) -> Option<&'a ChannelState> { s.channels.get(id) }

/// What a call for a view that does not exist returns. A call never creates a view.
pub const WINDOW_CLOSED: &str = "This window is closed.";

/// The selected channel of a view. With no channel selected, the placeholder of the view stands in. The pool
/// holds every other selected channel.
fn current_of<'a>(s: &'a State, view: &'a View) -> &'a ChannelState {
    if view.selected.is_empty() { &view.placeholder } else { s.channels.get(&view.selected).expect("the selected channel is in the pool") }
}
/// The selected channel of a view. The caller checked that the view exists.
fn current<'a>(s: &'a State, view: &ViewId) -> &'a ChannelState { current_of(s, s.views.get(view).expect("the view exists")) }
fn current_mut<'a>(s: &'a mut State, view: &ViewId) -> &'a mut ChannelState {
    let State { views, channels, .. } = s;
    let view = views.get_mut(view).expect("the view exists");
    if view.selected.is_empty() { &mut view.placeholder } else { channels.get_mut(&view.selected).expect("the selected channel is in the pool") }
}
/// Whether a view shows the channel as its selected channel.
fn is_shown(s: &State, channel: &str) -> bool { !channel.is_empty() && s.views.values().any(|v| v.selected == channel) }
/// A loaded channel that a view shows as its selected channel.
fn shown_channel<'a>(s: &'a State, channel: &str) -> Option<&'a ChannelState> { channel_state(s, channel).filter(|_| is_shown(s, channel)) }
fn shown_channel_mut<'a>(s: &'a mut State, channel: &str) -> Option<&'a mut ChannelState> {
    let shown = is_shown(s, channel);
    s.channels.get_mut(channel).filter(|_| shown)
}
/// Whether the app-wide guard or the navigation guard of a view blocks work in the view.
fn view_busy(s: &State, view: &View) -> bool { s.busy || view.navigation_busy }
/// Whether the app-wide guard or the navigation guard of a view that shows `channel` blocks work in it.
fn busy_for(s: &State, channel: &str) -> bool { s.busy || s.views.values().any(|v| v.navigation_busy && v.selected == channel) }
/// Whether a view shows the thread beside its selected channel.
fn thread_open(s: &State, thread: &str) -> bool { s.views.values().any(|v| v.open_thread.as_deref() == Some(thread)) }
/// Close the thread panel of every view that shows `thread`.
fn close_open_thread(s: &mut State, thread: &str) {
    let mut closed = false;
    for view in s.views.values_mut().filter(|v| v.open_thread.as_deref() == Some(thread)) { view.open_thread = None; closed = true; }
    if closed { s.thread_rev += 1; }
}
/// Close the thread panel of every view whose thread left the pool.
fn close_missing_threads(s: &mut State) {
    let State { views, channels, thread_rev, .. } = s;
    for view in views.values_mut().filter(|v| v.open_thread.as_ref().is_some_and(|thread| !channels.contains_key(thread))) { view.open_thread = None; *thread_rev += 1; }
}
/// An empty channel for a view that selects no channel.
fn empty_channel() -> ChannelState {
    let (commands, _) = tokio::sync::mpsc::unbounded_channel();
    ChannelState::new(CoreInit { state: UiState::new(discard_core::model::ChannelId(String::new()), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() })
}
/// Give the selected channel of a view a new ID. Its key in the pool follows the ID, and so does the selection of
/// every view that shows it. A view with no channel selected moves its placeholder into the pool.
fn set_selected_id(s: &mut State, view: &ViewId, id: &str) {
    let old = s.views.get(view).expect("the view exists").selected.clone();
    let mut channel = if old.is_empty() { std::mem::replace(&mut s.views.get_mut(view).expect("the view exists").placeholder, empty_channel()) } else { s.channels.remove(&old).expect("the selected channel is in the pool") };
    debug_assert!(!s.channels.contains_key(id), "a channel ID is in the pool once");
    channel.channel_id = id.to_owned();
    s.channels.insert(id.to_owned(), channel);
    for other in s.views.values_mut().filter(|v| !old.is_empty() && v.selected == old) { other.selected = id.to_owned(); }
    s.views.get_mut(view).expect("the view exists").selected = id.to_owned();
}
/// Copy the saved layout of the main window to the main view.
fn sync_main_layout(s: &mut State) {
    let layout = s.window.state();
    if let Some(main) = s.views.get_mut(&ViewId::main()) { main.panels = layout.panels; main.member_list_visible = layout.member_list_visible; }
}
/// Panel widths in the range that the saved state allows.
fn clamped_panels(panels: PanelWidths) -> PanelWidths {
    let clamp = |width: f32| if width.is_finite() { width.clamp(discard_core::window_state::MIN_PANEL_WIDTH, discard_core::window_state::MAX_PANEL_WIDTH) } else { discard_core::window_state::DEFAULT_PANEL_WIDTH };
    PanelWidths { channel_list: clamp(panels.channel_list), member_list: clamp(panels.member_list) }
}

/// What a window of a view that does not exist reads in its history: no channel and no messages.
fn closed_history(s: &State) -> HistoryDto { HistoryDto { channel_id: String::new(), rev: s.history_rev, messages: vec![] } }

/// What a window of a view that does not exist reads: no project, no channel, and the theme of the app. It builds
/// no view, no channel list, and no plugin slots.
fn closed_snapshot(s: &State) -> SnapshotDto {
    let panels = PanelWidths::default();
    SnapshotDto {
        active_project_id: None, composer_disabled: true, rev: s.rev, history_rev: s.history_rev, memory_rev: s.memory_rev,
        project: String::new(), project_id: String::new(), project_path: String::new(), project_display_path: String::new(),
        channel_id: String::new(), agent_id: String::new(), channel: String::new(), agent: String::new(), provider: String::new(),
        responder: None, agent_status: agent_status_name(AgentStatus::Idle).into(), turn_running: false, autorun: None, resume_state: None, notice: None,
        live_blocks: vec![], prompts: vec![], pending_approvals: 0, unread: false, new_anchor_id: None, archived: false, archive_busy: false,
        member_list_visible: true, panels: PanelsDto { channel_list: panels.channel_list, member_list: panels.member_list },
        branches: vec![], repositories: vec![], repositories_loading: false, channels: vec![], live_sub_agent_threads: vec![],
        channel_threads: vec![], thread_approvals: vec![], open_thread: None, slots: BTreeMap::new(),
        banners: BannersDto { review_count: 0, review_names: vec![], problem_lines: vec![], confirm: None, archive: None, safe_mode: false },
        theme: s.theme.clone(),
    }
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
fn thread_approvals(s: &State, thread: &str) -> usize { s.channels.get(thread).map_or(0, |c| c.ui.pending_approvals()) }

/// The followed threads of a channel with live state, oldest first.
fn followed_links(s: &State, channel: &str) -> Vec<ThreadLinkDto> {
    s.threads.values().filter(|r| r.parent == channel && r.followed && r.visible()).map(|r| {
        let live = s.channels.get(&r.id);
        r.link(s.window.last_read(&r.id), live.is_some_and(|c| c.ui.turn_running()), live.map_or(0, |c| c.ui.pending_approvals()))
    }).collect()
}

/// The sub-agent threads of the running turn of `channel`: they have no anchor yet.
fn live_sub_agent_chips(s: &State, channel: &str) -> Vec<ThreadChipDto> {
    s.threads.values().filter(|r| r.parent == channel && r.kind == ThreadKind::SubAgent && r.anchor.is_empty())
        .map(|r| r.chip(s.window.last_read(&r.id), thread_approvals(s, &r.id))).collect()
}

/// Every thread of `channel` that shows anywhere, followed or not, latest activity first.
fn channel_thread_list(s: &State, channel: &str) -> Vec<ThreadChipDto> {
    let mut records: Vec<&ThreadRecord> = s.threads.values().filter(|r| r.parent == channel && r.visible()).collect();
    records.sort_by(|a, b| b.activity().cmp(a.activity()).then_with(|| b.id.cmp(&a.id)));
    records.into_iter().map(|r| r.chip(s.window.last_read(&r.id), thread_approvals(s, &r.id))).collect()
}

/// Whether a view lists the channel of `project` in its snapshot. The main view lists every project. A project view
/// lists its own project only (decision 21a).
fn lists_project(view: &ViewId, view_project: &str, project: Option<&discard_core::model::ProjectId>) -> bool {
    view.is_main() || project.is_some_and(|project| project.0 == view_project)
}

/// Every loaded thread of a channel that `listed` accepts with an approval that waits, followed or not.
fn pending_thread_approvals(s: &State, listed: impl Fn(Option<&discard_core::model::ProjectId>) -> bool) -> Vec<ThreadApprovalDto> {
    s.threads.values().filter(|r| listed(channel_state(s, &r.parent).or_else(|| channel_state(s, &r.id)).and_then(|c| c.project_id.as_ref()))).filter_map(|r| {
        let pending = thread_approvals(s, &r.id);
        (pending > 0).then(|| ThreadApprovalDto { thread_id: r.id.clone(), parent_channel_id: r.parent.clone(), name: r.name.clone(), kind: r.kind.as_str().into(), agent_name: r.agent_name.clone(), pending_approvals: pending })
    }).collect()
}

/// A thread is read-only when it is, or its parent channel is, read-only or archived.
fn thread_blocked(s: &State, thread: &ChannelState) -> bool {
    thread.read_only || thread.archived || thread.thread_parent.as_deref().and_then(|parent| channel_state(s, parent)).is_some_and(|parent| parent.archived)
}

fn open_thread_dto(s: &State, view: &View) -> Option<OpenThreadDto> {
    let id = view.open_thread.as_ref()?;
    let channel = s.channels.get(id)?;
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
    let Some(channel) = s.channels.get(thread) else { return };
    let Some(record) = s.threads.get_mut(thread) else { return };
    record.refresh(channel.ui.history(), channel.ui.authors());
    let parent = record.parent.clone();
    touch_thread(s, &parent, thread);
}

/// Tell the views that show a thread that its record changed: the parent transcript with the chip, the open panel.
fn touch_thread(s: &mut State, parent: &str, thread: &str) {
    if thread_open(s, thread) { s.thread_rev += 1; }
    if is_shown(s, parent) { s.history_rev += 1; }
}

/// Add a thread message to its parent channel as a reference row, once. Returns whether the parent changed.
fn post_into_parent(s: &mut State, thread: &str, reference: Message, source: PostedSource) -> bool {
    let Some(parent_id) = s.channels.get(thread).and_then(|t| t.thread_parent.clone()) else { return false };
    let author = s.channels.get(thread).and_then(|t| t.ui.authors().get(&source.source_message_id).cloned());
    let marker = s.window.last_read(&parent_id).map(str::to_owned);
    let is_active = is_shown(s, &parent_id);
    let Some(parent) = s.channels.get_mut(&parent_id) else { return false };
    parent.posted.insert(reference.id.clone(), source);
    let added = !parent.ui.history().iter().any(|m| m.id == reference.id);
    if added {
        if let Some(author) = author { parent.ui.set_author(reference.id.clone(), author); }
        parent.ui.apply(Update::ConversationMarker(reference));
        parent.new_anchor = new_divider_anchor(parent.ui.history(), marker.as_deref(), parent.new_anchor.as_deref(), false);
    }
    if is_active { s.history_rev += 1; }
    if thread_open(s, thread) { s.thread_rev += 1; }
    added
}

/// The conversation a command for `channel` acts on: a channel that a view shows, or a loaded thread.
fn command_target<'a>(s: &'a State, channel: &str) -> Option<&'a ChannelState> {
    channel_state(s, channel).filter(|c| c.thread_parent.is_some() || is_shown(s, channel))
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
    /// A muted channel shows no unread marker. Approval badges stay.
    muted: bool,
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
            archive: None, archived: false, archive_busy: false, read_only: false, muted: false, thread_parent: None, posted: HashMap::new() }
    }
}
/// The selection of one app window. The views share the channel pool.
struct View {
    project: String,
    project_id: String,
    project_path: String,
    /// The ID of the selected channel. Empty when no channel is selected.
    selected: String,
    /// The empty channel that this view shows while it selects no channel. A write to it shows in this view only.
    placeholder: ChannelState,
    /// The thread shown beside the selected channel.
    open_thread: Option<String>,
    branches: Vec<TreeRow>,
    branch_repositories: Vec<String>,
    repositories: Vec<RepositoryDto>,
    repositories_loading: bool,
    current_branch: Option<usize>,
    collapsed: HashSet<String>,
    branch_source: Option<BranchSource>,
    branch_generation: u64,
    /// Background branch refreshes of this view that have not finished.
    branch_refreshes_pending: usize,
    /// A project navigation of this view runs. It blocks the selection of this view only.
    navigation_busy: bool,
    panels: PanelWidths,
    member_list_visible: bool,
    /// The plugin slots that this view rendered last. A shared handle: rendering runs without the state lock.
    slots: Arc<Mutex<super::slots::SlotCache>>,
}
impl View {
    fn new(project: String, placeholder: ChannelState) -> Self {
        Self {
            project, project_id: String::new(), project_path: String::new(), selected: String::new(), placeholder, open_thread: None,
            branches: vec![], branch_repositories: vec![], repositories: vec![], repositories_loading: true, current_branch: None, collapsed: HashSet::new(),
            branch_source: None, branch_generation: 0, branch_refreshes_pending: 0, navigation_busy: false,
            panels: PanelWidths::default(), member_list_visible: true, slots: Arc::new(Mutex::new(super::slots::SlotCache::default())),
        }
    }
}
/// The focused view, or the main view when the focused view is gone.
fn focused_view_of(state: &State) -> ViewId {
    if state.views.contains_key(&state.focused) { state.focused.clone() } else { ViewId::main() }
}
struct State {
    /// The app-wide guard of changes that touch shared data: a project change, a write approval.
    busy: bool,
    changing_agents: HashSet<String>,
    changing_channels: HashSet<String>,
    runtime_generation: u64,
    /// Every loaded channel and thread, by channel ID. The key is never empty: a view that selects no channel holds
    /// its own placeholder.
    channels: BTreeMap<String, ChannelState>,
    /// The views by window label.
    views: BTreeMap<ViewId, View>,
    /// The view of the app window that had focus last. Read it through `focused_view_of`.
    focused: ViewId,
    rev: u64,
    history_rev: u64,
    memory_rev: u64,
    /// Every thread the app knows, by thread channel ID. Loaded threads keep their record current.
    threads: BTreeMap<String, ThreadRecord>,
    thread_rev: u64,
    theme: ThemeDto,
    window: StateStore,
    confirm: Option<super::plan::Confirm>,
    plugin_notice: Option<String>,
}
/// The canonical (repository, branch) of the selected channel, and its working directory for a legacy channel.
type BranchHint = (Option<(String, String)>, Option<PathBuf>);
#[derive(Clone)]
struct BranchSource {
    store: Arc<Mutex<discard_core::store::Store>>,
    project: PathBuf,
}
pub struct AppCore {
    state: Mutex<State>,
    plugins: Mutex<Option<Arc<PluginService>>>,
    load_report: Mutex<LoadReport>,
    plugin_signature: Mutex<String>,
    /// Moves when a thread list that the webview reads from storage changed without a command of the webview.
    conversations_rev: AtomicU64,
    sub_agents: super::delegation::SubAgents,
    /// Focus, seen approvals, and the notifier. Never locked together with `state`.
    notify: Mutex<NotifyState>,
    pub(crate) changed: Notify,
}
type EnabledFn = Arc<dyn Fn() -> bool + Send + Sync>;
#[derive(Default)]
struct NotifyState {
    focus: super::notify::FocusTracker,
    seen: super::notify::Seen,
    sink: Option<(Arc<dyn super::notify::Notifier>, EnabledFn)>,
}
impl AppCore {
    pub fn new(init: CoreInit) -> Self {
        let first = ChannelState::new(init);
        let selected = first.channel_id.clone();
        let mut main = View::new(first.project_name.clone(), empty_channel());
        let mut channels = BTreeMap::new();
        // A first channel with no ID is the placeholder of the main view.
        if selected.is_empty() { main.placeholder = first; } else { main.selected = selected.clone(); channels.insert(selected, first); }
        Self {
            state: Mutex::new(State {
                busy: false,
                changing_agents: HashSet::new(),
                changing_channels: HashSet::new(),
                runtime_generation: 0,
                channels,
                views: BTreeMap::from([(ViewId::main(), main)]),
                focused: ViewId::main(),
                window: StateStore::open(Path::new("")),
                confirm: None,
                plugin_notice: None,
                rev: 0,
                history_rev: 0,
                memory_rev: 0,
                threads: BTreeMap::new(),
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
                    terminal: Default::default(),
                },
            }),
            plugins: Mutex::new(None),
            load_report: Mutex::new(LoadReport::default()),
            plugin_signature: Mutex::new(String::new()),
            conversations_rev: AtomicU64::new(0),
            sub_agents: super::delegation::SubAgents::default(),
            notify: Mutex::new(NotifyState::default()),
            changed: Notify::new(),
        }
    }
    /// The running sub-agent tasks.
    pub fn sub_agents(&self) -> &super::delegation::SubAgents { &self.sub_agents }
    /// Moves when the engine followed a thread or a followed sub-agent thread changed state. The pump tells the
    /// webview (`discard://conversations`) so lists that it reads from storage load again.
    pub fn conversations_rev(&self) -> u64 { self.conversations_rev.load(Ordering::SeqCst) }
    /// The labels of the views, in order.
    pub fn view_ids(&self) -> Vec<ViewId> { self.state.lock().unwrap_or_else(|e| e.into_inner()).views.keys().cloned().collect() }
    pub fn has_view(&self, view: &ViewId) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).views.contains_key(view) }
    /// Add a view fixed to `project`, with the saved panel widths and member list visibility. It selects no channel
    /// yet. Returns false and changes nothing when the view exists.
    pub fn open_view(&self, view: ViewId, project: &discard_core::model::Project) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.views.contains_key(&view) { return false; }
        let layout = s.window.state();
        let mut opened = View::new(project.name.clone(), empty_channel());
        opened.project_id = project.id.0.clone();
        opened.project_path = project.path.clone();
        opened.panels = layout.panels;
        opened.member_list_visible = layout.member_list_visible;
        s.views.insert(view, opened);
        s.rev += 1;
        self.changed.notify_one();
        true
    }
    /// Remove a view. It stops no turn and unloads no channel: the pool keeps every channel, and a turn goes on. The
    /// guards of the view go with it, and the app-wide guard is not touched. The main view stays.
    pub fn drop_view(&self, view: &ViewId) {
        if view.is_main() { return; }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.views.remove(view).is_some() {
            // The main view takes the focus back. The system focuses another window and sends its focus event.
            if s.focused == *view { s.focused = ViewId::main(); }
            s.rev += 1;
            self.changed.notify_one();
        }
    }
    /// The views that select `channel`, in order. No view selects the empty ID.
    pub fn views_selecting(&self, channel: &str) -> Vec<ViewId> {
        if channel.is_empty() { return vec![]; }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).views.iter().filter(|(_, v)| v.selected == channel).map(|(view, _)| view.clone()).collect()
    }
    /// The view that carries out a selection of `target_project` for `view`. The main view selects anything. A project
    /// view is fixed to its project: a target in another project, and a target with no project (a direct message),
    /// go to the main view (decisions 17 and 17b). The caller checks that `view` exists.
    pub fn route(&self, view: &ViewId, target_project: Option<&discard_core::model::ProjectId>) -> ViewId {
        if view.is_main() || view.project_id().as_ref() == target_project { view.clone() } else { ViewId::main() }
    }
    /// The view of the window that the user used last, while that view exists. The main view otherwise.
    pub fn focused_view(&self) -> ViewId { focused_view_of(&self.state.lock().unwrap_or_else(|e| e.into_inner())) }
    /// Record that the window of `view` took focus. The menu and the quit confirmation follow it, so the pump wakes. A
    /// view that does not exist takes no focus.
    pub fn set_focused_view(&self, view: &ViewId) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.views.contains_key(view) || s.focused == *view { return; }
        s.focused = view.clone();
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn clear_project(&self, view: &ViewId) {
        if !self.has_view(view) { return; }
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let project = discard_core::model::Project { id: discard_core::model::ProjectId(String::new()), name: String::new(), path: String::new() };
        let channel = discard_core::model::Channel { id: discard_core::model::ChannelId(String::new()), project_id: None, name: String::new(), agent_id: AgentId(String::new()), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        self.replace_project(view, CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: String::new(), channel: String::new(), agent: String::new(), provider: String::new() }, &project, &channel);
        if let Some(target) = self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get_mut(view) { target.branch_source = None; }
        self.clear_slots();
    }
    /// Browse projects independently of the active conversation.
    pub fn select_sidebar_project(&self, view: &ViewId, project: Option<&discard_core::model::Project>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get_mut(view) else { return };
        // The same project keeps its rows. A refresh replaces them in one step.
        let same = project.is_some_and(|p| p.id.0 == v.project_id && p.path == v.project_path);
        v.project = project.map(|p| p.name.clone()).unwrap_or_default();
        v.project_id = project.map(|p| p.id.0.clone()).unwrap_or_default();
        v.project_path = project.map(|p| p.path.clone()).unwrap_or_default();
        if !same {
            v.branches.clear(); v.branch_repositories.clear(); v.repositories.clear(); v.repositories_loading = project.is_some();
            v.current_branch = None;
            v.collapsed.clear();
        }
        v.branch_generation += 1;
        s.rev += 1;
        self.changed.notify_one();
    }

    /// Show the new name of a project in every view that shows it.
    pub fn update_project_name(&self, project_id: &str, name: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = false;
        for v in s.views.values_mut().filter(|v| v.project_id == project_id) { v.project = name.to_owned(); changed = true; }
        if changed {
            s.rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn saved_conversation(&self) -> Option<String> { self.state.lock().unwrap_or_else(|e| e.into_inner()).window.state().selected_conversation }
    /// Whether the channel has a turn that runs.
    pub fn channel_running(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.get(id).is_some_and(|c| c.ui.turn_running())
    }
    pub fn has_channel(&self, id: &str) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).channels.contains_key(id) }
    /// The channels and threads that the pool holds. Loaded thread channels count. The placeholder of a view that
    /// selects no channel does not.
    pub fn loaded_channel_count(&self) -> usize { self.state.lock().unwrap_or_else(|e| e.into_inner()).channels.len() }
    pub fn agent_channel_ids(&self, id: &str) -> Vec<String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<String> = s.channels.values().filter(|c| c.agent_id == id).map(|c| c.channel_id.clone()).collect();
        // The channels that a view shows come first, then the others by ID.
        ids.sort_by_key(|channel| !is_shown(&s, channel));
        ids
    }
    pub fn channel_session_closed(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.get(id).is_some_and(|c| c.commands.is_closed())
    }
    /// Replace only a closed transport. Preserve transcript, notices and unsaved reply blocks.
    pub fn restart_closed_channel(&self, init: CoreInit, channel: &discard_core::model::Channel) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next_generation = s.runtime_generation + 1;
        let Some(target) = s.channels.get_mut(&channel.id.0) else { return Ok(false) };
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
        let channel = s.channels.get(id);
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
        let Some(target) = s.channels.get_mut(&channel.id.0) else { return Ok(false) };
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
        if !s.busy { return Err("Begin the project change before reloading its sessions.".into()); }
        if s.channels.values().any(|c| c.project_id.as_ref() == Some(id) && (c.ui.turn_running() || c.archive_busy)) {
            return Err("Stop this project's responses before restarting its sessions.".into());
        }
        let State { channels, runtime_generation, .. } = &mut *s;
        for channel in channels.values_mut().filter(|c| c.project_id.as_ref() == Some(id)) {
            *runtime_generation += 1;
            channel.runtime_generation = *runtime_generation;
            let (commands, _) = tokio::sync::mpsc::unbounded_channel();
            channel.commands = commands;
        }
        s.rev += 1; self.changed.notify_one(); Ok(())
    }
    pub fn agent_sessions_closed(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.values().any(|c| c.agent_id == id && c.commands.is_closed())
    }
    /// Whether the agent has a turn that runs in any loaded channel, as the channel agent or as the responder.
    pub fn agent_running(&self, id: &str) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.values().any(|c| (c.agent_id == id || c.ui.responder().is_some_and(|author| author.agent_id.0 == id)) && c.ui.turn_running())
    }
    pub fn begin_agent_change(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.changing_agents.contains(id) || s.channels.values().any(|c| (c.agent_id == id || c.ui.responder().is_some_and(|author| author.agent_id.0 == id)) && (c.ui.turn_running() || c.archive_busy)) {
            return Err("Stop this agent's responses before changing its lifecycle.".into());
        }
        s.changing_agents.insert(id.to_owned()); Ok(())
    }
    pub fn end_agent_change(&self, id: &str) { self.state.lock().unwrap_or_else(|e| e.into_inner()).changing_agents.remove(id); }
    pub fn refresh_agent(&self, id: &str, config: &discard_core::agent::AgentConfig, read_only: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for channel in s.channels.values_mut().filter(|c| c.agent_id == id) {
            channel.agent = config.name.clone(); channel.provider = config.provider.clone(); channel.read_only = read_only;
        }
        s.rev += 1; drop(s); self.clear_slots(); self.changed.notify_one();
    }
    pub fn channel_read_only(&self, id: &str, read_only: bool, archived: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(channel) = s.channels.get_mut(id) { channel.read_only = read_only; channel.archived = archived; }
        s.rev += 1; self.changed.notify_one();
    }
    pub fn remove_agent_channels(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let removed: HashSet<String> = s.channels.iter().filter(|(_, c)| c.agent_id == id).map(|(channel, _)| channel.clone()).collect();
        let affected: Vec<ViewId> = s.views.iter().filter(|(view, _)| current(&s, view).agent_id == id).map(|(view, _)| view.clone()).collect();
        s.channels.retain(|_, c| c.agent_id != id);
        {
            let st = &mut *s;
            st.threads.retain(|thread, record| !removed.contains(thread) && !removed.contains(&record.parent));
            st.channels.retain(|thread, c| c.thread_parent.is_none() || st.threads.contains_key(thread));
        }
        close_missing_threads(&mut s);
        for view in &affected {
            let target = s.views.get_mut(view).expect("the view exists");
            target.selected = String::new();
            target.placeholder = empty_channel();
        }
        if !affected.is_empty() {
            if affected.iter().any(ViewId::is_main) { let _ = s.window.set_selected_conversation(None); }
            s.history_rev += 1;
        }
        s.rev += 1; drop(s);
        for view in &affected { self.clear_branch_source(view); }
        self.changed.notify_one();
    }
    pub fn clear_branch_source(&self, view: &ViewId) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(v) = s.views.get_mut(view) else { return };
            v.branch_source = None;
            v.branch_generation += 1;
        }
        self.set_branches(view, vec![], None);
    }
    pub fn current_project(&self, view: &ViewId) -> discard_core::model::Project {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (id, name, path) = s.views.get(view).map(|v| (v.project_id.clone(), v.project.clone(), v.project_path.clone())).unwrap_or_default();
        discard_core::model::Project { id: discard_core::model::ProjectId(id), name, path }
    }
    /// Show `project` and `channel` in a view. The channels that other views show stay in the pool. Returns false
    /// when `init` was not installed: the view is gone, or another view already loaded the channel. The caller must
    /// not launch a session before it knows the channel is not loaded, because a dropped `init` ends its engine.
    pub fn replace_project(&self, view: &ViewId, init: CoreInit, project: &discard_core::model::Project, channel: &discard_core::model::Channel) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.views.contains_key(view) { return false; }
        s.runtime_generation += 1;
        let mut next = ChannelState::new(init);
        next.runtime_generation = s.runtime_generation;
        next.channel_id = channel.id.0.clone(); next.agent_id = channel.agent_id.0.clone(); next.mode = channel.workspace_mode; next.project_id = channel.project_id.clone();
        let kept: HashSet<String> = s.views.iter().filter(|(other, _)| *other != view).map(|(_, v)| v.selected.clone()).collect();
        s.channels.retain(|id, c| kept.contains(id) || c.thread_parent.as_ref().is_some_and(|parent| kept.contains(parent)));
        s.threads.retain(|_, record| kept.contains(&record.parent));
        {
            let st = &mut *s;
            let target = st.views.get_mut(view).expect("the view exists");
            target.project = project.name.clone(); target.project_id = project.id.0.clone(); target.project_path = project.path.clone();
            target.selected = channel.id.0.clone();
            target.branches.clear(); target.branch_repositories.clear(); target.repositories.clear(); target.repositories_loading = false; target.collapsed.clear(); target.current_branch = None;
            target.branch_generation += 1;
            if target.open_thread.take().is_some() { st.thread_rev += 1; }
        }
        // A channel that another view shows keeps its loaded state. The new runtime is dropped, so its engine ends.
        let installed = channel.id.0.is_empty() || !s.channels.contains_key(&channel.id.0);
        if channel.id.0.is_empty() { s.views.get_mut(view).expect("the view exists").placeholder = next; } else if installed { s.channels.insert(channel.id.0.clone(), next); }
        close_missing_threads(&mut s);
        if let Some(marker) = startup_marker(current(&s, view).ui.history(), s.window.last_read(&channel.id.0)) { let _ = s.window.set_last_read(&channel.id.0, &marker); }
        let anchor = new_divider_anchor(current(&s, view).ui.history(), s.window.last_read(&channel.id.0), None, false);
        current_mut(&mut s, view).new_anchor = anchor;
        s.rev += 1; s.history_rev += 1;
        self.changed.notify_one();
        installed
    }
    /// Take the app-wide guard for a write approval in a channel that a view shows.
    pub fn begin_write_approval(&self, channel: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(active) = shown_channel(&s, channel) else { return Err("Open this channel before approving.".into()) };
        if active.ui.turn_running() { return Err("Wait for the current response to finish.".into()); }
        if busy_for(&s, channel) || active.archive_busy || active.archive.is_some() || s.confirm.is_some() || active.read_only || active.archived {
            return Err("Finish the open operation before approving.".into());
        }
        s.busy = true; s.rev += 1; self.changed.notify_one(); Ok(())
    }

    /// Whether a project change holds the app-wide guard.
    pub fn project_change_running(&self) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).busy }
    /// Take the app-wide guard. Refused while any view navigates, because a project change reloads shared sessions.
    pub fn begin_project_change(&self) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.busy || s.views.values().any(|v| v.navigation_busy) || s.channels.values().any(|c| c.archive_busy || c.archive.is_some()) || s.confirm.is_some() || s.channels.values().any(|c| c.ui.turn_running()) { return Err("Stop all agents and finish pending confirmations before changing projects.".into()); }
        s.busy = true; s.rev += 1; self.changed.notify_one(); Ok(())
    }

    /// Project selection preserves the transports of every loaded channel, including running turns. It takes the
    /// navigation guard of one view, so the other views stay free.
    pub fn begin_project_navigation(&self, view: &ViewId) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get(view) else { return Err(WINDOW_CLOSED.into()) };
        let active = current_of(&s, v);
        if view_busy(&s, v) || active.archive_busy || active.archive.is_some() || s.confirm.is_some() {
            return Err("Finish the open confirmation before switching projects.".into());
        }
        s.views.get_mut(view).expect("the view exists").navigation_busy = true;
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn end_project_navigation(&self, view: &ViewId) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get_mut(view) else { return };
        v.navigation_busy = false;
        s.rev += 1; self.changed.notify_one();
    }
    pub fn end_project_change(&self) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.busy = false; s.rev += 1; self.changed.notify_one();
    }
    /// The ID of the channel that a view selects. `None` when the view does not exist.
    fn selected_of(&self, view: &ViewId) -> Option<String> { self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get(view).map(|v| v.selected.clone()) }
    pub fn active_channel_id(&self, view: &ViewId) -> String { self.selected_of(view).unwrap_or_default() }
    pub fn active_context(&self, view: &ViewId) -> (AgentId, Option<discard_core::model::ProjectId>) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get(view) else { return (AgentId(String::new()), None) };
        let active = current_of(&state, v);
        (AgentId(active.agent_id.clone()), active.project_id.clone())
    }
    pub fn active_agent_id(&self, view: &ViewId) -> String { self.active_context(view).0.0 }
    pub fn any_running(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.values().any(|c| c.ui.turn_running())
    }
    /// Stop every running turn. Threads are loaded channels too, so this stops every sub-agent with its parent.
    pub fn cancel_all(&self) {
        let running = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            for channel in s.channels.values() { let _ = channel.commands.send(Command::Cancel); }
            s.channels.values().filter(|c| c.ui.turn_running()).map(|c| c.channel_id.clone()).collect::<Vec<_>>()
        };
        // Every loaded channel got its stop. A sub-agent that a running parent would still start must stop too.
        for parent in running { self.sub_agents.request_cancel_parent(&parent, true); }
    }
    pub fn add_channel(&self, init: CoreInit, channel: &discard_core::model::Channel) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.channels.contains_key(&channel.id.0) { return; }
        s.runtime_generation += 1;
        let mut runtime = ChannelState::new(init);
        runtime.runtime_generation = s.runtime_generation;
        runtime.channel_id = channel.id.0.clone(); runtime.agent_id = channel.agent_id.0.clone(); runtime.mode = channel.workspace_mode; runtime.project_id = channel.project_id.clone();
        if let Some(marker) = startup_marker(runtime.ui.history(), s.window.last_read(&channel.id.0)) { let _ = s.window.set_last_read(&channel.id.0, &marker); }
        runtime.new_anchor = new_divider_anchor(runtime.ui.history(), s.window.last_read(&channel.id.0), None, false);
        s.channels.insert(channel.id.0.clone(), runtime);
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn can_switch(&self, view: &ViewId) -> Result<(), String> { self.can_switch_inner(view, false) }
    pub(crate) fn can_switch_for_project(&self, view: &ViewId) -> Result<(), String> { self.can_switch_inner(view, true) }
    fn can_switch_inner(&self, view: &ViewId, project_change: bool) -> Result<(), String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get(view) else { return Err(WINDOW_CLOSED.into()) };
        let active = current_of(&s, v);
        if (view_busy(&s, v) && !project_change) || active.archive_busy || active.archive.is_some() || s.confirm.is_some() { Err("Finish the open confirmation before switching channels.".into()) } else { Ok(()) }
    }
    pub fn switch_channel(&self, view: &ViewId, id: &str) -> Result<(), String> { self.switch_channel_inner(view, id, false) }
    pub(crate) fn switch_channel_for_project(&self, view: &ViewId, id: &str) -> Result<(), String> { self.switch_channel_inner(view, id, true) }
    fn switch_channel_inner(&self, view: &ViewId, id: &str, project_change: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get(view) else { return Err(WINDOW_CLOSED.into()) };
        if v.selected == id { return Ok(()); }
        let active = current_of(&s, v);
        if (view_busy(&s, v) && !project_change) || active.archive_busy || active.archive.is_some() || s.confirm.is_some() { return Err("Finish the open confirmation before switching channels.".into()); }
        if !s.channels.contains_key(id) || s.channels.get(id).is_some_and(|c| c.thread_parent.is_some()) { return Err("This channel is unavailable.".into()); }
        // Only the main window saves its selection.
        if view.is_main() { s.window.set_selected_conversation(Some(id))?; }
        let st = &mut *s;
        let target = st.views.get_mut(view).expect("the view exists");
        // The thread panel belongs to the channel that held its anchor.
        if target.open_thread.take().is_some() { st.thread_rev += 1; }
        target.selected = id.to_owned();
        target.current_branch = None;
        target.branch_generation += 1;
        s.rev += 1; s.history_rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn history_snapshot(&self, view: &ViewId) -> HistoryDto {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get(view) else { return closed_history(&s) };
        HistoryDto { channel_id: v.selected.clone(), rev: s.history_rev, messages: channel_messages(&s, current_of(&s, v)) }
    }
    /// The history of a window that has no view. A command for such a window reads no view.
    pub fn closed_history(&self) -> HistoryDto { closed_history(&self.state.lock().unwrap_or_else(|e| e.into_inner())) }
    /// The state of a window that has no view. A command for such a window reads no view.
    pub fn closed_snapshot(&self) -> SnapshotDto { closed_snapshot(&self.state.lock().unwrap_or_else(|e| e.into_inner())) }
    /// The state of one view for its window. A view that does not exist gives a minimal state that selects nothing.
    pub fn snapshot(&self, view_id: &ViewId) -> SnapshotDto {
        let (mut snapshot, slot_state, context, slot_cache, focused) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(view) = state.views.get(view_id) else { return closed_snapshot(&state) };
            let active = current_of(&state, view);
            let ui = &active.ui;
            let rev = state.rev;
            let history_rev = state.history_rev;
            let theme = state.theme.clone();
            let labels = SlotLabels {
                project: active.project_name.clone(),
                channel: active.channel.clone(),
                agent_id: active.agent_id.clone(),
                agent_name: active.agent.clone(),
                provider: active.provider.clone(),
            };
            let slot_state = ui.slot_state(&labels);
            let branches = view.branches.iter().enumerate().map(|(index, row)| BranchRowDto {
                repository: view.branch_repositories.get(index).cloned().unwrap_or_default(),
                key: row.selection_key(view.branch_repositories.get(index).map(String::as_str).unwrap_or("")),
                label: row.name.clone(),
                kind: match row.kind { RowKind::Main => "main", RowKind::Worktree => "worktree", RowKind::Branch => "branch" }.into(),
                depth: row.depth(), current: view.current_branch.is_some_and(|i| std::ptr::eq(&view.branches[i], row)),
                collapsed: view.collapsed.contains(&row.name),
                detached: row.detached,
                worktree: row.worktree.as_ref().map(|path| path.to_string_lossy().into_owned()),
                has_children: view.branches.iter().enumerate().any(|(child_index, child)| view.branch_repositories.get(child_index) == view.branch_repositories.get(index) && child.parent.as_ref() == Some(&row.name)),
            }).collect();
            let mut channels: Vec<_> = state.channels.values()
                    .filter(|channel| channel.thread_parent.is_none() && lists_project(view_id, &view.project_id, channel.project_id.as_ref())).map(|channel| ChannelDto {
                        threads: followed_links(&state, &channel.channel_id),
                        id: channel.channel_id.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.as_ref().map(|id| id.0.clone()), name: channel.channel.clone(), agent: channel.agent.clone(),
                        provider: channel.provider.clone(), workspace_mode: channel.mode.as_str().into(),
                        running: channel.ui.turn_running(), pending_approvals: channel.ui.pending_approvals(),
                        thread_pending_approvals: state.threads.values().filter(|r| r.parent == channel.channel_id).map(|r| thread_approvals(&state, &r.id)).sum(),
                        unread: super::windows::channel_indicators(channel.ui.history(), state.window.last_read(&channel.channel_id), channel.ui.pending_approvals(), channel.muted).0,
                        muted: channel.muted,
                    }).collect();
            channels.sort_by(|a, b| a.id.cmp(&b.id));

            let snapshot = SnapshotDto {
                rev,
                history_rev,
                memory_rev: state.memory_rev,
                project: view.project.clone(),
                project_id: view.project_id.clone(),
                project_path: view.project_path.clone(),
                project_display_path: display_project_path(&view.project_path, std::env::home_dir().as_deref()),
                active_project_id: active.project_id.as_ref().map(|id| id.0.clone()),
                composer_disabled: active.channel_id.is_empty() || active.read_only || active.archived,
                channel_id: active.channel_id.clone(),
                agent_id: active.agent_id.clone(),
                channel: active.channel.clone(),
                agent: active.agent.clone(),
                provider: active.provider.clone(),
                responder: ui.responder().cloned(),
                agent_status: agent_status_name(ui.agent_status()).into(),
                turn_running: ui.turn_running(),
                // The indicator describes a running response only, like the plugin slot state.
                autorun: if ui.turn_running() { AutorunDto::from_profile(&ui.session_profile) } else { None },
                resume_state: ui.resume_state().map(|state| ResumeStateDto { state: state.clone(), agent: ui.responder().map_or_else(|| active.agent.clone(), |author| author.name.clone()) }),
                notice: ui.notice().map(str::to_owned),
                live_blocks: ui.live_blocks().to_vec(),
                prompts: prompt_dtos(ui),
                pending_approvals: ui.pending_approvals(),
                unread: super::windows::channel_indicators(ui.history(), state.window.last_read(&active.channel_id), ui.pending_approvals(), active.muted).0,
                new_anchor_id: active.new_anchor.clone(),
                archived: active.archived,
                archive_busy: view_busy(&state, view) || active.archive_busy,
                member_list_visible: view.member_list_visible,
                panels: PanelsDto {
                    channel_list: view.panels.channel_list,
                    member_list: view.panels.member_list,
                },
                channels,
                live_sub_agent_threads: live_sub_agent_chips(&state, &active.channel_id),
                channel_threads: channel_thread_list(&state, &active.channel_id),
                thread_approvals: pending_thread_approvals(&state, |project| lists_project(view_id, &view.project_id, project)),
                open_thread: open_thread_dto(&state, view),
                branches,
                repositories: view.repositories.clone(),
                repositories_loading: view.repositories_loading,
                slots: BTreeMap::new(),
                banners: BannersDto {
                    review_count: 0,
                    review_names: vec![],
                    problem_lines: state.plugin_notice.iter().cloned().collect(),
                    // The confirmation is app-wide. Only the focused window shows it.
                    confirm: state.confirm.filter(|_| focused_view_of(&state) == *view_id).map(|item| item.message().to_string()),
                    archive: active.archive.clone().map(|message| ArchiveNoticeDto { message }),
                    safe_mode: false,
                },
                theme,
            };
            let focused = focused_view_of(&state) == *view_id;
            (snapshot, slot_state, (AgentId(active.agent_id.clone()), active.project_id.clone()), view.slots.clone(), focused)
        };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots = BTreeMap::new();
        if let Some(plugins) = plugins.as_ref() {
            // The plugins read one UI state: the state of the focused view.
            if focused { plugins.set_ui_state(slot_state.clone()); }
            let mut manager = plugins.manager().lock().unwrap_or_else(|e| e.into_inner());
            slots = slot_cache.lock().unwrap_or_else(|e| e.into_inner())
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
    pub fn configure_window(&self, view: &ViewId, data_dir: &Path, project_dir: &Path, channel_dir: Option<&Path>) {
        let rows = discard_core::worktree::list_branch_tree(project_dir).unwrap_or_default();
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.window = StateStore::open(&data_dir.join("state.toml"));
            sync_main_layout(&mut s);
        }
        self.set_branches(view, rows, channel_dir);
    }
    pub fn set_branches(&self, view: &ViewId, rows: Vec<TreeRow>, channel_dir: Option<&Path>) {
        let current = channel_dir.and_then(|path| {
            let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            current_row(&rows, &path)
        });
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return };
        v.branch_repositories = vec![String::new(); rows.len()];
        v.repositories.clear();
        v.repositories_loading = false;
        v.branches = rows;
        v.current_branch = current;
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn configure_branch_source(&self, view: &ViewId, store: Arc<Mutex<discard_core::store::Store>>, channel: discard_core::model::ChannelId, mode: discard_core::model::WorkspaceMode, project: PathBuf) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return };
        v.branch_source = Some(BranchSource { store, project });
        set_selected_id(&mut state, view, &channel.0);
        current_mut(&mut state, view).mode = mode;
    }
    /// Start a branch refresh of a view. A refresh that an older request started does not apply its result.
    pub fn request_branch_refresh(&self, view: &ViewId) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return 0 };
        v.branch_generation += 1;
        v.branch_generation
    }
    /// What marks the selected branch row: the canonical branch of the channel, or its legacy working directory.
    fn branch_hint(&self, view: &ViewId) -> Option<(PathBuf, BranchHint)> {
        let (source, project, channel, mode, active_project) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let v = state.views.get(view)?;
            let active = current_of(&state, v);
            (v.branch_source.clone(), PathBuf::from(&v.project_path), discard_core::model::ChannelId(active.channel_id.clone()), active.mode,
                active.project_id.as_ref().is_some_and(|id| id.0 == v.project_id))
        };
        if project.as_os_str().is_empty() { return None; }
        let hint = source.filter(|source| active_project && source.project == project).map(|source| {
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
        Some((project, hint))
    }
    fn pick_branch(rows: &[TreeRow], repositories: &[String], (canonical_branch, current): BranchHint) -> Option<usize> {
        canonical_branch.and_then(|(repository, branch)| {
            rows.iter().enumerate().position(|(index, row)| repositories[index] == repository && !row.detached && row.name == branch)
        }).or_else(|| current.and_then(|path| current_row(rows, &path.canonicalize().unwrap_or(path))))
    }
    /// Mark the selected channel's row in the rows already in state. It runs no Git command.
    pub fn reselect_current_branch(&self, view: &ViewId) {
        let Some((_, hint)) = self.branch_hint(view) else {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(v) = state.views.get_mut(view) else { return };
            v.current_branch = None;
            state.rev += 1;
            self.changed.notify_one();
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return };
        v.current_branch = Self::pick_branch(&v.branches, &v.branch_repositories, hint);
        state.rev += 1;
        self.changed.notify_one();
    }
    /// Reload repositories on a blocking thread. The caller does not wait for Git.
    pub fn refresh_branches_in_background(self: &Arc<Self>, view: &ViewId) {
        let generation = self.request_branch_refresh(view);
        let core = self.clone();
        let view = view.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return core.refresh_branches(&view, generation) };
        if let Some(v) = self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get_mut(&view) { v.branch_refreshes_pending += 1; }
        runtime.spawn_blocking(move || {
            core.refresh_branches(&view, generation);
            core.finish_branch_refresh(&view);
        });
    }
    /// A background refresh of the view ended. The view can have closed and opened again under the same label while
    /// the refresh ran, so the count of the new view can be 0.
    fn finish_branch_refresh(&self, view: &ViewId) {
        if let Some(v) = self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get_mut(view) { v.branch_refreshes_pending = v.branch_refreshes_pending.saturating_sub(1); }
    }
    /// Reload the branches of every view that selects `channel`. The engine of the channel changed its state, and
    /// the rows of each view mark the branch of its selected channel. Call it from an async task.
    pub fn refresh_branches_selecting(self: &Arc<Self>, channel: &str) {
        for view in self.views_selecting(channel) {
            let generation = self.request_branch_refresh(&view);
            let core = self.clone();
            tokio::task::spawn_blocking(move || core.refresh_branches(&view, generation));
        }
    }
    /// True while a background branch refresh of the view runs.
    pub fn branch_refresh_pending(&self, view: &ViewId) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get(view).is_some_and(|v| v.branch_refreshes_pending > 0) }
    /// Reload repositories for the project of a view, even without an active conversation.
    pub fn refresh_branches(&self, view: &ViewId, generation: u64) {
        let Some((project, _)) = self.branch_hint(view) else { return };
        let discovered = discard_core::worktree::discover_repositories(&project).unwrap_or_default();
        let mut rows = Vec::new();
        let mut branch_repositories = Vec::new();
        let mut repositories = Vec::new();
        for repository in discovered {
            let mut repo_rows = discard_core::worktree::list_branch_tree(&repository.path).unwrap_or_default();
            let listed: Vec<String> = repo_rows.iter().filter(|row| !row.detached).map(|row| row.name.clone()).collect();
            if self.follow_renamed_branches(view, &repository.id, &repository.path, &listed) {
                repo_rows = discard_core::worktree::list_branch_tree(&repository.path).unwrap_or_default();
            }
            branch_repositories.extend(std::iter::repeat_n(repository.id.clone(), repo_rows.len()));
            rows.extend(repo_rows);
            repositories.push(RepositoryDto {
                id: repository.id,
                label: repository.label,
                path: repository.path.to_string_lossy().into_owned(),
            });
        }
        // Read the hint after the renames: a followed branch changes the binding that marks the selected row.
        let Some((_, hint)) = self.branch_hint(view) else { return };
        let selected = Self::pick_branch(&rows, &branch_repositories, hint);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return };
        if v.branch_generation == generation {
            v.branches = rows;
            v.branch_repositories = branch_repositories;
            v.repositories = repositories;
            v.repositories_loading = false;
            v.current_branch = selected;
            state.rev += 1;
            self.changed.notify_one();
        }
    }
    /// Move the binding of each active branch channel whose branch was renamed outside Discard. A channel moves only
    /// when the reflog gives one new name that no other channel binds or reaches, and the channel is not busy.
    /// A busy channel moves on a later refresh. Errors skip the channel. Returns true when a binding moved.
    fn follow_renamed_branches(&self, view: &ViewId, repository: &str, repo_path: &Path, listed: &[String]) -> bool {
        use discard_core::model::{Block, ChannelId, Message, MessageId, ProjectChannelKind, ProjectId, Role};
        let (source, project) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(v) = state.views.get(view) else { return false };
            (v.branch_source.clone(), v.project_id.clone())
        };
        let Some(source) = source else { return false };
        if project.is_empty() { return false; }
        let project = ProjectId(project);
        // Stranded channels and the listed names that no active channel binds.
        let (stranded, candidates) = {
            let store = source.store.lock().unwrap_or_else(|e| e.into_inner());
            let Ok(channels) = store.all_channels_for_project(&project) else { return false };
            let mut bound = std::collections::HashSet::new();
            let mut stranded = Vec::new();
            for (channel, archived) in channels {
                if archived { continue; }
                let Ok(Some(metadata)) = store.project_channel_metadata(&channel.id) else { continue };
                if metadata.kind != ProjectChannelKind::Branch || metadata.repository != repository { continue; }
                let Some(branch) = metadata.branch else { continue };
                if listed.contains(&branch) { bound.insert(branch); } else { stranded.push((channel.id, branch)); }
            }
            let candidates: Vec<String> = listed.iter().filter(|name| !bound.contains(*name)).cloned().collect();
            (stranded, candidates)
        };
        if stranded.is_empty() || candidates.is_empty() { return false; }
        // A busy channel still counts when it looks for conflicts, so its target stays unclaimed until it moves.
        // A channel without a creation time moves nothing: an old rename could belong to another branch.
        let edges = discard_core::branch_actions::rename_edges(repo_path, &candidates);
        let mut targets: Vec<(ChannelId, String, String)> = Vec::new();
        for (id, old) in stranded {
            let Some(created) = id.created_at() else { continue };
            if let Some(new) = discard_core::branch_actions::follow_rename(&edges, &old, &candidates, created) { targets.push((id, old, new)); }
        }
        let mut moved = false;
        for (id, old, new) in &targets {
            if self.channel_busy_reason(&id.0).is_some() { continue; }
            if targets.iter().filter(|(_, _, other)| other == new).count() > 1 { continue; }
            let store = source.store.lock().unwrap_or_else(|e| e.into_inner());
            if store.project_repository_branch_channel(&project, repository, new).ok().flatten().is_some() { continue; }
            if store.rename_project_branch(&project, id, new).is_err() { continue; }
            moved = true;
            let note = Message { id: MessageId::new(), channel_id: id.clone(), role: Role::System, blocks: vec![Block::Markdown { text: format!("Branch renamed from `{old}` to `{new}`.") }] };
            if store.insert_message(&note).is_ok() {
                drop(store);
                self.apply_channel(&id.0, Update::ConversationMarker(note));
            }
        }
        moved
    }
    pub fn configure_targets(&self, view: &ViewId, project_id: String, project_dir: &Path, agent_id: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get_mut(view) else { return };
        v.project_id = project_id;
        v.project_path = project_dir.to_string_lossy().into_owned();
        let project_id = (!v.project_id.is_empty()).then(|| discard_core::model::ProjectId(v.project_id.clone()));
        let active = current_mut(&mut state, view);
        active.agent_id = agent_id;
        active.project_id = project_id;
    }
    /// Initialize read markers for this channel, treating old history as read on first use.
    pub fn configure_read_marker(&self, view: &ViewId, data_dir: &Path, channel_id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.views.contains_key(view) { return; }
        s.window = StateStore::open(&data_dir.join("state.toml"));
        sync_main_layout(&mut s);
        set_selected_id(&mut s, view, channel_id);
        let marker = startup_marker(current(&s, view).ui.history(), s.window.last_read(channel_id));
        if let Some(marker) = marker { let _ = s.window.set_last_read(channel_id, &marker); }
        let anchor = new_divider_anchor(current(&s, view).ui.history(), s.window.last_read(channel_id), None, false);
        current_mut(&mut s, view).new_anchor = anchor;
    }
    /// Mark a message of the channel that a view selects as read.
    pub fn mark_read(&self, view: &ViewId, message_id: &str) -> Result<(), String> {
        let Some(selected) = self.selected_of(view) else { return Err(WINDOW_CLOSED.into()) };
        if selected.is_empty() { return Ok(()); }
        self.mark_read_in(Some(&selected), message_id)
    }
    /// Mark a message of a channel that a view shows, or of a loaded thread, as read. `None` names no channel.
    pub fn mark_read_in(&self, channel: Option<&str>, message_id: &str) -> Result<(), String> {
        let Some(channel) = channel else { return Ok(()) };
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !is_shown(&s, channel) {
            if s.channels.get(channel).is_some_and(|c| c.thread_parent.is_some()) { return self.mark_thread_read(&mut s, channel, message_id); }
            return Ok(());
        }
        let Some(index) = channel_state(&s, channel).and_then(|c| c.ui.history().iter().position(|m| m.id.0 == message_id)) else { return Ok(()) };
        if s.window.last_read(channel).and_then(|id| channel_state(&s, channel).and_then(|c| c.ui.history().iter().position(|m| m.id.0 == id))).is_some_and(|previous| previous > index) { return Ok(()); }
        let changed = s.window.last_read(channel) != Some(message_id);
        if changed { s.window.set_last_read(channel, message_id)?; }
        let cleared = s.channels.get_mut(channel).is_some_and(|c| c.new_anchor.take().is_some());
        if changed || cleared { s.rev += 1; self.changed.notify_one(); }
        Ok(())
    }
    /// The last message id of each loaded non-thread channel of these projects. An empty history gives `None`.
    pub fn loaded_channel_tails(&self, project_ids: &[String]) -> Vec<(String, Option<String>)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.values()
            .filter(|c| c.thread_parent.is_none() && c.project_id.as_ref().is_some_and(|p| project_ids.contains(&p.0)))
            .map(|c| (c.channel_id.clone(), c.ui.history().last().map(|m| m.id.0.clone())))
            .collect()
    }
    /// Save read markers for channels and threads with one write. Returns whether any marker changed.
    /// A save error changes nothing in memory. Pending approvals stay as they are.
    /// Callers pass current tails, so a marker does not move backward in practice.
    pub fn apply_read_markers(&self, markers: &[(String, String)]) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A marker never moves backward. A channel that is not loaded has no history to compare, so it takes the new marker.
        let position = |s: &State, channel: &str, message: &str| channel_state(s, channel).and_then(|c| c.ui.history().iter().position(|m| m.id.0 == message));
        let changed: Vec<(String, String)> = markers.iter().filter(|(id, message)| {
            let current = s.window.last_read(id);
            if current == Some(message.as_str()) { return false; }
            let behind = current.and_then(|current| position(&s, id, current)).zip(position(&s, id, message)).is_some_and(|(current, new)| current >= new);
            !behind
        }).cloned().collect();
        if changed.is_empty() { return Ok(false); }
        s.window.set_last_read_many(&changed)?;
        let mut shown = false;
        for (id, _) in &changed {
            if let Some(channel) = s.channels.get_mut(id) { channel.new_anchor = None; }
            shown |= is_shown(&s, id) || s.threads.get(id).is_some_and(|record| is_shown(&s, &record.parent));
        }
        if shown { s.history_rev += 1; }
        s.rev += 1;
        self.conversations_rev.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
        Ok(true)
    }
    pub fn window_bounds(&self) -> Option<Bounds> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).window.state().bounds
    }
    pub fn save_bounds(&self, bounds: Bounds, screens: &[Bounds]) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        super::windows::save_restorable_bounds(&mut s.window, bounds, screens)
    }
    /// Set the panel widths of a view. Only the main window saves them. Another view keeps them in memory.
    pub fn set_panels(&self, view: &ViewId, channel_list: f32, member_list: f32) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.views.contains_key(view) { return Err(WINDOW_CLOSED.into()); }
        let panels = PanelWidths { channel_list, member_list };
        if view.is_main() { s.window.set_panels(panels)?; sync_main_layout(&mut s); }
        else { s.views.get_mut(view).expect("the view exists").panels = clamped_panels(panels); }
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    /// Show or hide the member list of a view. Only the main window saves it. Another view keeps it in memory.
    pub fn set_member_list_visible(&self, view: &ViewId, visible: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.views.contains_key(view) { return Err(WINDOW_CLOSED.into()); }
        if view.is_main() { s.window.set_member_list_visible(visible)?; sync_main_layout(&mut s); }
        else { s.views.get_mut(view).expect("the view exists").member_list_visible = visible; }
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn toggle_branch(&self, view: &ViewId, key: String) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get_mut(view) else { return };
        if !v.collapsed.remove(&key) { v.collapsed.insert(key); }
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
    pub fn plugin_action(&self, view: &ViewId, plugin: &str, action: &str) -> Result<(), String> {
        let (agent, project) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); let Some(v) = state.views.get(view) else { return Err(WINDOW_CLOSED.into()) }; let active = current_of(&state, v); (AgentId(active.agent_id.clone()), active.project_id.clone()) };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Err("plugins are unavailable".into()) };
        let result = PluginMemory::for_context(agent, project, || plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_action(plugin, action).map_err(|e| e.to_string()));
        self.clear_slots();
        result
    }
    /// The slash commands of the running plugins, in load order. Empty when plugins are unavailable.
    pub fn plugin_commands(&self) -> Vec<discard_core::slash::PluginCommand> {
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Vec::new() };
        let text = |value: Option<String>| value.filter(|text| !text.trim().is_empty());
        let commands = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).commands();
        commands.into_iter().map(|(plugin, info)| discard_core::slash::PluginCommand { plugin, name: info.name, description: text(info.description), args: text(info.args) }).collect()
    }
    /// The state of a channel a command can target: the active channel or a loaded thread.
    pub fn command_channel(&self, channel_id: &str) -> Option<CommandChannel> {
        if channel_id.is_empty() { return None; }
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = shown_channel(&s, channel_id) {
            return Some(CommandChannel { turn_running: active.ui.turn_running(), archived: active.archived || active.read_only, is_thread: false });
        }
        let thread = s.channels.get(channel_id).filter(|c| c.thread_parent.is_some())?;
        Some(CommandChannel { turn_running: thread.ui.turn_running(), archived: thread_blocked(&s, thread), is_thread: true })
    }
    /// Run a plugin command in the memory context (agent, project) of `channel_id`. `None` when no running plugin has the command.
    pub fn run_plugin_command(&self, channel_id: &str, name: &str, args: &str) -> Option<Result<Option<String>, String>> {
        let context = { let s = self.state.lock().unwrap_or_else(|e| e.into_inner()); channel_state(&s, channel_id).map(|c| (AgentId(c.agent_id.clone()), c.project_id.clone())) };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let plugins = plugins.as_ref()?;
        let Some((agent, project)) = context else { return Some(Err("The channel is not open.".into())) };
        let result = PluginMemory::for_context(agent, project, || plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_command(name, args));
        self.clear_slots();
        result.map(|result| result.map_err(|e| e.to_string()))
    }
    /// Discard the plugin trees after plugin state changes.
    pub fn clear_slots(&self) {
        let caches: Vec<_> = self.state.lock().unwrap_or_else(|e| e.into_inner()).views.values().map(|v| v.slots.clone()).collect();
        for cache in caches { cache.lock().unwrap_or_else(|e| e.into_inner()).clear(); }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn set_load_report(&self, report: LoadReport) {
        *self.load_report.lock().unwrap_or_else(|e| e.into_inner()) = report;
    }
    /// Show a notice in the window of a view.
    pub fn notice(&self, view: &ViewId, message: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.views.contains_key(view) { return; }
        current_mut(&mut state, view).ui.apply(Update::Notice(message));
        state.rev += 1;
        self.changed.notify_one();
    }
    /// Retain an existing recovery notice when an idle conversation cannot restart.
    pub fn append_channel_notice(&self, id: &str, message: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(channel) = state.channels.get_mut(id) else { return };
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
    pub fn archive_started(&self, view: &ViewId) -> bool { self.archive_started_for(view, None) }
    pub fn archive_started_for(&self, view: &ViewId, expected_channel: Option<&str>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = state.views.get(view) else { return false };
        let active = current_of(&state, v);
        if active.channel_id.is_empty() || active.read_only || state.changing_agents.contains(&active.agent_id) || state.changing_channels.contains(&active.channel_id) || view_busy(&state, v) || expected_channel.is_some_and(|id| id != active.channel_id) || active.ui.turn_running() || active.archived || active.archive_busy { return false; }
        let active = current_mut(&mut state, view);
        active.archive = None;
        active.archive_busy = true;
        state.rev += 1;
        self.changed.notify_one();
        true
    }
    pub fn archive_finished(&self, view: &ViewId, result: super::plan::ArchiveStep) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.views.contains_key(view) { return; }
        let active = current_mut(&mut state, view);
        active.archive_busy = false;
        match result {
            super::plan::ArchiveStep::Done(message) => { active.archived = true; active.ui.apply(Update::Notice(message)); }
            super::plan::ArchiveStep::NeedsConfirm(items) => active.archive = Some(format!("Archiving will delete these items:\n{}", items.join("\n"))),
            super::plan::ArchiveStep::Failed(message) => active.ui.apply(Update::Notice(format!("Could not archive channel: {message}"))),
        }
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn cancel_archive(&self, view: &ViewId) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.views.contains_key(view) { return; }
        if current_mut(&mut state, view).archive.take().is_some() { state.rev += 1; self.changed.notify_one(); }
    }
    pub fn has_pending_archive(&self, view: &ViewId) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.views.get(view).is_some_and(|v| current_of(&state, v).archive.is_some())
    }
    pub fn resolve_copy(&self, view: &ViewId, target: &discard_core::context_menu::ContextTarget, action: discard_core::action::Action) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let v = state.views.get(view)?;
        let active = current_of(&state, v);
        discard_core::context_menu::resolve_copy(active.ui.history(), &active.channel, target, action)
            .or_else(|| {
                let thread = v.open_thread.as_ref().and_then(|id| state.channels.get(id))?;
                discard_core::context_menu::resolve_copy(thread.ui.history(), &thread.channel, target, action)
            })
    }
    pub fn menu_context(&self, view: &ViewId) -> discard_core::action::MenuContext {
        use discard_core::action::{MenuContext, MenuTarget, PluginMenuItem};
        let (turn_running, archived) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); state.views.get(view).map(|v| current_of(&state, v)).map_or((false, true), |active| (active.ui.turn_running(), active.archived || active.read_only || active.channel_id.is_empty())) };
        let safe_mode = self.load_report.lock().unwrap_or_else(|e| e.into_inner()).safe_mode;
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return MenuContext { turn_running, archived, safe_mode, ..Default::default() } };
        let plugin_items = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).menu_items().into_iter().filter_map(|(plugin, index, item)| Some(PluginMenuItem { plugin, index, target: MenuTarget::parse(&item.target)?, label: item.label })).collect();
        MenuContext { turn_running, archived, safe_mode, pending_reviews: plugins.pending().len(), plugin_items }
    }
    pub fn run_plugin_menu(&self, view: &ViewId, plugin: &str, index: usize, target: Option<&discard_core::context_menu::ContextTarget>) -> Result<(), String> {
        let (agent, project) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); let Some(v) = state.views.get(view) else { return Err(WINDOW_CLOSED.into()) }; let active = current_of(&state, v); (AgentId(active.agent_id.clone()), active.project_id.clone()) };
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
    pub fn history(&self, view: &ViewId) -> Vec<MessageDto> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.views.get(view).map(|v| channel_messages(&state, current_of(&state, v))).unwrap_or_default()
    }
    /// Send a message to the channel that a view selects.
    pub fn send(&self, view: &ViewId, text: String) {
        let Some(selected) = self.selected_of(view) else { return };
        let _ = self.send_to(Some(&selected), text);
    }
    /// Send a message to a channel that a view shows. `None` names no channel, and nothing is sent.
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
        let Some(channel) = channel else { return Err("Name the channel to send to.".into()) };
        self.send_user_message(channel, text, recipient, attachments, false)
    }
    /// Send a user message to a loaded channel that no view needs to show. It has the checks and the effects of
    /// `send_to_agent_with_attachments`.
    pub fn send_in_background(&self, channel: &str, text: String, recipient: Option<discard_core::model::MessageAuthor>, attachments: Vec<discard_core::attachments::Attachment>) -> Result<(), String> {
        if text.trim().is_empty() && attachments.is_empty() {
            return Ok(());
        }
        self.send_user_message(channel, text, recipient, attachments, true)
    }
    /// The checks of a user message. The channel is the one that a view shows, or with `background` any loaded
    /// channel that is not a thread. One predicate serves the check and the send, so they cannot differ.
    fn send_blocked(s: &State, channel: &str, recipient: Option<&discard_core::model::MessageAuthor>, background: bool) -> Result<(), String> {
        let found = if background { channel_state(s, channel).filter(|c| c.thread_parent.is_none()) } else { shown_channel(s, channel) };
        let Some(active) = found else { return Err("The channel changed. Send the message again.".into()) };
        // A background send belongs to an action that holds the app-wide guard itself, such as a move approval.
        let guarded = if background { s.views.values().any(|v| v.navigation_busy && v.selected == channel) } else { busy_for(s, channel) };
        if active.read_only || s.changing_agents.contains(&active.agent_id) || s.changing_channels.contains(&active.channel_id) || guarded || active.ui.turn_running() || active.archived || active.archive_busy || active.archive.is_some() {
            return Err("This channel cannot send a message right now.".into());
        }
        if recipient.is_some_and(|agent| s.changing_agents.contains(&agent.agent_id.0)) {
            return Err("The mentioned agent is being updated. Try again.".into());
        }
        Ok(())
    }
    /// Check that `send_in_background` would pass its checks now. Only a closed engine session can still refuse it.
    pub fn can_send_in_background(&self, channel: &str, recipient: Option<&discard_core::model::MessageAuthor>) -> Result<(), String> {
        Self::send_blocked(&self.state.lock().unwrap_or_else(|e| e.into_inner()), channel, recipient, true)
    }
    fn send_user_message(&self, channel: &str, text: String, recipient: Option<discard_core::model::MessageAuthor>, attachments: Vec<discard_core::attachments::Attachment>, background: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::send_blocked(&s, channel, recipient.as_ref(), background)?;
        let active = channel_state(&s, channel).expect("checked under lock");
        let command = if attachments.is_empty() { Command::Send(text.clone()) } else { Command::SendAttachments { text: text.clone(), attachments: attachments.clone() } };
        if active.commands.send(command).is_ok() {
            let shown = is_shown(&s, channel);
            let active = s.channels.get_mut(channel).expect("checked under lock");
            active.new_anchor = None;
            active.ui.user_sent_with_attachments(&text, &attachments);
            if let Some(author) = recipient { active.ui.apply(Update::Responding(author)); }
            s.rev += 1;
            if shown { s.history_rev += 1; }
            self.changed.notify_one();
        } else { return Err("The agent session is unavailable.".into()); }
        Ok(())
    }
    /// Start a response of `author` in a loaded channel that no view needs to show, with no user message. The move of a
    /// conversation uses it, so the target does not receive a copy of the instruction to move. It has the checks of
    /// `send_in_background`.
    pub fn continue_in_background(&self, channel: &str, author: discard_core::model::MessageAuthor, note: String) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::send_blocked(&s, channel, Some(&author), true)?;
        let active = s.channels.get_mut(channel).expect("checked under lock");
        active.commands.send(Command::Continue { agent: author.agent_id.clone(), note }).map_err(|_| "The agent session is unavailable.")?;
        active.new_anchor = None;
        active.ui.response_started();
        active.ui.apply(Update::Responding(author));
        s.rev += 1;
        if is_shown(&s, channel) { s.history_rev += 1; }
        self.changed.notify_one();
        Ok(())
    }
    /// Record the author of a message of a loaded channel, such as a handoff that an agent posted.
    pub fn set_message_author(&self, channel: &str, message: discard_core::model::MessageId, author: discard_core::model::MessageAuthor) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let shown = is_shown(&s, channel);
        let Some(target) = s.channels.get_mut(channel) else { return };
        target.ui.set_author(message, author);
        s.rev += 1;
        if shown { s.history_rev += 1; }
        self.changed.notify_one();
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
        let Some(active) = shown_channel(s, channel) else { return Err("Open this channel first.".into()) };
        if active.ui.turn_running() { return Err("Wait for the current response to finish.".into()); }
        if active.read_only || active.archived || active.archive_busy || active.archive.is_some() || busy_for(s, channel) || s.changing_channels.contains(channel) || agent.is_some_and(|agent| s.changing_agents.contains(&agent.0)) {
            return Err("This channel cannot send a message right now.".into());
        }
        Ok(())
    }
    fn continue_with(&self, channel: &str, agent: AgentId, note: String, declined: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !is_shown(&s, channel) { return Err("The channel changed. Open the channel and try again.".into()); }
        Self::continue_blocked(&s, channel, Some(&agent))?;
        let command = if declined { Command::ContinueDeclined { agent, note } } else { Command::Continue { agent, note } };
        let active = shown_channel_mut(&mut s, channel).expect("checked under lock");
        active.commands.send(command).map_err(|_| "The agent session is unavailable.")?;
        active.ui.response_started();
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }

    /// Decide an approval of the channel that a view selects.
    pub fn decide(&self, view: &ViewId, approval_id: String, decision: String) {
        let Some(selected) = self.selected_of(view) else { return };
        self.decide_in(Some(&selected), approval_id, decision, None);
    }
    /// Decide an approval of a channel that a view shows, or of a loaded thread. `None` names no channel.
    pub fn decide_in(&self, channel: Option<&str>, approval_id: String, decision: String, rule_cut: Option<usize>) {
        let Some(channel) = channel else { return };
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
    /// Stop the turn of the channel that a view selects.
    pub fn cancel(&self, view: &ViewId) {
        let Some(selected) = self.selected_of(view) else { return };
        self.cancel_in(Some(&selected));
    }
    /// Stop the turn of a channel that a view shows or of a loaded thread. `None` names no channel. Stopping a
    /// parent channel also stops the sub-agents that its turn started. Stopping a sub-agent thread ends only that task.
    pub fn cancel_in(&self, channel: Option<&str>) {
        let Some(channel) = channel else { return };
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
        for thread in threads { if let Some(channel) = s.channels.get(&thread).filter(|c| c.thread_parent.is_some()) { let _ = channel.commands.send(Command::Cancel); } }
    }
    /// Track a thread. Its state lives in the channel pool and never becomes the selected channel.
    /// A thread starts with no read marker, so an agent message that nobody read counts as unread.
    pub fn add_thread(&self, init: CoreInit, channel: &discard_core::model::Channel, info: &discard_core::model::ThreadInfo) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.channels.contains_key(&channel.id.0) { return; }
        s.runtime_generation += 1;
        let record = ThreadRecord::build(info, init.state.history(), init.state.authors());
        let mut runtime = ChannelState::new(init);
        runtime.runtime_generation = s.runtime_generation;
        runtime.channel_id = channel.id.0.clone(); runtime.agent_id = channel.agent_id.0.clone(); runtime.mode = channel.workspace_mode; runtime.project_id = channel.project_id.clone();
        runtime.thread_parent = Some(info.parent_channel_id.0.clone());
        s.channels.insert(channel.id.0.clone(), runtime);
        s.threads.insert(channel.id.0.clone(), record);
        if is_shown(&s, &info.parent_channel_id.0) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Drop the runtime of a sub-agent thread whose task could not start, and stop its engine. The thread keeps its
    /// record, so its chip shows the final state. Opening the thread later loads it again from storage.
    pub fn drop_thread_runtime(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(runtime) = s.channels.get(id).filter(|c| c.thread_parent.is_some()) else { return };
        // The engine ends its turn on a stop, and its loop ends when the sender is dropped.
        let _ = runtime.commands.send(Command::Cancel);
        s.channels.remove(id);
        close_open_thread(&mut s, id);
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Replace what the app knows about the threads of `parent`. A loaded thread keeps its live record.
    pub fn set_channel_threads(&self, parent: &str, records: Vec<ThreadRecord>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let shown = is_shown(&s, parent);
        let st = &mut *s;
        let loaded: HashSet<String> = st.channels.iter().filter(|(_, c)| c.thread_parent.as_deref() == Some(parent)).map(|(id, _)| id.clone()).collect();
        st.threads.retain(|id, record| record.parent != parent || loaded.contains(id));
        for record in records.into_iter().filter(|record| !loaded.contains(&record.id)) { st.threads.insert(record.id.clone(), record); }
        if shown { st.history_rev += 1; }
        st.rev += 1;
        self.changed.notify_one();
    }
    /// Set the reference rows of a loaded channel: the thread message each one shows.
    pub fn set_posted(&self, channel: &str, posted: HashMap<discard_core::model::MessageId, PostedSource>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let is_active = is_shown(&s, channel);
        let Some(target) = s.channels.get_mut(channel) else { return };
        target.posted = posted;
        if is_active { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn thread_parent(&self, id: &str) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).channels.get(id).and_then(|c| c.thread_parent.clone())
    }
    pub fn open_thread_id(&self, view: &ViewId) -> Option<String> { self.state.lock().unwrap_or_else(|e| e.into_inner()).views.get(view)?.open_thread.clone() }
    /// Show a loaded thread beside the selected channel of a view. Only a thread of that channel can open.
    pub fn open_thread(&self, view: &ViewId, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get(view) else { return Err(WINDOW_CLOSED.into()) };
        let Some(record) = s.threads.get(id) else { return Err("This thread is unavailable.".into()) };
        if record.parent != v.selected { return Err("Open this thread's channel first.".into()); }
        if !s.channels.get(id).is_some_and(|c| c.thread_parent.is_some()) { return Err("This thread is not loaded.".into()); }
        if v.open_thread.as_deref() == Some(id) { return Ok(()); }
        s.views.get_mut(view).expect("the view exists").open_thread = Some(id.to_owned());
        s.thread_rev += 1; s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }
    pub fn close_thread(&self, view: &ViewId) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = s.views.get_mut(view) else { return };
        if v.open_thread.take().is_some() { s.thread_rev += 1; s.rev += 1; self.changed.notify_one(); }
    }
    /// Forget a thread that never received its first message.
    pub fn remove_thread(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.channels.get(id).is_some_and(|c| c.thread_parent.is_some()) { return; }
        s.channels.remove(id);
        let parent = s.threads.remove(id).map(|record| record.parent);
        close_open_thread(&mut s, id);
        if parent.is_some_and(|parent| is_shown(&s, &parent)) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    pub fn set_thread_followed(&self, id: &str, followed: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = s.threads.get_mut(id) else { return };
        record.followed = followed;
        let parent = record.parent.clone();
        if is_shown(&s, &parent) { s.history_rev += 1; }
        if thread_open(&s, id) { s.thread_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Why a loaded thread cannot be deleted now: a turn runs, its sub-agent task runs, or an approval waits.
    pub fn thread_delete_blocker(&self, id: &str) -> Option<&'static str> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::delete_blocker(&s, id)
    }
    fn delete_blocker(s: &State, id: &str) -> Option<&'static str> {
        let loaded = s.channels.get(id);
        if loaded.is_some_and(|c| c.ui.turn_running()) { return Some("A turn runs in this thread. Stop it before you delete the thread."); }
        if s.threads.get(id).is_some_and(|r| r.state == Some(ThreadState::Running)) { return Some("A sub-agent task still runs in this thread. Stop it or wait for it before you delete the thread."); }
        if loaded.is_some_and(|c| c.ui.pending_approvals() > 0) { return Some("An approval waits in this thread. Decide it before you delete the thread."); }
        None
    }
    /// Take a thread out of use before its rows go: nothing can send in it until `end_channel_change`. Refused when
    /// `thread_delete_blocker` names a reason.
    pub fn begin_thread_delete(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reason) = Self::delete_blocker(&s, id) { return Err(reason.into()); }
        if !s.changing_channels.insert(id.to_owned()) { return Err("This thread is changing. Try again.".into()); }
        Ok(())
    }
    /// Forget a thread that the store deleted: its runtime, record, read marker and panel. Its posted messages are
    /// normal messages of the parent now, so the parent keeps them and loses only the thread link.
    pub fn thread_deleted(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.channels.get(id).is_some_and(|c| c.thread_parent.is_some()) && let Some(runtime) = s.channels.remove(id) {
            let _ = runtime.commands.send(Command::Cancel);
        }
        let parent = s.threads.remove(id).map(|record| record.parent);
        let _ = s.window.clear_last_read(id);
        close_open_thread(&mut s, id);
        let st = &mut *s;
        for channel in st.channels.values_mut() { channel.posted.retain(|_, source| source.thread_channel_id.0 != id); }
        if parent.is_none_or(|parent| is_shown(&s, &parent)) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Show the new name of a thread everywhere the app holds it: its record, its runtime and the reference rows.
    pub fn thread_renamed(&self, id: &str, name: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = s.threads.get_mut(id) else { return };
        record.name = name.to_owned();
        let parent = record.parent.clone();
        if let Some(runtime) = s.channels.get_mut(id) { runtime.channel = name.to_owned(); }
        let st = &mut *s;
        for channel in st.channels.values_mut() {
            for source in channel.posted.values_mut().filter(|source| source.thread_channel_id.0 == id) { source.thread_name = name.to_owned(); }
        }
        touch_thread(&mut s, &parent, id);
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Show the new name of a channel.
    pub fn channel_renamed(&self, id: &str, name: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = s.channels.get_mut(id) else { return };
        target.channel = name.to_owned();
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Mute or unmute a loaded channel. A muted channel shows no unread marker and counts for no unread marker of
    /// its project. Read markers, messages and turns do not change.
    pub fn set_channel_muted(&self, id: &str, muted: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = s.channels.get_mut(id) else { return };
        if target.muted == muted { return; }
        target.muted = muted;
        s.rev += 1;
        self.changed.notify_one();
    }
    /// Why a channel cannot take a Git or archive action now, or `None` when it is idle. A channel is busy while a
    /// turn runs in it, while an approval waits in it or in one of its threads, while a sub-agent task runs in one of
    /// its threads, and while an archive or another channel action runs.
    fn busy_reason(s: &State, id: &str) -> Option<&'static str> {
        let loaded = s.channels.get(id).filter(|c| c.thread_parent.is_none());
        if loaded.is_some_and(|c| c.ui.turn_running()) { return Some("A turn runs in this channel. Stop it first."); }
        if loaded.is_some_and(|c| c.archive_busy || c.archive.is_some()) { return Some("An archive is in progress for this channel."); }
        if loaded.is_some_and(|c| c.ui.pending_approvals() > 0) { return Some("An approval waits in this channel. Decide it first."); }
        if s.changing_channels.contains(id) { return Some("This channel is changing. Try again."); }
        for record in s.threads.values().filter(|record| record.parent == id) {
            if record.state == Some(ThreadState::Running) { return Some("A sub-agent task runs in a thread of this channel. Stop it first."); }
            let thread = s.channels.get(&record.id);
            if thread.is_some_and(|c| c.ui.turn_running()) { return Some("A turn runs in a thread of this channel. Stop it first."); }
            if thread.is_some_and(|c| c.ui.pending_approvals() > 0) { return Some("An approval waits in a thread of this channel. Decide it first."); }
        }
        None
    }
    pub fn channel_busy_reason(&self, id: &str) -> Option<&'static str> {
        Self::busy_reason(&self.state.lock().unwrap_or_else(|e| e.into_inner()), id)
    }
    /// Take a channel out of use for a channel action until `end_channel_change`. Refused while it is busy.
    pub fn begin_channel_action(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reason) = Self::busy_reason(&s, id) { return Err(reason.into()); }
        s.changing_channels.insert(id.to_owned());
        Ok(())
    }
    /// `begin_channel_action` with the extra rules of the menu bar archive: no archive while the project or the
    /// channel's agent changes, and none for a read-only channel.
    pub fn begin_channel_archive(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reason) = Self::busy_reason(&s, id) { return Err(reason.into()); }
        let loaded = s.channels.get(id).filter(|c| c.thread_parent.is_none());
        if busy_for(&s, id) || loaded.is_some_and(|c| s.changing_agents.contains(&c.agent_id)) { return Err("Finish the open operation before you archive the channel.".into()); }
        if loaded.is_some_and(|c| c.read_only) { return Err("This channel is read only. It cannot be archived.".into()); }
        s.changing_channels.insert(id.to_owned());
        Ok(())
    }
    /// A reference row left its channel: the row, its link and its position in the read marker go. The thread keeps
    /// the message, and the thread panel shows it as not posted.
    pub fn reference_removed(&self, reference: &discard_core::model::MessageId, source: &PostedSource) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let st = &mut *s;
        let Some(parent) = st.channels.values_mut().find(|c| c.posted.contains_key(reference)) else { return };
        let parent_id = parent.channel_id.clone();
        let before = parent.ui.history().iter().position(|m| m.id == *reference).and_then(|index| index.checked_sub(1)).map(|index| parent.ui.history()[index].id.0.clone());
        parent.posted.remove(reference);
        parent.ui.remove_message(reference);
        if parent.new_anchor.as_deref() == Some(reference.0.as_str()) { parent.new_anchor = None; }
        // A marker on the removed row moves to the row before it, so the messages after it keep their read state.
        if st.window.last_read(&parent_id) == Some(reference.0.as_str()) && let Some(before) = before { let _ = st.window.set_last_read(&parent_id, &before); }
        if is_shown(&s, &parent_id) { s.history_rev += 1; }
        if thread_open(&s, source.thread_channel_id.0.as_str()) { s.thread_rev += 1; }
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
        let Some(active) = shown_channel(&s, parent) else { return Err("The channel changed. Start the thread again.".into()) };
        if active.read_only || active.archived || active.archive_busy || active.archive.is_some() || busy_for(&s, parent) || s.changing_channels.contains(parent) {
            return Err("This channel cannot start a thread right now.".into());
        }
        Ok(())
    }
    /// Send a user message in a loaded thread. `also_post` also posts the saved message to the parent channel.
    pub fn send_in_thread(&self, thread: &str, text: String, attachments: Vec<discard_core::attachments::Attachment>, also_post: bool, recipient: Option<discard_core::model::MessageAuthor>) -> Result<(), String> {
        if text.trim().is_empty() && attachments.is_empty() { return Ok(()); }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = s.channels.get(thread).filter(|c| c.thread_parent.is_some()) else { return Err("This thread is unavailable.".into()) };
        // A message in a sub-agent thread whose task runs steers the task. Any other running turn refuses it.
        let steering = s.threads.get(thread).is_some_and(|record| record.kind == ThreadKind::SubAgent && record.state == Some(ThreadState::Running)) && target.ui.turn_running();
        if thread_blocked(&s, target) || s.changing_agents.contains(&target.agent_id) || s.changing_channels.contains(thread) || busy_for(&s, target.thread_parent.as_deref().unwrap_or_default()) || (target.ui.turn_running() && !steering) {
            return Err("This thread cannot send a message right now.".into());
        }
        if recipient.as_ref().is_some_and(|agent| s.changing_agents.contains(&agent.agent_id.0)) {
            return Err("The mentioned agent is being updated. Try again.".into());
        }
        // The engine refuses such a steer too. The refusal here keeps the draft in the composer.
        if steering && recipient.as_ref().is_some_and(|agent| agent.agent_id.0 != target.agent_id) {
            return Err("Wait for the sub-agent to finish before you mention another agent.".into());
        }
        let target = s.channels.get_mut(thread).expect("thread checked under lock");
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
        let Some(target) = s.channels.get_mut(thread).filter(|c| c.thread_parent.is_some()) else { return Err("This thread is unavailable.".into()) };
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
    pub fn thread_history_snapshot(&self, view: &ViewId) -> Option<ThreadHistoryDto> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = s.views.get(view)?.open_thread.as_ref()?;
        let thread = s.channels.get(id)?;
        let record = s.threads.get(id)?;
        let anchor = channel_state(&s, &record.parent).and_then(|parent| {
            let message = parent.ui.history().iter().find(|m| m.id.0 == record.anchor)?;
            history_dto_with_authors(std::slice::from_ref(message), parent.ui.authors()).pop()
        });
        Some(ThreadHistoryDto { channel_id: id.clone(), parent_channel_id: record.parent.clone(), rev: s.thread_rev, anchor, messages: channel_messages(&s, thread) })
    }
    fn mark_thread_read(&self, s: &mut State, thread: &str, message_id: &str) -> Result<(), String> {
        let Some(channel) = s.channels.get(thread) else { return Ok(()) };
        let Some(index) = channel.ui.history().iter().position(|m| m.id.0 == message_id) else { return Ok(()) };
        let marker = s.window.last_read(thread);
        if marker.and_then(|id| channel.ui.history().iter().position(|m| m.id.0 == id)).is_some_and(|previous| previous > index) || marker == Some(message_id) { return Ok(()); }
        s.window.set_last_read(thread, message_id)?;
        // The chip and the channel bar show the thread's unread state. The parent's own state does not change.
        if s.threads.get(thread).is_some_and(|record| is_shown(s, &record.parent)) { s.history_rev += 1; }
        s.rev += 1;
        self.changed.notify_one();
        Ok(())
    }

    /// Apply an update to the channel that a view selects. A view that selects no channel has no engine, so there is
    /// nothing to apply and the result is false.
    pub fn apply(&self, view: &ViewId, update: Update) -> bool {
        match self.selected_of(view) {
            Some(selected) if !selected.is_empty() => self.apply_channel(&selected, update),
            _ => false,
        }
    }
    pub fn runtime_generation(&self, view: &ViewId) -> u64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.views.get(view).map_or(0, |v| current_of(&s, v).runtime_generation)
    }
    pub fn channel_runtime_generation(&self, id: &str) -> Option<u64> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.channels.get(id).map(|c| c.runtime_generation)
    }
    pub fn apply_channel(&self, channel: &str, update: Update) -> bool { self.apply_runtime(channel, None, update) }
    pub fn apply_runtime(&self, channel: &str, generation: Option<u64>, update: Update) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let loaded = s.channels.get(channel).map(|c| c.runtime_generation);
        if generation.is_some_and(|generation| Some(generation) != loaded) {
            // The runtime of a thread is gone, but its task still waits for an end.
            if loaded.is_none() { drop(s); self.sub_agents.observe(channel, &SubAgentNote::of(&update)); }
            return false;
        }
        let memory_changed = matches!(&update, Update::MemoryChanges { .. });
        // What a notification needs, read before the update moves into the state.
        // Only a sub-agent thread turn is silent: its parent channel reports the result. A question thread notifies
        // like a channel. A thread shows its parent's name and mute flag, and carries its parent's channel id.
        let notice = s.channels.get(channel).and_then(|c| {
            let record = s.threads.get(channel);
            let sub_agent = record.is_some_and(|r| r.kind == discard_core::model::ThreadKind::SubAgent);
            let trigger = super::notify::trigger_of(&update, sub_agent)?;
            let owner = record.and_then(|r| s.channels.get(&r.parent)).unwrap_or(c);
            Some((trigger, owner.channel.clone(), c.agent.clone(), owner.muted, owner.channel_id.clone()))
        });
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
        let thread_len = s.channels.get(channel).filter(|c| c.thread_parent.is_some()).map(|c| c.ui.history().len());
        let thread_pending = s.channels.get(channel).filter(|c| c.thread_parent.is_some()).map(|c| c.ui.pending_approvals());
        // What a sub-agent task or its parent needs from this update, read before the update moves into the state.
        let note = SubAgentNote::of(&update);
        let finished;
        if !is_shown(&s, channel) {
            let marker = s.window.last_read(channel).map(str::to_owned);
            let Some(other) = s.channels.get_mut(channel) else { drop(s); self.sub_agents.observe(channel, &note); return false };
            let was_running = other.ui.turn_running();
            other.ui.apply(update);
            other.new_anchor = new_divider_anchor(other.ui.history(), marker.as_deref(), other.new_anchor.as_deref(), false);
            finished = was_running && !other.ui.turn_running();
            if memory_changed { s.memory_rev += 1; }
            if !s.channels.values().any(|c| c.ui.turn_running()) { s.confirm = None; }
            s.rev += 1;
        } else {
            if memory_changed { s.memory_rev += 1; }
            let shown = s.channels.get_mut(channel).expect("a shown channel is in the pool");
            let old_len = shown.ui.history().len();
            let was_running = shown.ui.turn_running();
            shown.ui.apply(update);
            let (grown, running) = (old_len != shown.ui.history().len(), shown.ui.turn_running());
            if super::windows::should_clear_confirm(s.channels.values().any(|c| c.ui.turn_running()), s.confirm.is_some()) { s.confirm = None; }
            s.rev += 1;
            if grown {
                let active = &s.channels[channel];
                let anchor = new_divider_anchor(active.ui.history(), s.window.last_read(&active.channel_id), active.new_anchor.as_deref(), false);
                s.channels.get_mut(channel).expect("a shown channel is in the pool").new_anchor = anchor;
                s.history_rev += 1;
            }
            finished = was_running && !running;
        }
        let id_replaced = saved.is_some();
        if let Some((old, new)) = saved {
            if s.window.last_read(channel) == Some(old.0.as_str()) { let _ = s.window.set_last_read(channel, &new.0); }
            if is_shown(&s, channel) { s.history_rev += 1; }
        }
        if let Some(before) = thread_len {
            let after = s.channels.get(channel).map(|c| c.ui.history().len());
            if after != Some(before) || id_replaced { refresh_thread(&mut s, channel); }
            // The chip shows the approvals that wait in its thread.
            else if thread_pending != s.channels.get(channel).map(|c| c.ui.pending_approvals()) && let Some(parent) = s.threads.get(channel).map(|r| r.parent.clone()) { touch_thread(&mut s, &parent, channel); }
        }
        let conversations = self.apply_thread_note(&mut s, channel, &note);
        drop(s);
        if let Some((trigger, name, agent, muted, channel_id)) = notice { self.maybe_notify(&trigger, &name, &agent, muted, &channel_id); }
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

    /// Install the notifier and the check of the desktop notifications setting.
    pub fn set_notifier(&self, notifier: Arc<dyn super::notify::Notifier>, enabled: EnabledFn) {
        self.notify.lock().unwrap_or_else(|e| e.into_inner()).sink = Some((notifier, enabled));
    }
    /// Record that a window gained or lost focus. Any window label counts.
    pub fn window_focus(&self, label: &str, focused: bool) {
        self.notify.lock().unwrap_or_else(|e| e.into_inner()).focus.set_focus(label, focused);
    }
    /// Record that a window closed. A closed window sends no focus-lost event.
    pub fn window_closed(&self, label: &str) {
        self.notify.lock().unwrap_or_else(|e| e.into_inner()).focus.window_closed(label);
    }
    /// Forget every window focus. Use it when the app hides its windows and the OS may send no focus-lost event.
    pub fn clear_window_focus(&self) {
        self.notify.lock().unwrap_or_else(|e| e.into_inner()).focus.clear();
    }
    /// Send a notification when the rules allow it. The `state` lock is not held here.
    fn maybe_notify(&self, trigger: &super::notify::Trigger, channel: &str, agent: &str, muted: bool, channel_id: &str) {
        // Cheap checks first, under the lock. The setting check reads a file and runs after the lock is released.
        let (notifier, enabled) = {
            let mut n = self.notify.lock().unwrap_or_else(|e| e.into_inner());
            let Some(sink) = n.sink.clone() else { return };
            if n.focus.app_focused() { return; }
            if let super::notify::Trigger::Approval { id, .. } = trigger && !n.seen.first(id) { return; }
            sink
        };
        if !enabled() { return; }
        let Some(notification) = super::notify::compose(trigger, channel, agent, muted, channel_id) else { return };
        notifier.notify(&notification);
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
        AppCore::new(CoreInit { state: ui, commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() }).snapshot(&ViewId::main()).prompts.remove(0)
    }
    fn core_with_channels(ids: &[&str]) -> AppCore {
        let init = |id: &str| {
            let (commands, _) = tokio::sync::mpsc::unbounded_channel();
            CoreInit { state: UiState::new(ChannelId(id.into()), vec![]), commands, project: "p".into(), channel: id.into(), agent: "a".into(), provider: "fake".into() }
        };
        let core = AppCore::new(init(ids[0]));
        for id in &ids[1..] {
            let channel = discard_core::model::Channel { id: ChannelId((*id).into()), agent_id: AgentId::new(), project_id: None, name: (*id).into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
            core.add_channel(init(id), &channel);
        }
        core
    }
    fn project(id: &str) -> discard_core::model::Project { discard_core::model::Project { id: discard_core::model::ProjectId(id.into()), name: id.into(), path: format!("/{id}") } }
    /// A core whose channels all belong to project `project`. The main view selects the first one.
    fn core_with_project_channels(project: &str, ids: &[&str]) -> AppCore {
        let init = |id: &str| {
            let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
            // The engine of the test channel never runs. Its command channel stays open, so a send succeeds.
            std::mem::forget(receiver);
            CoreInit { state: UiState::new(ChannelId(id.into()), vec![]), commands, project: project.into(), channel: id.into(), agent: "a".into(), provider: "fake".into() }
        };
        let core = AppCore::new(init(ids[0]));
        core.configure_targets(&ViewId::main(), project.into(), Path::new("/p"), "agent".into());
        for id in &ids[1..] {
            let channel = discard_core::model::Channel { id: ChannelId((*id).into()), agent_id: AgentId("agent".into()), project_id: Some(discard_core::model::ProjectId(project.into())), name: (*id).into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
            core.add_channel(init(id), &channel);
        }
        core
    }
    /// Open the view with this label on project `p`. It selects `selected`, a channel of the pool, unless that is empty.
    fn add_view(core: &AppCore, label: &str, selected: &str) -> ViewId {
        let view = ViewId::from_label(label).unwrap();
        assert!(core.open_view(view.clone(), &project("p")));
        if !selected.is_empty() { core.switch_channel(&view, selected).unwrap(); }
        view
    }
    fn main() -> ViewId { ViewId::main() }
    /// A core with the loaded channels `a` (shown) and `b` (not shown). It returns the command receiver of `b`.
    fn core_with_hidden_channel() -> (AppCore, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let init = |id: &str| {
            let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
            (CoreInit { state: UiState::new(ChannelId(id.into()), vec![]), commands, project: "p".into(), channel: id.into(), agent: "a".into(), provider: "fake".into() }, receiver)
        };
        let (first, first_receiver) = init("a");
        std::mem::forget(first_receiver);
        let core = AppCore::new(first);
        let (second, receiver) = init("b");
        core.add_channel(second, &discard_core::model::Channel { id: ChannelId("b".into()), agent_id: AgentId::new(), project_id: None, name: "b".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout });
        (core, receiver)
    }
    #[test]
    fn send_in_background_reaches_a_loaded_channel_that_no_view_shows() {
        let (core, mut receiver) = core_with_hidden_channel();
        core.send_in_background("b", "hello".into(), None, vec![]).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), Command::Send("hello".into()));
        assert!(receiver.try_recv().is_err());
        assert_eq!(core.active_channel_id(&main()), "a", "the view still shows the other channel");
        assert!(core.channel_busy_reason("b").is_some(), "the turn runs in the channel");
        let (core, mut receiver) = core_with_hidden_channel();
        let attachment = discard_core::attachments::Attachment { id: "att".into(), name: "n.txt".into(), media_type: "text/plain".into(), size: 1, path: "/n.txt".into(), text: Some("x".into()) };
        core.send_in_background("b", String::new(), None, vec![attachment.clone()]).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), Command::SendAttachments { text: String::new(), attachments: vec![attachment] });
        // The app-wide guard of the calling action does not block the send.
        let (core, mut receiver) = core_with_hidden_channel();
        core.begin_project_change().unwrap();
        core.send_in_background("b", "guarded".into(), None, vec![]).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), Command::Send("guarded".into()));
    }
    #[test]
    fn send_in_background_refuses_a_running_or_read_only_channel() {
        let refused = "This channel cannot send a message right now.";
        let (core, mut receiver) = core_with_hidden_channel();
        core.send_in_background("b", "first".into(), None, vec![]).unwrap();
        assert_eq!(core.send_in_background("b", "second".into(), None, vec![]).unwrap_err(), refused);
        assert_eq!(receiver.try_recv().unwrap(), Command::Send("first".into()));
        assert!(receiver.try_recv().is_err(), "the refused message sent no command");
        let (core, mut receiver) = core_with_hidden_channel();
        core.channel_read_only("b", true, false);
        assert_eq!(core.send_in_background("b", "text".into(), None, vec![]).unwrap_err(), refused);
        assert!(receiver.try_recv().is_err());
        assert_eq!(core.send_in_background("missing", "text".into(), None, vec![]).unwrap_err(), "The channel changed. Send the message again.");
    }
    #[test]
    fn a_background_message_author_shows_in_the_history_of_a_loaded_channel() {
        let (core, _receiver) = core_with_hidden_channel();
        let message = MessageId::new();
        let author = discard_core::model::MessageAuthor { agent_id: AgentId("agent".into()), name: "Moderator".into(), provider: "fake".into(), avatar: None };
        core.apply_channel("b", Update::ConversationMarker(Message { id: message.clone(), channel_id: ChannelId("b".into()), role: Role::Agent, blocks: vec![] }));
        core.set_message_author("b", message.clone(), author.clone());
        core.set_message_author("missing", message.clone(), author.clone());
        let state = core.state.lock().unwrap();
        assert_eq!(state.channels["b"].ui.authors().get(&message), Some(&author));
    }
    #[test]
    fn a_switch_keeps_both_channels_in_the_pool() {
        let core = core_with_channels(&["a", "b"]);
        core.switch_channel(&main(), "b").unwrap();
        assert_eq!(core.active_channel_id(&main()), "b");
        assert!(core.has_channel("a") && core.has_channel("b"));
        assert_eq!(core.loaded_channel_count(), 2);
    }
    #[test]
    fn the_no_channel_placeholder_is_never_in_the_pool() {
        let core = core_with_channels(&["", "b"]);
        assert_eq!(core.active_channel_id(&main()), "");
        assert!(!core.has_channel(""));
        assert_eq!(core.loaded_channel_count(), 1);
        core.switch_channel(&main(), "b").unwrap();
        assert_eq!(core.active_channel_id(&main()), "b");
        assert!(!core.has_channel(""));
        assert_eq!(core.loaded_channel_count(), 1);
    }
    #[test]
    fn a_missing_view_is_an_error_and_is_not_created() {
        let core = core_with_channels(&["a", "b"]);
        let gone = ViewId::project(&discard_core::model::ProjectId("p".into()));
        assert_eq!(core.switch_channel(&gone, "b").unwrap_err(), "This window is closed.");
        assert_eq!(core.can_switch(&gone).unwrap_err(), "This window is closed.");
        assert_eq!(core.begin_project_navigation(&gone).unwrap_err(), "This window is closed.");
        assert_eq!(core.set_panels(&gone, 250.0, 250.0).unwrap_err(), "This window is closed.");
        assert_eq!(core.mark_read(&gone, "m").unwrap_err(), "This window is closed.");
        assert!(core.open_thread(&gone, "t").is_err());
        core.notice(&gone, "lost".into());
        core.toggle_branch(&gone, "k".into());
        core.close_thread(&gone);
        assert_eq!(core.active_channel_id(&gone), "");
        assert_eq!(core.snapshot(&gone).channel_id, "");
        assert!(core.snapshot(&gone).notice.is_none());
        assert_eq!(core.view_ids(), vec![ViewId::main()]);
        assert!(!core.has_view(&gone));
        assert_eq!(core.active_channel_id(&ViewId::main()), "a");
        assert!(core.snapshot(&ViewId::main()).notice.is_none());
    }
    #[test]
    fn views_select_independently_and_share_the_pool() {
        let core = core_with_channels(&["a"]);
        let (commands, mut received) = tokio::sync::mpsc::unbounded_channel();
        let channel = discard_core::model::Channel { id: ChannelId("b".into()), agent_id: AgentId::new(), project_id: None, name: "b".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_channel(CoreInit { state: UiState::new(ChannelId("b".into()), vec![]), commands, project: "p".into(), channel: "b".into(), agent: "a".into(), provider: "fake".into() }, &channel);
        let other = add_view(&core, "project-p", "a");
        assert!(core.send_to(Some("b"), "nobody shows b".into()).is_err(), "a channel that no view shows takes no message");
        core.switch_channel(&other, "b").unwrap();
        assert_eq!(core.snapshot(&other).channel_id, "b");
        assert_eq!(core.snapshot(&main()).channel_id, "a");
        assert_eq!(core.history_snapshot(&other).channel_id, "b");
        assert_eq!(core.history_snapshot(&main()).channel_id, "a");
        assert_eq!(core.loaded_channel_count(), 2);
        core.send_to(Some("b"), "hello".into()).unwrap();
        assert!(matches!(received.try_recv(), Ok(Command::Send(text)) if text == "hello"));
        assert!(core.snapshot(&other).turn_running && !core.snapshot(&main()).turn_running);
        core.switch_channel(&main(), "b").unwrap();
        assert!(core.snapshot(&main()).turn_running, "a second view of the channel shows the same turn");
        core.switch_channel(&other, "a").unwrap();
        assert!(core.snapshot(&main()).turn_running && !core.snapshot(&other).turn_running);
    }
    #[test]
    fn each_view_owns_its_no_channel_placeholder() {
        let core = core_with_channels(&["", "b"]);
        let other = add_view(&core, "project-p", "");
        core.notice(&main(), "main only".into());
        assert_eq!(core.snapshot(&main()).notice.as_deref(), Some("main only"));
        assert_eq!(core.snapshot(&other).notice, None);
        core.configure_targets(&other, "p".into(), Path::new("/p"), "agent-p".into());
        assert_eq!(core.active_agent_id(&other), "agent-p");
        assert_eq!(core.active_agent_id(&main()), "");
        assert!(!core.has_channel(""));
    }
    #[test]
    fn the_navigation_guard_belongs_to_one_view() {
        let core = core_with_channels(&["a", "b"]);
        let other = add_view(&core, "project-p", "b");
        core.begin_project_navigation(&main()).unwrap();
        assert!(core.begin_project_navigation(&main()).is_err(), "navigation of one view is serialized");
        assert!(core.snapshot(&main()).archive_busy);
        assert!(!core.snapshot(&other).archive_busy, "another view stays free");
        assert!(core.switch_channel(&main(), "b").is_err());
        core.switch_channel(&other, "a").unwrap();
        core.begin_project_navigation(&other).unwrap();
        core.end_project_navigation(&other);
        assert!(core.begin_project_change().is_err(), "a project change waits for every navigation");
        core.end_project_navigation(&main());
        core.begin_project_change().unwrap();
        // The app-wide guard shows in every view and blocks navigation in every view.
        assert!(core.snapshot(&main()).archive_busy && core.snapshot(&other).archive_busy);
        assert!(core.begin_project_navigation(&other).is_err());
        core.end_project_change();
        assert!(!core.snapshot(&main()).archive_busy && !core.snapshot(&other).archive_busy);
    }
    #[test]
    fn only_the_main_view_saves_its_layout() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_channels(&["a"]);
        core.configure_read_marker(&main(), dir.path(), "a");
        let other = add_view(&core, "project-p", "a");
        core.set_panels(&other, 300.0, 1000.0).unwrap();
        core.set_member_list_visible(&other, false).unwrap();
        let panels = core.snapshot(&other).panels;
        assert_eq!((panels.channel_list, panels.member_list), (300.0, discard_core::window_state::MAX_PANEL_WIDTH));
        assert!(!core.snapshot(&other).member_list_visible);
        assert!(core.snapshot(&main()).member_list_visible, "the main view keeps its own layout");
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).state().panels, PanelWidths::default());
        core.set_panels(&main(), 260.0, 270.0).unwrap();
        assert_eq!(core.snapshot(&main()).panels.channel_list, 260.0);
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).state().panels, PanelWidths { channel_list: 260.0, member_list: 270.0 });
    }
    #[test]
    fn a_thread_panel_belongs_to_its_view() {
        let core = core_with_channels(&["a"]);
        let other = add_view(&core, "project-p", "a");
        core.state.lock().unwrap().views.get_mut(&other).unwrap().open_thread = Some("t".into());
        assert_eq!(core.open_thread_id(&other).as_deref(), Some("t"));
        assert_eq!(core.open_thread_id(&main()), None);
        core.close_thread(&main());
        assert_eq!(core.open_thread_id(&other).as_deref(), Some("t"));
        core.close_thread(&other);
        assert_eq!(core.open_thread_id(&other), None);
    }
    #[test]
    fn removing_the_agent_of_the_selected_channel_leaves_an_empty_snapshot() {
        let core = core_with_channels(&["a", "b"]);
        core.configure_targets(&main(), "".into(), Path::new(""), "agent-a".into());
        core.remove_agent_channels("agent-a");
        let snapshot = core.snapshot(&main());
        assert_eq!(snapshot.channel_id, "");
        assert!(snapshot.composer_disabled);
        assert_eq!(core.active_channel_id(&main()), "");
        assert!(!core.has_channel("a") && !core.has_channel(""));
        // The view writes to its fresh placeholder again.
        core.notice(&main(), "after".into());
        assert_eq!(core.snapshot(&main()).notice.as_deref(), Some("after"));
    }
    #[test]
    fn removing_an_agent_resets_only_the_views_that_show_it() {
        let core = core_with_channels(&["a", "b"]);
        let other = add_view(&core, "project-p", "b");
        core.configure_targets(&main(), "".into(), Path::new(""), "agent-a".into());
        core.remove_agent_channels("agent-a");
        assert_eq!(core.active_channel_id(&main()), "");
        assert_eq!(core.active_channel_id(&other), "b");
        assert_eq!(core.snapshot(&other).channel_id, "b");
    }
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "once")]
    fn a_re_key_onto_a_pooled_id_is_loud() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_channels(&["a", "b"]);
        core.configure_read_marker(&main(), dir.path(), "b");
    }
    /// A core whose channel "a" has a command receiver. The main view and a project view both select "a".
    fn two_views_on_a() -> (AppCore, ViewId, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let (commands, received) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId("a".into()), vec![]), commands, project: "p".into(), channel: "a".into(), agent: "a".into(), provider: "fake".into() });
        core.configure_targets(&main(), "p".into(), Path::new("/p"), "agent".into());
        let other = add_view(&core, "project-p", "a");
        (core, other, received)
    }
    #[test]
    fn views_select_independently() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = ViewId::project(&discard_core::model::ProjectId("p".into()));
        assert!(core.open_view(other.clone(), &project("p")));
        core.switch_channel(&other, "b").unwrap();
        assert_eq!(core.active_channel_id(&ViewId::main()), "a");
        assert_eq!(core.snapshot(&other).channel_id, "b");
        assert_eq!(core.snapshot(&ViewId::main()).channel_id, "a");
    }
    #[test]
    fn a_new_view_shows_its_project_and_the_saved_layout() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_project_channels("p", &["a"]);
        core.configure_read_marker(&main(), dir.path(), "a");
        core.set_panels(&main(), 260.0, 270.0).unwrap();
        core.set_member_list_visible(&main(), false).unwrap();
        let other = ViewId::project(&discard_core::model::ProjectId("p".into()));
        assert!(core.open_view(other.clone(), &project("p")));
        let snapshot = core.snapshot(&other);
        assert_eq!((snapshot.project.as_str(), snapshot.project_id.as_str(), snapshot.project_path.as_str()), ("p", "p", "/p"));
        assert_eq!((snapshot.panels.channel_list, snapshot.panels.member_list), (260.0, 270.0));
        assert!(!snapshot.member_list_visible);
        assert_eq!(core.current_project(&other).id, discard_core::model::ProjectId("p".into()));
        assert_eq!(core.view_ids(), vec![main(), other]);
    }
    #[test]
    fn opening_a_view_that_exists_changes_nothing() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = add_view(&core, "project-p", "b");
        assert!(!core.open_view(other.clone(), &project("q")), "the view exists");
        assert_eq!(core.active_channel_id(&other), "b");
        assert_eq!(core.current_project(&other).id.0, "p");
        assert!(!core.open_view(main(), &project("p")), "the main view exists");
        assert_eq!(core.view_ids().len(), 2);
    }
    #[test]
    fn a_dropped_view_stops_no_turn_and_unloads_no_channel() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = ViewId::project(&discard_core::model::ProjectId("p".into()));
        core.open_view(other.clone(), &project("p"));
        core.switch_channel(&other, "b").unwrap();
        core.send_to(Some("b"), "hello".into()).unwrap();
        core.drop_view(&other);
        assert!(core.channel_running("b"));
        assert_eq!(core.loaded_channel_count(), 2);
        assert!(!core.has_view(&other));
        core.drop_view(&ViewId::main());
        assert!(core.has_view(&ViewId::main()));
        // A call for the dropped view does not bring it back.
        assert_eq!(core.switch_channel(&other, "a").unwrap_err(), "This window is closed.");
        assert_eq!(core.view_ids(), vec![ViewId::main()]);
    }
    #[test]
    fn dropping_a_view_in_the_middle_of_a_navigation_leaves_no_guard_behind() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = add_view(&core, "project-p", "b");
        core.begin_project_navigation(&other).unwrap();
        core.drop_view(&other);
        // The guard went with the view. The app-wide guard is free, and the late end of the navigation is harmless.
        core.end_project_navigation(&other);
        core.begin_project_change().unwrap();
        core.end_project_change();
        assert_eq!(core.view_ids(), vec![main()]);
        assert!(core.begin_project_navigation(&main()).is_ok(), "the main view navigates as before");
        // The same label opens a fresh view that is not busy.
        let again = add_view(&core, "project-p", "a");
        assert!(core.begin_project_navigation(&again).is_ok());
    }
    #[test]
    fn a_view_that_reopens_under_the_same_label_survives_a_late_refresh_end() {
        let core = core_with_project_channels("p", &["a"]);
        let other = add_view(&core, "project-p", "");
        core.drop_view(&other);
        core.open_view(other.clone(), &project("p"));
        // The refresh of the old view ends after the new view opened: its counter is 0 and stays 0.
        core.finish_branch_refresh(&other);
        assert!(!core.branch_refresh_pending(&other));
        core.state.lock().unwrap().views.get_mut(&other).unwrap().branch_refreshes_pending = 1;
        assert!(core.branch_refresh_pending(&other));
        core.finish_branch_refresh(&other);
        assert!(!core.branch_refresh_pending(&other));
        core.drop_view(&other);
        core.finish_branch_refresh(&other);
    }
    #[test]
    fn views_selecting_lists_the_views_that_show_a_channel() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = add_view(&core, "project-p", "a");
        assert_eq!(core.views_selecting("a"), vec![main(), other.clone()]);
        core.switch_channel(&other, "b").unwrap();
        assert_eq!(core.views_selecting("a"), vec![main()]);
        assert_eq!(core.views_selecting("b"), vec![other]);
        assert!(core.views_selecting("").is_empty(), "no view shows the empty ID");
        assert!(core.views_selecting("missing").is_empty());
    }
    #[tokio::test]
    async fn a_runtime_update_refreshes_the_branches_of_every_view_that_shows_the_channel() {
        let core = Arc::new(core_with_project_channels("p", &["a", "b"]));
        let on_a = add_view(&core, "project-p", "a");
        let on_b = add_view(&core, "project-q", "b");
        let generation = |view: &ViewId| core.state.lock().unwrap().views[view].branch_generation;
        let before = (generation(&main()), generation(&on_a), generation(&on_b));
        core.refresh_branches_selecting("a");
        assert_eq!((generation(&main()), generation(&on_a), generation(&on_b)), (before.0 + 1, before.1 + 1, before.2), "the view of another channel is left alone");
    }
    #[test]
    fn the_focused_view_is_the_main_view_until_a_window_takes_focus() {
        let core = core_with_project_channels("p", &["a"]);
        add_view(&core, "project-p", "a");
        assert_eq!(core.focused_view(), main());
    }
    #[test]
    fn the_confirmation_shows_in_the_focused_view_only() {
        let core = core_with_project_channels("p", &["a"]);
        let p_view = add_view(&core, "project-p", "a");
        core.set_focused_view(&p_view);
        assert_eq!(core.focused_view(), p_view);
        core.ask(super::super::plan::Confirm::Quit);
        assert!(core.snapshot(&p_view).banners.confirm.is_some());
        assert!(core.snapshot(&main()).banners.confirm.is_none());
        // The question follows the focus.
        core.set_focused_view(&main());
        assert!(core.snapshot(&p_view).banners.confirm.is_none());
        assert!(core.snapshot(&main()).banners.confirm.is_some());
        // A view that closes gives the focus back to the main view.
        core.set_focused_view(&p_view);
        core.drop_view(&p_view);
        assert_eq!(core.focused_view(), main());
        assert!(core.snapshot(&main()).banners.confirm.is_some());
    }
    #[test]
    fn a_view_that_does_not_exist_takes_no_focus() {
        let core = core_with_project_channels("p", &["a"]);
        let gone = ViewId::from_label("project-gone").unwrap();
        core.set_focused_view(&gone);
        assert_eq!(core.focused_view(), main());
        // A closed view that opens again with the same label does not inherit the old focus.
        let p_view = add_view(&core, "project-p", "a");
        core.set_focused_view(&p_view);
        core.drop_view(&p_view);
        add_view(&core, "project-p", "a");
        assert_eq!(core.focused_view(), main());
    }
    #[test]
    fn a_change_of_focus_wakes_the_pump_and_a_repeat_does_not() {
        let core = core_with_project_channels("p", &["a"]);
        let p_view = add_view(&core, "project-p", "a");
        let rev = || core.state.lock().unwrap().rev;
        let before = rev();
        core.set_focused_view(&p_view);
        assert_eq!(rev(), before + 1);
        core.set_focused_view(&p_view);
        assert_eq!(rev(), before + 1);
    }
    #[test]
    fn a_decision_in_one_view_clears_the_prompt_in_the_other() { // Review Focus 1
        let (core, other, mut received) = two_views_on_a();
        let approval_id = ApprovalId::new();
        core.apply_channel("a", Update::Event(Box::new(discard_core::event::Event {
            id: discard_core::event::EventId::new(),
            session_id: discard_core::event::SessionId::new(),
            seq: 1,
            raw: None,
            kind: discard_core::event::EventKind::ApprovalRequest {
                kind: discard_core::approval::ApprovalKind::ProviderTool,
                approval_id: approval_id.clone(),
                call_id: "shell".into(),
                tool: "Bash".into(),
                input: serde_json::json!({"command":"cargo test -p x"}),
                paths: vec![],
            },
        })));
        for view in [main(), other.clone()] {
            let snapshot = core.snapshot(&view);
            assert_eq!(snapshot.prompts.len(), 1, "{view:?}");
            assert_eq!((snapshot.prompts[0].view.as_str(), snapshot.pending_approvals), ("pending", 1), "{view:?}");
        }
        core.decide_in(Some("a"), approval_id.to_string(), "approve".into(), None);
        assert!(matches!(received.try_recv(), Ok(Command::Decide { approval_id: id, .. }) if id == approval_id));
        assert!(received.try_recv().is_err(), "one decision sends one command");
        // The engine reports the decision once. Both views read the same channel.
        core.apply_channel("a", Update::ApprovalState { approval_id, state: discard_core::approval::ApprovalState::Decided(Decision::Approve), decided_by: discard_core::model::DecidedBy::User });
        for view in [main(), other] {
            let snapshot = core.snapshot(&view);
            assert_eq!(snapshot.pending_approvals, 0, "{view:?}");
            assert!(snapshot.prompts.iter().all(|prompt| prompt.view != "pending"), "{view:?}");
        }
    }
    #[test]
    fn a_missing_view_gets_a_minimal_snapshot_without_a_view_or_a_channel_list() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let theme = core.state.lock().unwrap().theme.clone();
        core.set_theme(ThemeDto { font_scale: 1.5, ..theme });
        let gone = ViewId::project(&discard_core::model::ProjectId("gone".into()));
        let snapshot = core.snapshot(&gone);
        assert_eq!((snapshot.channel_id.as_str(), snapshot.project_id.as_str()), ("", ""));
        assert!(snapshot.composer_disabled);
        assert!(snapshot.channels.is_empty() && snapshot.branches.is_empty() && snapshot.prompts.is_empty() && snapshot.slots.is_empty());
        assert!(snapshot.open_thread.is_none() && snapshot.notice.is_none());
        assert_eq!(snapshot.theme.font_scale, 1.5, "the window still gets the app theme");
        assert_eq!(snapshot.rev, core.snapshot(&main()).rev);
        assert!(!core.has_view(&gone), "a snapshot does not create the view");
    }
    #[test]
    fn replacing_the_project_of_a_view_keeps_the_state_of_a_channel_that_another_view_shows() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = add_view(&core, "project-p", "b");
        core.notice(&main(), "main only".into());
        // A runtime for "a" is launched while main still shows "a". The pool keeps the loaded state.
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let channel = discard_core::model::Channel { id: ChannelId("a".into()), agent_id: AgentId("agent".into()), project_id: Some(discard_core::model::ProjectId("p".into())), name: "a".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        let installed = core.replace_project(&other, CoreInit { state: UiState::new(channel.id.clone(), vec![]), commands, project: "p".into(), channel: "a".into(), agent: "a".into(), provider: "fake".into() }, &project("p"), &channel);
        assert!(!installed, "the pool held the channel, so the new runtime was dropped and the caller is told");
        assert_eq!(core.active_channel_id(&other), "a");
        assert_eq!(core.active_channel_id(&main()), "a");
        assert_eq!(core.snapshot(&main()).notice.as_deref(), Some("main only"), "the loaded channel kept its state");
        assert_eq!(core.snapshot(&other).notice.as_deref(), Some("main only"));
        assert!(core.has_channel("a") && !core.has_channel("b"), "the channel that no view shows is gone");
        assert_eq!(core.loaded_channel_count(), 1);
    }
    #[test]
    fn replacing_the_project_of_a_view_reports_whether_its_runtime_was_installed() {
        let core = core_with_project_channels("p", &["a"]);
        let other = add_view(&core, "project-p", "a");
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let fresh = discard_core::model::Channel { id: ChannelId("c".into()), agent_id: AgentId("agent".into()), project_id: project_of("p"), name: "c".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        assert!(core.replace_project(&other, CoreInit { state: UiState::new(fresh.id.clone(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "fake".into() }, &project("p"), &fresh), "a channel that no view loaded takes the runtime");
        assert!(core.has_channel("c") && core.has_channel("a"), "main still shows a");
        let gone = ViewId::project(&discard_core::model::ProjectId("gone".into()));
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        assert!(!core.replace_project(&gone, CoreInit { state: UiState::new(fresh.id.clone(), vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "fake".into() }, &project("p"), &fresh), "a view that does not exist installs nothing");
        assert!(!core.has_view(&gone));
    }
    #[test]
    fn clearing_the_project_of_a_view_unloads_no_channel_that_another_view_selects() {
        let core = core_with_project_channels("p", &["a", "b"]);
        let other = add_view(&core, "project-p", "a");
        core.notice(&main(), "kept".into());
        core.clear_project(&other);
        assert_eq!(core.active_channel_id(&other), "");
        assert_eq!(core.current_project(&other).id.0, "");
        assert!(core.has_channel("a"), "the main view still shows a");
        assert!(!core.has_channel("b"), "no view shows b");
        assert_eq!(core.active_channel_id(&main()), "a");
        assert_eq!(core.snapshot(&main()).notice.as_deref(), Some("kept"));
        assert_eq!(core.current_project(&main()).id.0, "p");
    }
    #[test]
    fn clearing_the_slots_clears_the_cache_of_every_view() {
        let core = core_with_project_channels("p", &["a"]);
        let other = add_view(&core, "project-p", "a");
        let state = serde_json::json!({"k": 1});
        let cache_of = |view: &ViewId| core.state.lock().unwrap().views[view].slots.clone();
        assert!(!Arc::ptr_eq(&cache_of(&main()), &cache_of(&other)), "each view has its own cache");
        let renders = |view: &ViewId| { let mut calls = 0; cache_of(view).lock().unwrap().get_or_render(&state, || { calls += 1; BTreeMap::new() }); calls };
        assert_eq!((renders(&main()), renders(&other)), (1, 1));
        assert_eq!((renders(&main()), renders(&other)), (0, 0), "the caches hold their entries");
        core.clear_slots();
        assert_eq!((renders(&main()), renders(&other)), (1, 1), "both caches render again");
    }
    #[test]
    fn a_view_that_selects_no_channel_has_nothing_to_apply() {
        let core = core_with_channels(&["", "b"]);
        assert!(!core.apply(&main(), Update::Notice("lost".into())));
        assert_eq!(core.snapshot(&main()).notice, None, "no engine writes to a placeholder");
        let gone = ViewId::project(&discard_core::model::ProjectId("gone".into()));
        assert!(!core.apply(&gone, Update::Notice("lost".into())));
        assert!(!core.has_view(&gone));
        core.switch_channel(&main(), "b").unwrap();
        core.apply(&main(), Update::Notice("kept".into()));
        assert_eq!(core.snapshot(&main()).notice.as_deref(), Some("kept"));
    }
    fn project_of(id: &str) -> Option<discard_core::model::ProjectId> { Some(discard_core::model::ProjectId(id.into())) }
    /// A channel of `project` with no engine. Its command channel stays open.
    fn add_project_channel(core: &AppCore, project: &str, id: &str) {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        let channel = discard_core::model::Channel { id: ChannelId(id.into()), agent_id: AgentId("agent".into()), project_id: project_of(project), name: id.into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_channel(CoreInit { state: UiState::new(ChannelId(id.into()), vec![]), commands, project: project.into(), channel: id.into(), agent: "a".into(), provider: "fake".into() }, &channel);
    }
    fn ask_approval(core: &AppCore, channel: &str) {
        core.apply_channel(channel, Update::Event(Box::new(discard_core::event::Event {
            id: discard_core::event::EventId::new(), session_id: discard_core::event::SessionId::new(), seq: 1, raw: None,
            kind: discard_core::event::EventKind::ApprovalRequest {
                kind: discard_core::approval::ApprovalKind::ProviderTool, approval_id: ApprovalId::new(), call_id: "shell".into(), tool: "Bash".into(),
                input: serde_json::json!({"command":"cargo test -p x"}), paths: vec![],
            },
        })));
    }
    #[test]
    fn a_target_outside_the_project_routes_to_the_main_view_in_the_core() {
        let core = core_with_project_channels("p", &["a"]);
        let p_view = add_view(&core, "project-p", "a");
        let (p, q) = (project_of("p").unwrap(), project_of("q").unwrap());
        assert_eq!(core.route(&p_view, Some(&p)), p_view);
        assert_eq!(core.route(&p_view, Some(&q)), main());
        assert_eq!(core.route(&p_view, None), main(), "a direct message");
        assert_eq!(core.route(&main(), Some(&q)), main());
        assert_eq!(core.route(&main(), None), main());
        assert_eq!(core.route(&main(), Some(&p)), main());
    }
    #[test]
    fn a_project_view_lists_only_its_project() {
        let core = core_with_project_channels("p", &["a", "b"]);
        add_project_channel(&core, "q", "x");
        // A direct message has no project.
        let direct = discard_core::model::Channel { id: ChannelId("dm".into()), agent_id: AgentId::new(), project_id: None, name: "dm".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        core.add_channel(CoreInit { state: UiState::new(ChannelId("dm".into()), vec![]), commands, project: String::new(), channel: "dm".into(), agent: "a".into(), provider: "fake".into() }, &direct);
        let p_view = add_view(&core, "project-p", "b");
        let listed = |view: &ViewId| core.snapshot(view).channels.into_iter().map(|c| (c.id, c.project_id)).collect::<Vec<_>>();
        assert_eq!(listed(&p_view), vec![("a".to_string(), Some("p".to_string())), ("b".to_string(), Some("p".to_string()))]);
        let all = listed(&main());
        assert!(all.contains(&("x".to_string(), Some("q".to_string()))) && all.contains(&("a".to_string(), Some("p".to_string()))) && all.contains(&("dm".to_string(), None)), "{all:?}");
        assert_eq!(all.len(), 4, "the main view lists every loaded channel");
    }
    #[test]
    fn a_project_view_lists_only_the_thread_approvals_of_its_project() {
        let core = core_with_project_channels("p", &["a"]);
        add_project_channel(&core, "q", "x");
        let p_view = add_view(&core, "project-p", "a");
        for (thread, parent, project) in [("t-p", "a", "p"), ("t-q", "x", "q")] {
            let info = discard_core::model::ThreadInfo { channel_id: ChannelId(thread.into()), parent_channel_id: ChannelId(parent.into()), anchor_message_id: None, kind: ThreadKind::SubAgent, followed: false, name: thread.into(), state: None, agent_id: AgentId("agent".into()), agent_name: "a".into() };
            let channel = discard_core::model::Channel { id: ChannelId(thread.into()), agent_id: AgentId("agent".into()), project_id: project_of(project), name: thread.into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
            let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
            std::mem::forget(receiver);
            core.add_thread(CoreInit { state: UiState::new(ChannelId(thread.into()), vec![]), commands, project: project.into(), channel: thread.into(), agent: "a".into(), provider: "fake".into() }, &channel, &info);
            ask_approval(&core, thread);
        }
        let waiting = |view: &ViewId| core.snapshot(view).thread_approvals.into_iter().map(|a| a.thread_id).collect::<Vec<_>>();
        assert_eq!(waiting(&p_view), vec!["t-p".to_string()]);
        assert_eq!(waiting(&main()), vec!["t-p".to_string(), "t-q".to_string()]);
    }
    #[test]
    fn a_read_in_one_view_clears_unread_in_the_other() { // Review Focus 4
        let dir = tempfile::tempdir().unwrap();
        let history: Vec<_> = ["one", "two"].into_iter().map(|id| Message { id: MessageId(id.into()), channel_id: ChannelId("a".into()), role: Role::Agent, blocks: vec![] }).collect();
        StateStore::open(&dir.path().join("state.toml")).set_last_read("a", "one").unwrap();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId("a".into()), history), commands, project: "p".into(), channel: "a".into(), agent: "a".into(), provider: "fake".into() });
        core.configure_targets(&main(), "p".into(), Path::new("/p"), "agent".into());
        core.configure_read_marker(&main(), dir.path(), "a");
        add_project_channel(&core, "p", "b");
        let p_view = add_view(&core, "project-p", "a");
        core.switch_channel(&main(), "b").unwrap();
        let unread = |view: &ViewId| core.snapshot(view).channels.into_iter().find(|c| c.id == "a").unwrap().unread;
        assert!(unread(&main()) && unread(&p_view), "a has a message that nobody read");
        assert!(core.snapshot(&p_view).unread);
        core.mark_read_in(Some("a"), "two").unwrap();
        assert!(!unread(&main()), "the read in the project view cleared the marker in the main view");
        assert!(!core.snapshot(&p_view).unread);
    }
    fn core_with_plugin(init: &str) -> (AppCore, tempfile::TempDir) {
        use discard_lua::{PluginLimits, PluginManager, keystore::StaticKeyStore, ledger::Ledger, snapshot::Snapshot};
        let manifest = "name = \"notes\"\nversion = \"1\"\napi = \"1\"\n[capabilities]\nui = true\n";
        let snapshot = Snapshot::from_files([("plugin.toml".to_string(), manifest.as_bytes().to_vec()), ("init.lua".to_string(), init.as_bytes().to_vec())].into());
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(Mutex::new(PluginManager::new(PluginLimits::default())));
        manager.lock().unwrap().load(snapshot).unwrap();
        let ledger = Ledger::open(&dir.path().join("plugins.db"), Arc::new(StaticKeyStore([5; 32]))).unwrap();
        let service = PluginService::new(dir.path(), ledger, manager, false).unwrap();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId::new(), vec![]), commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() });
        core.set_plugins(Arc::new(service));
        (core, dir)
    }
    #[test]
    fn plugin_commands_list_and_run_in_the_active_channel() {
        let (core, _dir) = core_with_plugin(r#"discard.command("echo", function(args) return "got " .. args end, { description = "  ", args = "<text>" })"#);
        let commands = core.plugin_commands();
        assert_eq!(commands, vec![discard_core::slash::PluginCommand { plugin: "notes".into(), name: "echo".into(), description: None, args: Some("<text>".into()) }]);
        let id = core.active_channel_id(&ViewId::main());
        assert_eq!(core.run_plugin_command(&id, "echo", "hi"), Some(Ok(Some("got hi".into()))));
        assert_eq!(core.run_plugin_command(&id, "nope", ""), None);
        assert_eq!(core.run_plugin_command("missing", "echo", "hi"), Some(Err("The channel is not open.".into())));
    }
    #[test]
    fn command_channel_covers_the_active_channel_only_when_it_is_known() {
        let (core, _dir) = core_with_plugin("");
        assert_eq!(core.command_channel(&core.active_channel_id(&ViewId::main())), Some(CommandChannel { turn_running: false, archived: false, is_thread: false }));
        assert_eq!(core.command_channel("missing"), None);
        assert_eq!(core.command_channel(""), None);
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
        assert_eq!(serde_json::to_value(core.snapshot(&ViewId::main())).unwrap()["autorun"], serde_json::json!({"state": "unavailable", "reason": "r"}));
    }

    #[test]
    fn snapshot_reports_how_the_response_started() {
        use discard_core::ui_state::ResumeState;
        let snapshot = |ui: UiState| {
            let (commands, _) = tokio::sync::mpsc::unbounded_channel();
            let core = AppCore::new(CoreInit { state: ui, commands, project: "P".into(), channel: "C".into(), agent: "A".into(), provider: "fake".into() });
            serde_json::to_value(core.snapshot(&ViewId::main())).unwrap()["resumeState"].clone()
        };
        let mut ui = UiState::new(ChannelId::new(), vec![]);
        assert_eq!(snapshot(ui.clone()), serde_json::Value::Null);
        ui.apply(Update::ResumeState(ResumeState::Resumed));
        assert_eq!(snapshot(ui.clone()), serde_json::json!({"kind": "resumed", "agent": "A"}));
        ui.apply(Update::Responding(discard_core::model::MessageAuthor { agent_id: discard_core::model::AgentId::new(), name: "Reviewer".into(), provider: "fake".into(), avatar: None }));
        ui.apply(Update::ResumeState(ResumeState::Replay { reason: "the model changed.".into() }));
        assert_eq!(snapshot(ui.clone()), serde_json::json!({"kind": "replay", "reason": "the model changed.", "agent": "Reviewer"}));
        ui.user_sent("next");
        assert_eq!(snapshot(ui), serde_json::Value::Null);
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
        assert!(core.snapshot(&ViewId::main()).autorun.is_none());
        assert_eq!(serde_json::to_value(core.snapshot(&ViewId::main())).unwrap()["autorun"], serde_json::Value::Null);
    }

    #[test]
    fn direct_conversation_context_does_not_use_selected_sidebar_project() {
        let channel = ChannelId::new();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId(String::new()), vec![]), commands, project: "Selected sidebar project".into(), channel: "Direct".into(), agent: "A".into(), provider: "fake".into() });
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let direct = discard_core::model::Channel { id: channel.clone(), agent_id: AgentId::new(), project_id: None, name: "Direct".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_channel(CoreInit { state: UiState::new(channel.clone(), vec![]), commands, project: String::new(), channel: "Direct".into(), agent: "A".into(), provider: "fake".into() }, &direct);
        core.switch_channel(&ViewId::main(), &channel.0).unwrap();
        assert_eq!(core.active_context(&ViewId::main()), (direct.agent_id, None));
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
        core.configure_read_marker(&ViewId::main(), dir.path(), "channel");
        let before = core.snapshot(&ViewId::main());
        assert!(before.unread);
        assert_eq!(before.new_anchor_id.as_deref(), Some("two"));
        core.mark_read(&ViewId::main(), "two").unwrap();
        let after = core.snapshot(&ViewId::main());
        assert!(!after.unread);
        assert!(after.new_anchor_id.is_none());
        assert!(after.rev > before.rev);
    }

    fn muted_fixture() -> (tempfile::TempDir, AppCore, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let dir = tempfile::tempdir().unwrap();
        let channel = ChannelId("channel".into());
        let history: Vec<_> = ["one", "two"].into_iter().map(|id| Message { id: MessageId(id.into()), channel_id: channel.clone(), role: Role::Agent, blocks: vec![] }).collect();
        let mut store = StateStore::open(&dir.path().join("state.toml"));
        store.set_last_read("channel", "one").unwrap();
        let (commands, received) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, history), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        core.configure_read_marker(&ViewId::main(), dir.path(), "channel");
        (dir, core, received)
    }

    #[test]
    fn a_muted_channel_shows_no_unread_marker_anywhere_and_unmuting_shows_it_again() {
        let (_dir, core, _received) = muted_fixture();
        let unread = |core: &AppCore| { let snapshot = core.snapshot(&ViewId::main()); (snapshot.unread, snapshot.channels[0].unread, snapshot.channels[0].muted) };
        assert_eq!(unread(&core), (true, true, false));
        let before = core.snapshot(&ViewId::main()).rev;
        core.set_channel_muted("channel", true);
        assert_eq!(unread(&core), (false, false, true), "the row and the project rail read the channel unread flag");
        assert!(core.snapshot(&ViewId::main()).rev > before);
        // Mute changes no read marker: unmuting shows the unread marker again.
        core.set_channel_muted("channel", false);
        assert_eq!(unread(&core), (true, true, false));
        core.set_channel_muted("unknown", true);
        assert_eq!(unread(&core), (true, true, false), "an unknown channel changes nothing");
    }

    #[test]
    fn a_muted_channel_keeps_its_pending_approval_count() {
        use discard_core::event::{Event, EventId, EventKind, SessionId};
        let (_dir, core, _received) = muted_fixture();
        core.apply(&ViewId::main(), Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 1, raw: None,
            kind: EventKind::ApprovalRequest { kind: Default::default(), paths: vec![], approval_id: ApprovalId::new(), call_id: "c".into(), tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) } })));
        core.set_channel_muted("channel", true);
        let snapshot = core.snapshot(&ViewId::main());
        assert_eq!((snapshot.pending_approvals, snapshot.channels[0].pending_approvals, snapshot.channels[0].unread), (1, 1, false));
    }

    #[derive(Default)]
    struct RecordingNotifier(Mutex<Vec<super::super::notify::Notification>>);
    impl super::super::notify::Notifier for RecordingNotifier {
        fn notify(&self, notification: &super::super::notify::Notification) { self.0.lock().unwrap().push(notification.clone()); }
    }
    fn notifying_fixture(enabled: bool) -> (tempfile::TempDir, AppCore, Arc<RecordingNotifier>) {
        let (dir, core, _received) = muted_fixture();
        let notifier = Arc::new(RecordingNotifier::default());
        core.set_notifier(notifier.clone(), Arc::new(move || enabled));
        (dir, core, notifier)
    }
    fn approval_request(id: &str) -> Update { Update::UserApprovalNeeded { approval_id: ApprovalId(id.into()), tool: "Bash".into() } }
    fn approval_event(id: &str) -> Update {
        use discard_core::event::{Event, EventId, EventKind, SessionId};
        Update::Event(Box::new(Event { id: EventId::new(), session_id: SessionId::new(), seq: 1, raw: None,
            kind: EventKind::ApprovalRequest { kind: Default::default(), paths: vec![], approval_id: ApprovalId(id.into()), call_id: "c".into(), tool: "Bash".into(), input: serde_json::json!({"command": "ls"}) } }))
    }
    fn finished() -> Update { Update::TurnFinished(discard_core::driver::TurnOutcome::Completed) }

    #[test]
    fn an_approval_notifies_when_no_window_is_focused() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", approval_request("ap-1"));
        let sent = notifier.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].title, "c · a");
        assert_eq!(sent[0].body, "Needs approval: Bash");
    }

    #[test]
    fn an_auto_approved_request_does_not_notify() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", approval_event("ap-1"));
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_notifies_while_a_window_is_focused() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.window_focus("main", true);
        core.apply_channel("channel", approval_request("ap-1"));
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_notifies_while_only_the_settings_window_is_focused() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.window_focus("settings", true);
        core.apply_channel("channel", approval_request("ap-1"));
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_notifies_when_the_setting_is_off() {
        let (_dir, core, notifier) = notifying_fixture(false);
        core.apply_channel("channel", approval_request("ap-1"));
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_finished_turn_notifies_once() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", finished());
        let sent = notifier.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].body, "Finished");
    }

    #[test]
    fn a_duplicate_approval_id_notifies_once() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", approval_request("ap-1"));
        core.apply_channel("channel", approval_request("ap-1"));
        assert_eq!(notifier.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_cancelled_turn_does_not_notify() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", Update::TurnFinished(discard_core::driver::TurnOutcome::Cancelled));
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    /// Load a thread of the channel `channel` through `add_thread`. Its own name and agent differ from the parent's.
    fn add_notifying_thread(core: &AppCore, kind: discard_core::model::ThreadKind) {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        let init = CoreInit { state: UiState::new(ChannelId("thread".into()), vec![]), commands, project: "p".into(), channel: "Why?".into(), agent: "Quinn".into(), provider: "fake".into() };
        let channel = discard_core::model::Channel { id: ChannelId("thread".into()), agent_id: AgentId("agent".into()), project_id: None, name: "Why?".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        let info = discard_core::model::ThreadInfo { channel_id: ChannelId("thread".into()), parent_channel_id: ChannelId("channel".into()), anchor_message_id: None, kind, followed: true, name: "Why?".into(), state: None, agent_id: AgentId("agent".into()), agent_name: "Quinn".into() };
        core.add_thread(init, &channel, &info);
    }

    #[test]
    fn a_sub_agent_thread_turn_does_not_notify_but_its_approval_does() {
        let (_dir, core, notifier) = notifying_fixture(true);
        add_notifying_thread(&core, discard_core::model::ThreadKind::SubAgent);
        core.apply_channel("thread", finished());
        assert!(notifier.0.lock().unwrap().is_empty());
        core.apply_channel("thread", approval_request("ap-1"));
        assert_eq!(notifier.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_question_thread_turn_notifies_with_the_parent_name_unless_the_parent_is_muted() {
        let (_dir, core, notifier) = notifying_fixture(true);
        add_notifying_thread(&core, discard_core::model::ThreadKind::Question);
        core.apply_channel("thread", finished());
        {
            let sent = notifier.0.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!((sent[0].title.as_str(), sent[0].body.as_str()), ("c · Quinn", "Finished"));
            assert_eq!(sent[0].channel_id, "channel", "a click opens the parent channel");
        }
        core.set_channel_muted("channel", true);
        core.apply_channel("thread", finished());
        assert_eq!(notifier.0.lock().unwrap().len(), 1, "a muted parent silences the finished turn");
        core.apply_channel("thread", approval_request("ap-1"));
        assert_eq!(notifier.0.lock().unwrap().len(), 2, "an approval still notifies");
    }

    #[test]
    fn a_muted_channel_notifies_for_an_approval_but_not_for_a_finished_turn() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.set_channel_muted("channel", true);
        core.apply_channel("channel", finished());
        assert!(notifier.0.lock().unwrap().is_empty());
        core.apply_channel("channel", approval_request("ap-1"));
        assert_eq!(notifier.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_sent_notification_carries_the_channel_id() {
        let (_dir, core, notifier) = notifying_fixture(true);
        core.apply_channel("channel", approval_request("ap-1"));
        core.apply_channel("channel", finished());
        let sent = notifier.0.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].channel_id.as_str(), sent[1].channel_id.as_str()), ("channel", "channel"));
    }

    #[test]
    fn a_channel_is_busy_while_a_turn_runs_an_approval_waits_or_another_action_runs() {
        let (_dir, core, _received) = muted_fixture();
        assert_eq!(core.channel_busy_reason("channel"), None);
        core.send(&ViewId::main(), "work".into());
        assert!(core.channel_busy_reason("channel").unwrap().contains("A turn runs"));
        assert!(core.begin_channel_action("channel").is_err());
        core.apply(&ViewId::main(), Update::TurnFinished(discard_core::driver::TurnOutcome::Completed));
        assert_eq!(core.channel_busy_reason("channel"), None);
        core.begin_channel_action("channel").unwrap();
        assert!(core.channel_busy_reason("channel").unwrap().contains("changing"), "a second action waits for the first");
        assert!(core.begin_channel_archive("channel").is_err());
        core.end_channel_change("channel");
        assert_eq!(core.channel_busy_reason("channel"), None);
        assert_eq!(core.channel_busy_reason("not loaded"), None, "a channel that is not loaded is idle");
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
        core.configure_read_marker(&ViewId::main(), dir.path(), "channel");
        core.send(&ViewId::main(), "hello".into());
        let before = core.snapshot(&ViewId::main());
        assert!(before.new_anchor_id.is_none());
        core.mark_read(&ViewId::main(), "two").unwrap();
        let after = core.snapshot(&ViewId::main());
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
        core.configure_read_marker(&ViewId::main(), dir.path(), "channel");
        core.mark_read(&ViewId::main(), "two").unwrap();
        core.mark_read(&ViewId::main(), "one").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("two"));
        assert!(!core.snapshot(&ViewId::main()).unread);
    }

    #[test]
    fn refreshed_branches_mark_the_channel_worktree_as_current() {
        use discard_core::branch_tree::{RowKind, TreeRow};
        let channel = ChannelId("channel".into());
        let (commands, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(channel, vec![]), commands, project: "p".into(), channel: "c".into(), agent: "a".into(), provider: "p".into() });
        let rows = vec![TreeRow { name: "main".into(), kind: RowKind::Main, worktree: Some("/repo".into()), detached: false, is_discard: false, parent: None }, TreeRow { name: "work".into(), kind: RowKind::Worktree, worktree: Some("/work".into()), detached: false, is_discard: true, parent: None }];
        core.set_branches(&ViewId::main(), rows, Some(Path::new("/work/sub")));
        let snapshot = core.snapshot(&ViewId::main());
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
        core.configure_read_marker(&ViewId::main(), dir.path(), "channel");
        core.mark_read(&ViewId::main(), "one").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("one"));
        core.mark_read(&ViewId::main(), "unknown").unwrap();
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("channel"), Some("one"));
    }

    /// A core with a channel "main" that has no project. State lives in `dir`, where `a` holds an older marker.
    fn core_for_markers(dir: &Path) -> AppCore {
        StateStore::open(&dir.join("state.toml")).set_last_read("a", "one").unwrap();
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        let core = AppCore::new(CoreInit { state: UiState::new(ChannelId("main".into()), vec![]), commands, project: "p".into(), channel: "main".into(), agent: "a".into(), provider: "fake".into() });
        core.configure_read_marker(&main(), dir, "main");
        core
    }
    fn add_channel_with_history(core: &AppCore, project: Option<&str>, id: &str, messages: &[&str]) {
        let history: Vec<_> = messages.iter().map(|m| Message { id: MessageId((*m).into()), channel_id: ChannelId(id.into()), role: Role::Agent, blocks: vec![] }).collect();
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        let channel = discard_core::model::Channel { id: ChannelId(id.into()), agent_id: AgentId("agent".into()), project_id: project.and_then(project_of), name: id.into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_channel(CoreInit { state: UiState::new(ChannelId(id.into()), history), commands, project: project.unwrap_or_default().into(), channel: id.into(), agent: "a".into(), provider: "fake".into() }, &channel);
    }
    fn channel_unread(core: &AppCore, id: &str) -> bool { core.snapshot(&main()).channels.into_iter().find(|c| c.id == id).unwrap().unread }
    fn marker_pair(channel: &str, message: &str) -> (String, String) { (channel.to_string(), message.to_string()) }
    #[test]
    fn loaded_channel_tails_lists_only_channels_of_the_given_projects() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        add_channel_with_history(&core, Some("p"), "empty", &[]);
        add_channel_with_history(&core, Some("q"), "other", &["x"]);
        add_channel_with_history(&core, None, "dm", &["y"]);
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        let info = discard_core::model::ThreadInfo { channel_id: ChannelId("t".into()), parent_channel_id: ChannelId("a".into()), anchor_message_id: None, kind: ThreadKind::SubAgent, followed: false, name: "t".into(), state: None, agent_id: AgentId("agent".into()), agent_name: "a".into() };
        let thread = discard_core::model::Channel { id: ChannelId("t".into()), agent_id: AgentId("agent".into()), project_id: project_of("p"), name: "t".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_thread(CoreInit { state: UiState::new(ChannelId("t".into()), vec![]), commands, project: "p".into(), channel: "t".into(), agent: "a".into(), provider: "fake".into() }, &thread, &info);
        let mut tails = core.loaded_channel_tails(&["p".to_string()]);
        tails.sort();
        assert_eq!(tails, vec![("a".to_string(), Some("two".to_string())), ("empty".to_string(), None)]);
    }
    #[test]
    fn apply_read_markers_marks_a_channel_read_and_clears_its_divider_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        assert!(channel_unread(&core, "a"));
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "two")]), Ok(true));
        assert!(!channel_unread(&core, "a"));
        assert!(core.state.lock().unwrap().channels["a"].new_anchor.is_none());
        assert_eq!(StateStore::open(&dir.path().join("state.toml")).last_read("a"), Some("two"));
    }
    #[test]
    fn apply_read_markers_skips_a_marker_that_already_matches_and_reports_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "two")]), Ok(true));
        let (rev, conversations) = (core.snapshot(&main()).rev, core.conversations_rev());
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "two")]), Ok(false));
        assert_eq!((core.snapshot(&main()).rev, core.conversations_rev()), (rev, conversations));
    }
    #[test]
    fn a_reply_saved_after_the_tails_were_read_stays_unread() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        let markers: Vec<_> = core.loaded_channel_tails(&["p".to_string()]).into_iter().filter_map(|(id, tail)| tail.map(|t| (id, t))).collect();
        assert_eq!(core.apply_read_markers(&markers), Ok(true));
        assert!(!channel_unread(&core, "a"));
        core.apply_channel("a", Update::ConversationMarker(Message { id: MessageId("three".into()), channel_id: ChannelId("a".into()), role: Role::Agent, blocks: vec![] }));
        assert!(channel_unread(&core, "a"));
    }
    #[test]
    fn apply_read_markers_never_moves_a_loaded_marker_backward() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two", "three"]);
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "three")]), Ok(true));
        let (rev, conversations) = (core.snapshot(&main()).rev, core.conversations_rev());
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "two")]), Ok(false));
        assert_eq!((core.snapshot(&main()).rev, core.conversations_rev()), (rev, conversations));
        assert_eq!(core.last_read("a"), Some("three".to_string()));
    }
    #[test]
    fn apply_read_markers_changes_nothing_when_the_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let core = core_for_markers(dir.path());
        // The state file now lives under a regular file, so every save fails.
        core.configure_read_marker(&main(), &blocker, "main");
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        add_channel_with_history(&core, Some("p"), "b", &["one", "two"]);
        let before = core.snapshot(&main()).rev;
        let conversations = core.conversations_rev();
        let anchor = core.state.lock().unwrap().channels["a"].new_anchor.clone();
        assert!(core.apply_read_markers(&[marker_pair("a", "two"), marker_pair("b", "two")]).is_err());
        assert_eq!(core.snapshot(&main()).rev, before);
        assert_eq!(core.conversations_rev(), conversations);
        assert_eq!(core.state.lock().unwrap().channels["a"].new_anchor, anchor);
        assert_eq!(core.state.lock().unwrap().window.last_read("a"), None);
    }
    #[test]
    fn a_thread_marker_bumps_history_rev_when_its_parent_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        StateStore::open(&dir.path().join("state.toml")).set_last_read("t", "t1").unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        core.switch_channel(&main(), "a").unwrap();
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::mem::forget(receiver);
        let history: Vec<_> = ["t1", "t2"].iter().map(|m| Message { id: MessageId((*m).into()), channel_id: ChannelId("t".into()), role: Role::Agent, blocks: vec![] }).collect();
        let info = discard_core::model::ThreadInfo { channel_id: ChannelId("t".into()), parent_channel_id: ChannelId("a".into()), anchor_message_id: None, kind: ThreadKind::SubAgent, followed: false, name: "t".into(), state: None, agent_id: AgentId("agent".into()), agent_name: "a".into() };
        let thread = discard_core::model::Channel { id: ChannelId("t".into()), agent_id: AgentId("agent".into()), project_id: project_of("p"), name: "t".into(), workspace_mode: discard_core::model::WorkspaceMode::MainCheckout };
        core.add_thread(CoreInit { state: UiState::new(ChannelId("t".into()), history), commands, project: "p".into(), channel: "t".into(), agent: "a".into(), provider: "fake".into() }, &thread, &info);
        let before = core.snapshot(&main()).history_rev;
        assert_eq!(core.apply_read_markers(&[marker_pair("t", "t2")]), Ok(true));
        assert!(core.snapshot(&main()).history_rev > before, "the open parent shows the thread chip");
    }
    #[test]
    fn apply_read_markers_leaves_pending_approvals_alone() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_for_markers(dir.path());
        add_channel_with_history(&core, Some("p"), "a", &["one", "two"]);
        ask_approval(&core, "a");
        let pending = |core: &AppCore| core.snapshot(&main()).channels.into_iter().find(|c| c.id == "a").unwrap().pending_approvals;
        assert_eq!(pending(&core), 1);
        assert_eq!(core.apply_read_markers(&[marker_pair("a", "two")]), Ok(true));
        assert_eq!(pending(&core), 1);
    }
}
