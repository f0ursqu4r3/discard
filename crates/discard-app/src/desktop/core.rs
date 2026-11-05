//! State shared between engine updates and desktop commands.
use super::dto::*;
use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::timeline::ApprovalView;
use discard_core::ui_state::{Command, SlotLabels, UiState, Update, new_divider_anchor, startup_marker};
use discard_lua::{PluginService, LoadReport};
use discard_core::branch_tree::{TreeRow, RowKind, current_row, visible_rows};
use discard_core::window_state::{Bounds, PanelWidths, StateStore};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc::UnboundedSender};

pub struct CoreInit {
    pub state: UiState,
    pub commands: UnboundedSender<Command>,
    pub project: String,
    pub channel: String,
    pub agent: String,
    pub provider: String,
}
struct State {
    ui: UiState,
    rev: u64,
    history_rev: u64,
    theme: ThemeDto,
    window: StateStore,
    branches: Vec<TreeRow>,
    current_branch: Option<usize>,
    collapsed: HashSet<String>,
    channel_id: String,
    project_id: String,
    project_path: String,
    agent_id: String,
    new_anchor: Option<String>,
    confirm: Option<super::plan::Confirm>,
    archive: Option<String>,
    archived: bool,
    archive_busy: bool,
    plugin_notice: Option<String>,
}
#[derive(Clone)]
struct BranchSource {
    store: Arc<Mutex<discard_core::store::Store>>,
    channel: discard_core::model::ChannelId,
    mode: discard_core::model::WorkspaceMode,
    project: PathBuf,
}
pub struct AppCore {
    state: Mutex<State>,
    commands: UnboundedSender<Command>,
    project: String,
    channel: String,
    agent: String,
    provider: String,
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
        Self {
            state: Mutex::new(State {
                ui: init.state,
                window: StateStore::open(Path::new("")),
                branches: vec![],
                current_branch: None,
                collapsed: HashSet::new(),
                channel_id: String::new(),
                project_id: String::new(),
                project_path: String::new(),
                agent_id: String::new(),
                new_anchor: None,
                confirm: None,
                archive: None,
                archived: false,
                archive_busy: false,
                plugin_notice: None,
                rev: 0,
                history_rev: 0,
                theme: ThemeDto {
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
            commands: init.commands,
            project: init.project,
            channel: init.channel,
            agent: init.agent,
            provider: init.provider,
            plugins: Mutex::new(None),
            load_report: Mutex::new(LoadReport::default()),
            slot_cache: Mutex::new(super::slots::SlotCache::default()),
            plugin_signature: Mutex::new(String::new()),
            branch_source: Mutex::new(None),
            branch_generation: AtomicU64::new(0),
            changed: Notify::new(),
        }
    }
    pub fn snapshot(&self) -> SnapshotDto {
        let (mut snapshot, slot_state) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ui = &state.ui;
            let rev = state.rev;
            let history_rev = state.history_rev;
            let theme = state.theme.clone();
            let labels = SlotLabels {
                project: self.project.clone(),
                channel: self.channel.clone(),
                agent_name: self.agent.clone(),
                provider: self.provider.clone(),
            };
            let slot_state = ui.slot_state(&labels);
            let window = state.window.state();
            let branches = visible_rows(&state.branches, &state.collapsed).into_iter().map(|row| BranchRowDto {
                key: row.name.clone(), label: row.name.clone(),
                kind: match row.kind { RowKind::Main => "main", RowKind::Worktree => "worktree", RowKind::Branch => "branch" }.into(),
                depth: row.depth(), current: state.current_branch.is_some_and(|i| std::ptr::eq(&state.branches[i], row)),
                collapsed: state.collapsed.contains(&row.name),
            }).collect();
            let snapshot = SnapshotDto {
                rev,
                history_rev,
                project: self.project.clone(),
                project_id: state.project_id.clone(),
                project_path: state.project_path.clone(),
                channel_id: state.channel_id.clone(),
                agent_id: state.agent_id.clone(),
                channel: self.channel.clone(),
                agent: self.agent.clone(),
                provider: self.provider.clone(),
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
                unread: super::windows::channel_indicators(ui.history(), state.window.last_read(&state.channel_id), ui.pending_approvals()).0,
                new_anchor_id: state.new_anchor.clone(),
                archived: state.archived,
                archive_busy: state.archive_busy,
                member_list_visible: window.member_list_visible,
                panels: PanelsDto {
                    channel_list: window.panels.channel_list,
                    member_list: window.panels.member_list,
                },
                branches,
                slots: BTreeMap::new(),
                banners: BannersDto {
                    review_count: 0,
                    review_names: vec![],
                    problem_lines: state.plugin_notice.iter().cloned().collect(),
                    confirm: state.confirm.map(|item| item.message().to_string()),
                    archive: state.archive.clone().map(|message| ArchiveNoticeDto { message }),
                    safe_mode: false,
                },
                theme,
            };
            (snapshot, slot_state)
        };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots = BTreeMap::new();
        if let Some(plugins) = plugins.as_ref() {
            plugins.set_ui_state(slot_state.clone());
            let mut manager = plugins.manager().lock().unwrap_or_else(|e| e.into_inner());
            slots = self.slot_cache.lock().unwrap_or_else(|e| e.into_inner())
                .get_or_render(&slot_state, || super::slots::render_slots(&mut manager, &slot_state));
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
        state.branches = rows;
        state.current_branch = current;
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn configure_branch_source(&self, store: Arc<Mutex<discard_core::store::Store>>, channel: discard_core::model::ChannelId, mode: discard_core::model::WorkspaceMode, project: PathBuf) {
        *self.branch_source.lock().unwrap_or_else(|e| e.into_inner()) = Some(BranchSource { store, channel, mode, project });
    }
    pub fn request_branch_refresh(&self) -> u64 { self.branch_generation.fetch_add(1, Ordering::SeqCst) + 1 }
    /// Reload the tree and determine the current worktree away from the UI thread.
    pub fn refresh_branches(&self, generation: u64) {
        let source = self.branch_source.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(source) = source else { return };
        let worktree = source.store.lock().unwrap_or_else(|e| e.into_inner()).worktree_for(&source.channel).ok().flatten().map(|record| PathBuf::from(record.path));
        let current = super::windows::branch_current_directory(Some(source.mode), &source.project, worktree.as_deref());
        let rows = discard_core::worktree::list_branch_tree(&source.project).unwrap_or_default();
        if self.branch_generation.load(Ordering::SeqCst) == generation { self.set_branches(rows, current.as_deref()); }
    }
    pub fn configure_targets(&self, project_id: String, project_dir: &Path, agent_id: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.project_id = project_id;
        state.project_path = project_dir.to_string_lossy().into_owned();
        state.agent_id = agent_id;
    }
    /// Initialize read markers for this channel, treating old history as read on first use.
    pub fn configure_read_marker(&self, data_dir: &Path, channel_id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.window = StateStore::open(&data_dir.join("state.toml"));
        s.channel_id = channel_id.to_owned();
        let marker = startup_marker(s.ui.history(), s.window.last_read(channel_id));
        if let Some(marker) = marker { let _ = s.window.set_last_read(channel_id, &marker); }
        s.new_anchor = new_divider_anchor(s.ui.history(), s.window.last_read(channel_id), None, false);
    }
    pub fn mark_read(&self, message_id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.channel_id.is_empty() { return Ok(()); }
        let Some(index) = s.ui.history().iter().position(|m| m.id.0 == message_id) else { return Ok(()) };
        let channel = s.channel_id.clone();
        if s.window.last_read(&channel).and_then(|id| s.ui.history().iter().position(|m| m.id.0 == id)).is_some_and(|previous| previous > index) { return Ok(()); }
        let changed = s.window.last_read(&channel) != Some(message_id);
        if changed { s.window.set_last_read(&channel, message_id)?; }
        let cleared = s.new_anchor.take().is_some();
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
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Err("plugins are unavailable".into()) };
        let result = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_action(plugin, action).map_err(|e| e.to_string());
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
        state.ui.apply(Update::Notice(message));
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
    pub fn archive_started(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.ui.turn_running() || state.archived || state.archive_busy { return false; }
        state.archive = None;
        state.archive_busy = true;
        state.rev += 1;
        self.changed.notify_one();
        true
    }
    pub fn archive_finished(&self, result: super::plan::ArchiveStep) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.archive_busy = false;
        match result {
            super::plan::ArchiveStep::Done(message) => { state.archived = true; state.ui.apply(Update::Notice(message)); }
            super::plan::ArchiveStep::NeedsConfirm(items) => state.archive = Some(format!("Archiving will delete these items:\n{}", items.join("\n"))),
            super::plan::ArchiveStep::Failed(message) => state.ui.apply(Update::Notice(format!("Could not archive channel: {message}"))),
        }
        state.rev += 1;
        self.changed.notify_one();
    }
    pub fn cancel_archive(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.archive.take().is_some() { state.rev += 1; self.changed.notify_one(); }
    }
    pub fn has_pending_archive(&self) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).archive.is_some() }
    pub fn resolve_copy(&self, target: &discard_core::context_menu::ContextTarget, action: discard_core::action::Action) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        discard_core::context_menu::resolve_copy(state.ui.history(), &self.channel, target, action)
    }
    pub fn menu_context(&self) -> discard_core::action::MenuContext {
        use discard_core::action::{MenuContext, MenuTarget, PluginMenuItem};
        let (turn_running, archived) = { let state = self.state.lock().unwrap_or_else(|e| e.into_inner()); (state.ui.turn_running(), state.archived) };
        let safe_mode = self.load_report.lock().unwrap_or_else(|e| e.into_inner()).safe_mode;
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return MenuContext { turn_running, archived, safe_mode, ..Default::default() } };
        let plugin_items = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).menu_items().into_iter().filter_map(|(plugin, index, item)| Some(PluginMenuItem { plugin, index, target: MenuTarget::parse(&item.target)?, label: item.label })).collect();
        MenuContext { turn_running, archived, safe_mode, pending_reviews: plugins.pending().len(), plugin_items }
    }
    pub fn run_plugin_menu(&self, plugin: &str, index: usize, target: Option<&discard_core::context_menu::ContextTarget>) -> Result<(), String> {
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let Some(plugins) = plugins.as_ref() else { return Err("plugins are unavailable".into()) };
        let (kind, id) = target.map_or(("app".to_owned(), String::new()), |target| (target.kind().id().to_owned(), target.target_id()));
        let result = plugins.manager().lock().unwrap_or_else(|e| e.into_inner()).run_menu_item(plugin, index, &kind, &id).map_err(|e| e.to_string());
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
        history_dto(
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .ui
                .history(),
        )
    }
    pub fn send(&self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.ui.turn_running() || s.archived || s.archive_busy || s.archive.is_some() {
            return;
        }
        if self.commands.send(Command::Send(text.clone())).is_ok() {
            s.new_anchor = None;
            s.ui.user_sent(&text);
            s.rev += 1;
            s.history_rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn decide(&self, approval_id: String, decision: String) {
        let decision = match decision.as_str() {
            "approve" => Decision::Approve,
            "deny" => Decision::Deny,
            "always_allow" => Decision::AlwaysAllow,
            _ => return,
        };
        let _ = self.commands.send(Command::Decide {
            approval_id: ApprovalId(approval_id),
            decision,
        });
    }
    pub fn cancel(&self) {
        let _ = self.commands.send(Command::Cancel);
    }
    pub fn apply(&self, update: Update) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let old_len = s.ui.history().len();
        let was_running = s.ui.turn_running();
        s.ui.apply(update);
        if super::windows::should_clear_confirm(s.ui.turn_running(), s.confirm.is_some()) { s.confirm = None; }
        s.rev += 1;
        if old_len != s.ui.history().len() {
            s.new_anchor = new_divider_anchor(s.ui.history(), s.window.last_read(&s.channel_id), s.new_anchor.as_deref(), false);
            s.history_rev += 1;
        }
        was_running && !s.ui.turn_running()
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use discard_core::model::{ChannelId, Message, MessageId, Role};

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
