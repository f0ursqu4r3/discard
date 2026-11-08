//! State shared between engine updates and desktop commands.
use super::dto::*;
use crate::plugin_memory::PluginMemory;
use discard_core::model::AgentId;
use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::timeline::ApprovalView;
use discard_core::ui_state::{Command, SlotLabels, UiState, Update, new_divider_anchor, startup_marker};
use discard_lua::{PluginService, LoadReport};
use discard_core::branch_tree::{TreeRow, RowKind, current_row};
use discard_core::window_state::{Bounds, PanelWidths, StateStore};
use std::collections::HashSet;
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
}
impl ChannelState {
    fn new(init: CoreInit) -> Self {
        let channel_id = init.state.channel_id().0.clone();
        Self { ui: init.state, runtime_generation: 0, commands: init.commands, channel: init.channel, project_name: init.project, agent: init.agent,
            provider: init.provider, channel_id, agent_id: String::new(), project_id: None,
            mode: discard_core::model::WorkspaceMode::MainCheckout, new_anchor: None,
            archive: None, archived: false, archive_busy: false, read_only: false }
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
            changed: Notify::new(),
        }
    }
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
        s.parked.retain(|_, c| c.agent_id != id);
        let removed_active = s.active.agent_id == id;
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
        if let Some(marker) = startup_marker(s.active.ui.history(), s.window.last_read(&channel.id.0)) { let _ = s.window.set_last_read(&channel.id.0, &marker); }
        s.active.new_anchor = new_divider_anchor(s.active.ui.history(), s.window.last_read(&channel.id.0), None, false);
        s.rev += 1; s.history_rev += 1;
        self.branch_generation.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
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
    pub fn cancel_all(&self) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for channel in std::iter::once(&s.active).chain(s.parked.values()) { let _ = channel.commands.send(Command::Cancel); }
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
        if !s.parked.contains_key(id) { return Err("This channel is unavailable.".into()); }
        s.window.set_selected_conversation(Some(id))?;
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
        HistoryDto { channel_id: s.active.channel_id.clone(), rev: s.history_rev, messages: history_dto_with_authors(s.active.ui.history(), s.active.ui.authors()) }
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
                    .filter(|channel| !channel.channel_id.is_empty()).map(|channel| ChannelDto {
                        id: channel.channel_id.clone(), agent_id: channel.agent_id.clone(), project_id: channel.project_id.as_ref().map(|id| id.0.clone()), name: channel.channel.clone(), agent: channel.agent.clone(),
                        provider: channel.provider.clone(), workspace_mode: channel.mode.as_str().into(),
                        running: channel.ui.turn_running(), pending_approvals: channel.ui.pending_approvals(),
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
                agent_status: match ui.agent_status() {
                    AgentStatus::Idle => "idle",
                    AgentStatus::Working => "working",
                    AgentStatus::AwaitingApproval => "awaiting_approval",
                    AgentStatus::Offline => "offline",
                }
                .into(),
                turn_running: ui.turn_running(),
                notice: ui.notice().map(str::to_owned),
                live_blocks: ui.live_blocks().to_vec(),
                prompts: ui
                    .prompts()
                    .iter()
                    .map(|p| PromptDto {
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
                    .collect(),
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
        history_dto_with_authors(state.active.ui.history(), state.active.ui.authors())
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
    pub fn decide(&self, approval_id: String, decision: String) { self.decide_in(None, approval_id, decision); }
    pub fn decide_in(&self, channel: Option<&str>, approval_id: String, decision: String) {
        let decision = match decision.as_str() {
            "approve" => Decision::Approve,
            "deny" => Decision::Deny,
            "always_allow" => Decision::AlwaysAllow,
            _ => return,
        };
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if channel.is_some_and(|id| id != s.active.channel_id) { return; }
        if !s.active.ui.prompts().iter().any(|p| p.approval_id.to_string() == approval_id && p.view == ApprovalView::Pending) { return; }
        let _ = s.active.commands.send(Command::Decide {
            approval_id: ApprovalId(approval_id),
            decision,
        });
    }
    pub fn cancel(&self) { self.cancel_in(None); }
    pub fn cancel_in(&self, channel: Option<&str>) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if channel.is_some_and(|id| id != s.active.channel_id) { return; }
        let _ = s.active.commands.send(Command::Cancel);
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
        if generation.is_some_and(|generation| Some(generation) != current) { return false; }
        let memory_changed = matches!(&update, Update::MemoryChanges { .. });
        if s.active.channel_id != channel {
            let marker = s.window.last_read(channel).map(str::to_owned);
            let Some(parked) = s.parked.get_mut(channel) else { return false };
            let was_running = parked.ui.turn_running();
            parked.ui.apply(update);
            parked.new_anchor = new_divider_anchor(parked.ui.history(), marker.as_deref(), parked.new_anchor.as_deref(), false);
            let finished = was_running && !parked.ui.turn_running();
            if memory_changed { s.memory_rev += 1; }
            if !s.active.ui.turn_running() && !s.parked.values().any(|c| c.ui.turn_running()) { s.confirm = None; }
            s.rev += 1;
            self.changed.notify_one();
            return finished;
        }
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
        self.changed.notify_one();
        was_running && !s.active.ui.turn_running()
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use discard_core::model::{ChannelId, Message, MessageId, Role};

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
