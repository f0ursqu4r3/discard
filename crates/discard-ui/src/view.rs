//! The main window: server rail, channel list, message pane, member list, composer.

use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::model::{Block, Message, Role};
use discard_core::timeline::{ApprovalPrompt, ApprovalView};
use crate::dispatch::{self, Confirm};
use crate::menus;
use crate::appearance::{self, AppearanceState};
use crate::plugin_element::{ActionFactory, render_element};
use crate::problems::{problem_count, problem_lines, problem_summary};
use discard_core::action::{Action, MenuContext};
use discard_core::action::{MenuTarget, PluginMenuItem};
use discard_core::context_menu::{ContextTarget, context_menu, message_text};
use gpui_kit::component::menu::ContextMenuExt as _;
use crate::shared::{PROTECTED_DEFAULT, Shared, hsla};
use discard_core::ui_state::{Command, SlotLabels, UiState, Update, continues_group, first_unread_index};
use discard_core::settings::SettingsStore;
use discard_lua::theme::{ProtectedPalette, ThemeOverrides, protected_palette, protected_accent, MIN_CONTRAST};
use discard_core::diff::{classify_line, looks_like_unified_diff, LineKind};
use discard_lua::{LoadReport, PluginService, Rendered};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use discard_core::window_state::{StateStore, PanelWidths, MIN_PANEL_WIDTH, MAX_PANEL_WIDTH};
use gpui_kit::component::text::markdown;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

fn may_send(archived: bool, text: &str) -> bool { !archived && !text.trim().is_empty() }

/// Whether the server has unread agent messages in its channel.
fn server_has_unread(messages: &[Message], last_read: Option<&str>) -> bool {
    first_unread_index(messages, last_read).is_some()
}

/// Up to two capital letters for a server icon. Words split on anything that is not a letter or digit.
/// A one-word name gives its first two letters.
fn server_initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let letters: Vec<char> = match words.as_slice() {
        [] => Vec::new(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    letters.into_iter().flat_map(char::to_uppercase).collect()
}

// Icon dimensions use the same spacing scale as the rest of the window.
const SERVER_ICON_UNITS: f32 = 12.0;

/// Static labels the window shows. The engine owns the real data.
#[derive(Debug, Clone)]
pub struct ViewData {
    pub project_name: String,
    pub project_id: String,
    pub project_path: String,
    pub agent_id: String,
    pub channel_name: String,
    pub agent_name: String,
    pub provider_label: String,
}

pub struct DiscardView {
    data: ViewData,
    plugins: Arc<PluginService>,
    /// What happened when user plugins were loaded at startup.
    load_report: Rc<LoadReport>,
    /// Bumped when plugin state changes in either window. The poller compares it.
    plugin_epoch: Arc<AtomicU64>,
    /// The last plugin notice or error. The Plugins page shows it.
    plugin_notice: Arc<Mutex<Option<String>>>,
    settings: Arc<Mutex<SettingsStore>>,
    /// The theme in use. The poller keeps it current.
    appearance: Arc<Mutex<AppearanceState>>,
    slot_cache: HashMap<&'static str, (String, Option<Rendered>)>,
    state: UiState,
    commands: UnboundedSender<Command>,
    composer: Entity<TextareaState>,
    scroller: Entity<MessageScrollerState>,
    /// Font and spacing scale from the plugin theme. Set at the start of each render.
    scales: (f32, f32),
    window_state: Arc<Mutex<StateStore>>,
    _composer_events: Subscription,
    _window_activation: Subscription,
    initial_history_len: usize,
    /// A Quit or relaunch that waits for the user to confirm it. The main window draws the notice.
    pending_confirm: Option<Confirm>,
    archived: bool,
    archive_busy: bool,
    pending_archive: Option<Vec<String>>,
    channel_id: String,
    /// The state the installed menu bar was built for. A change installs it again.
    menu_ctx: MenuContext,
}

/// Everything the window needs from the app to start.
pub struct ViewInit {
    pub data: ViewData,
    pub shared: Shared,
    pub state: UiState,
    pub commands: UnboundedSender<Command>,
    pub updates: UnboundedReceiver<Update>,
    pub window_state: Arc<Mutex<StateStore>>,
}

impl DiscardView {
    pub fn new(init: ViewInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ViewInit { data, shared, state, commands, mut updates, window_state } = init;
        let Shared { plugins, load_report, plugin_epoch, plugin_notice, settings, appearance, channel_id, .. } = shared;
        let placeholder = format!("Message #{}", data.channel_name);
        let composer = cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder).auto_grow(1, 6).submit_on_enter(true));
        let scroller = cx.new(|cx| MessageScrollerState::new(row_count(state.history().len(), state.turn_running()), cx));
        let initial_history_len = state.history().len();
        {
            let mut store = window_state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.history().iter().any(|message| store.state().last_read.get(&channel_id.0) == Some(&message.id.0))
                && let Some(last) = state.history().last() {
                let _ = store.set_last_read(&channel_id.0, &last.id.0);
            }
        }
        let handle = window.window_handle();
        let bounds_store = Arc::clone(&window_state);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if this.upgrade().is_none() { break; }
                let result = handle.update(cx, |_, window, _| window.window_bounds().get_bounds());
                let Ok(b) = result else { break; };
                let bounds = discard_core::window_state::Bounds { x: f32::from(b.origin.x), y: f32::from(b.origin.y), width: f32::from(b.size.width), height: f32::from(b.size.height) };
                let mut store = bounds_store.lock().unwrap_or_else(|p| p.into_inner());
                if store.state().bounds != Some(bounds) && let Err(error) = store.set_bounds(bounds) {
                    tracing::warn!(%error, "cannot save window bounds");
                }
            }
        }).detach();
        let window_activation = cx.observe_window_activation(window, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(&composer, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.send(window, cx);
            }
            if matches!(event, InputEvent::Change) { cx.notify(); }
        });

        // Pump engine updates into the view. `recv` is runtime-agnostic, so GPUI's executor can await it.
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let alive = this
                    .update(cx, |view, cx| {
                        view.state.apply(update);
                        if should_clear_confirm(view.state.turn_running(), view.pending_confirm.is_some()) { view.pending_confirm = None; }
                        view.sync_scroller(cx);
                        // The pending-review count comes from the poller. Here only `turn_running` can move.
                        let pending_reviews = view.menu_ctx.pending_reviews;
                        view.refresh_menus(pending_reviews, cx);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        // The agent can submit a plugin while the user is looking at something else, and the user can approve,
        // reject, or roll back in Settings. Redraw when a review arrives. Apply the change when the epoch moves.
        {
            let plugins = Arc::clone(&plugins);
            let epoch = Arc::clone(&plugin_epoch);
            cx.spawn(async move |this, cx| {
                let mut seen_pending = 0usize;
                let mut seen_epoch = epoch.load(Ordering::SeqCst);
                loop {
                    cx.background_executor().timer(Duration::from_millis(700)).await;
                    if this.update(cx, |view, cx| view.refresh_appearance(cx)).is_err() {
                        break;
                    }
                    let now_pending = plugins.pending().len();
                    let now_epoch = epoch.load(Ordering::SeqCst);
                    let epoch_moved = now_epoch != seen_epoch;
                    let pending_moved = now_pending != seen_pending;
                    seen_pending = now_pending;
                    seen_epoch = now_epoch;
                    if epoch_moved || pending_moved {
                        let alive = this
                            .update(cx, |view, cx| {
                                view.refresh_menus(now_pending, cx);
                                if epoch_moved {
                                    view.after_plugin_change(cx);
                                } else {
                                    cx.notify();
                                }
                            })
                            .is_ok();
                        if !alive {
                            break;
                        }
                    }
                }
            })
            .detach();
        }

        let menu_ctx = MenuContext {
            safe_mode: load_report.safe_mode,
            turn_running: state.turn_running(),
            pending_reviews: plugins.pending().len(),
            plugin_items: menu_items(&plugins),
            ..Default::default()
        };
        menus::install(&menu_ctx, cx);

        Self {
            data,
            plugins,
            load_report,
            plugin_epoch,
            plugin_notice,
            settings,
            appearance,
            slot_cache: HashMap::new(),
            state,
            commands,
            composer,
            scroller,
            scales: (1.0, 1.0),
            window_state,
            _composer_events: events,
            _window_activation: window_activation,
            initial_history_len,
            pending_confirm: None,
            archived: false,
            archive_busy: false,
            pending_archive: None,
            channel_id: channel_id.0,
            menu_ctx,
        }
    }

    pub(crate) fn turn_running(&self) -> bool {
        self.state.turn_running()
    }

    pub(crate) fn archived(&self) -> bool { self.archived }
    pub(crate) fn archive_busy(&self) -> bool { self.archive_busy }
    pub(crate) fn set_archive_busy(&mut self, busy: bool, cx: &mut Context<Self>) { self.archive_busy = busy; cx.notify(); }
    pub(crate) fn set_archived(&mut self, cx: &mut Context<Self>) {
        self.archived = true;
        self.pending_archive = None;
        self.refresh_menus(self.menu_ctx.pending_reviews, cx);
        cx.notify();
    }
    pub(crate) fn set_pending_archive(&mut self, pending: Option<Vec<String>>, cx: &mut Context<Self>) { self.pending_archive = pending; cx.notify(); }
    pub(crate) fn resolve_copy(&self, target: &discard_core::context_menu::ContextTarget, action: Action) -> Option<String> {
        discard_core::context_menu::resolve_copy(self.state.history(), &self.data.channel_name, target, action)
    }

    /// Show `text` in the main window's notice line. The next send clears it.
    pub(crate) fn show_notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.state.apply(Update::Notice(text));
        cx.notify();
    }

    pub(crate) fn pending_confirm(&self) -> Option<Confirm> {
        self.pending_confirm
    }

    pub(crate) fn set_pending_confirm(&mut self, pending: Option<Confirm>, cx: &mut Context<Self>) {
        self.pending_confirm = pending;
        cx.notify();
    }

    /// Install the menu bar again when the state it shows changed.
    fn refresh_menus(&mut self, pending_reviews: usize, cx: &mut App) {
        let ctx = MenuContext {
            safe_mode: self.load_report.safe_mode,
            turn_running: self.state.turn_running(),
            pending_reviews,
            archived: self.archived,
            plugin_items: menu_items(&self.plugins),
        };
        if ctx != self.menu_ctx {
            menus::install(&ctx, cx);
            self.menu_ctx = ctx;
        }
    }

    fn sync_scroller(&mut self, cx: &mut Context<Self>) {
        let count = row_count(self.state.history().len(), self.state.turn_running());
        self.scroller.update(cx, |state, cx| {
            let old = state.item_count();
            if count > old { state.append(count - old, cx); }
            else if count < old { state.splice(count..old, 0, cx); }
            if count > 0 { state.remeasure_items(count - 1..count, cx); }
        });
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if !may_send(self.archived || self.archive_busy || self.pending_archive.is_some(), text) || self.state.turn_running() {
            return;
        }
        self.state.user_sent(text);
        self.sync_scroller(cx);
        let _ = self.commands.send(Command::Send(text.to_string()));
        self.composer.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn decide(&mut self, id: &str, decision: Decision) {
        let _ = self.commands.send(Command::Decide { approval_id: ApprovalId(id.to_string()), decision });
    }

    pub(crate) fn cancel(&mut self) {
        let _ = self.commands.send(Command::Cancel);
    }

    fn labels(&self) -> SlotLabels {
        SlotLabels {
            project: self.data.project_name.clone(),
            channel: self.data.channel_name.clone(),
            agent_name: self.data.agent_name.clone(),
            provider: self.data.provider_label.clone(),
        }
    }

    /// Render a slot with the plugin that owns it. `None` means the built-in view draws it.
    fn plugin_slot(&mut self, slot: &'static str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.state.slot_state(&self.labels());
        let key = state.to_string();
        let mut rendered = match self.slot_cache.get(slot) {
            Some((cached_key, rendered)) if *cached_key == key => rendered.clone(),
            _ => {
                let rendered = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).render_slot(slot, &state);
                self.slot_cache.insert(slot, (key, rendered.clone()));
                rendered
            }
        }?;
        // The member slot is a panel, so its root must track the divider width.
        // A plugin can still set widths on elements inside that root.
        if slot == "member_list" {
            rendered.element.props.remove("width");
            rendered.element.props.insert("grow".into(), discard_lua::element::Prop::Bool(true));
        }
        let this = cx.entity();
        let plugin = rendered.plugin.clone();
        let factory: ActionFactory = Rc::new(move |action: &str| {
            let (this, plugin, action) = (this.clone(), plugin.clone(), action.to_string());
            Box::new(move |_, _, cx| {
                this.update(cx, |view, cx| view.plugin_action(&plugin, &action, cx));
            })
        });
        Some(render_element(&rendered.element, &rendered.plugin, &factory, cx))
    }

    fn plugin_action(&mut self, plugin: &str, action: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).run_action(plugin, action) {
            tracing::warn!(plugin, action, error = %e, "plugin action failed");
        }
        // The plugin may have changed its own state, so its slots must render again.
        self.slot_cache.clear();
        cx.notify();
    }

    fn after_plugin_change(&mut self, cx: &mut Context<Self>) {
        self.slot_cache.clear();
        self.refresh_appearance(cx);
        cx.notify();
    }

    /// Check the settings, the system appearance, the theme files, and the plugin theme.
    /// Apply the theme when any of them changed.
    fn refresh_appearance(&mut self, cx: &mut Context<Self>) {
        let settings = {
            let mut store = self.settings.lock().unwrap_or_else(|p| p.into_inner());
            store.reload_if_changed();
            store.settings().appearance.clone()
        };
        let plugin_theme = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme();
        let system = appearance::system_appearance(cx.window_appearance());
        let mut state = self.appearance.lock().unwrap_or_else(|p| p.into_inner());
        if state.update(&settings, system, &plugin_theme) {
            appearance::apply(&state, cx);
            // Row heights depend on the font and spacing scale.
            self.scroller.update(cx, |scroller, cx| scroller.remeasure(cx));
            cx.notify();
        }
    }

    /// The theme the protected surfaces build their palette from.
    fn protected_theme(&self) -> ThemeOverrides {
        self.appearance.lock().unwrap_or_else(|p| p.into_inner()).protected_overrides()
    }

    /// A banner, drawn by Rust in the main column, when reviews are waiting.
    fn review_banner(&self) -> Option<AnyElement> {
        let pending = self.plugins.pending();
        if pending.is_empty() {
            return None;
        }
        let palette = protected_palette(&self.protected_theme(), PROTECTED_DEFAULT);
        let names = pending.iter().map(|r| r.name.clone()).collect::<Vec<_>>().join(", ");
        Some(
            h_flex()
                .mx_4()
                .mb_2()
                .p_2()
                .gap_2()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(hsla(palette.border))
                .bg(hsla(palette.background))
                .text_color(hsla(palette.foreground))
                .child(div().flex_1().text_sm().child(format!("Plugin code is waiting for your review: {names}")))
                .child(
                    Button::new("banner-review")
                        .primary()
                        .small()
                        .label("Review")
                        .on_click(|_, _, cx| dispatch::run(Action::ReviewPlugins, cx)),
                )
                .into_any_element(),
        )
    }

    /// The confirm notice for a Quit or relaunch while a turn runs. Rust draws it, so plugins cannot hide it.
    fn confirm_notice(&self) -> Option<AnyElement> {
        let pending = self.pending_confirm?;
        let palette = protected_palette(&self.protected_theme(), PROTECTED_DEFAULT);
        Some(
            h_flex()
                .mx_4()
                .mb_2()
                .p_2()
                .gap_2()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(hsla(palette.border))
                .bg(hsla(palette.background))
                .text_color(hsla(palette.foreground))
                .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(pending.message()))
                // Plain closures, not `cx.listener`: the dispatcher reads this view, so it must not be leased.
                .child(
                    Button::new("confirm-cancel")
                        .outline()
                        .small()
                        .label("Cancel")
                        .on_click(|_, _, cx| dispatch::cancel_pending(cx)),
                )
                .child(
                    Button::new("confirm-ok")
                        .danger()
                        .small()
                        .label("Confirm")
                        .on_click(|_, _, cx| dispatch::confirm_pending(cx)),
                )
                .into_any_element(),
        )
    }

    fn archive_notice(&self) -> Option<AnyElement> {
        if self.archived {
            return Some(h_flex().mx_4().mb_2().child(Button::new("archive-relaunch").label("Relaunch").on_click(|_, _, cx| dispatch::relaunch_same_mode(cx))).into_any_element());
        }
        let items = self.pending_archive.as_ref()?;
        Some(v_flex().mx_4().mb_2().gap_2().child("Archiving will delete these items:").children(items.iter().cloned().map(|item| div().child(item))).child(h_flex().gap_2()
            .child(Button::new("archive-cancel").label("Cancel").on_click(|_, _, cx| dispatch::cancel_archive(cx)))
            .child(Button::new("archive-confirm").danger().label("Archive and delete").on_click(|_, _, cx| dispatch::confirm_archive(cx)))).into_any_element())
    }

    /// One Rust-drawn line when plugins have problems. Plugins cannot hide or replace it.
    /// The list itself is on the Plugins page in Settings.
    fn plugin_problem_line(&self) -> Option<AnyElement> {
        let errors = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).errors();
        let lines = problem_lines(&errors, &self.plugins.statuses(), &self.load_report);
        let has_notice = self.plugin_notice.lock().unwrap_or_else(|p| p.into_inner()).is_some();
        let count = problem_count(lines.len(), has_notice);
        if count == 0 {
            return None;
        }
        let palette = protected_palette(&self.protected_theme(), PROTECTED_DEFAULT);
        Some(
            h_flex()
                .mx_4()
                .mb_2()
                .p_2()
                .gap_2()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(hsla(palette.border))
                .bg(hsla(palette.background))
                .text_color(hsla(palette.foreground))
                .text_sm()
                .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(problem_summary(count)))
                .child(
                    Button::new("problems-details")
                        .outline()
                        .small()
                        .label("Details")
                        .on_click(|_, _, cx| dispatch::run(Action::ShowPluginErrors, cx)),
                )
                .into_any_element(),
        )
    }
}

fn status_color(status: AgentStatus, cx: &App) -> Hsla {
    match status {
        AgentStatus::Working => cx.theme().success,
        AgentStatus::AwaitingApproval => cx.theme().warning,
        AgentStatus::Idle => cx.theme().muted_foreground,
        AgentStatus::Offline => cx.theme().border,
    }
}

fn menu_items(plugins: &PluginService) -> Vec<PluginMenuItem> {
    plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).menu_items().into_iter().filter_map(|(plugin, index, item)| Some(PluginMenuItem { plugin, index, target: MenuTarget::parse(&item.target)?, label: item.label })).collect()
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "Working",
        AgentStatus::AwaitingApproval => "Waiting for approval",
        AgentStatus::Idle => "Idle",
        AgentStatus::Offline => "Offline",
    }
}

impl DiscardView {
    /// `units` of GPUI's 4 px spacing step, times the spacing scale.
    fn space(&self, units: f32) -> Pixels {
        px(units * 4.0 * self.scales.1)
    }

    fn server_rail(&self, cx: &App) -> impl IntoElement {
        let marker = self.window_state.lock().unwrap_or_else(|p| p.into_inner()).state().last_read.get(&self.channel_id).cloned();
        let unread = server_has_unread(self.state.history(), marker.as_deref());
        let pending = self.state.pending_approvals();
        let initials = server_initials(&self.data.project_name);
        let size = self.space(SERVER_ICON_UNITS);
        let hover_radius = self.space(SERVER_ICON_UNITS * 0.2);
        v_flex()
            .w(px(72.))
            .h_full()
            .flex_none()
            .items_center()
            .py(self.space(3.0))
            .gap(self.space(2.0))
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex().relative().w_full().items_center().justify_center()
                    .child(div().absolute().left_0().w(self.space(1.0)).h(self.space(8.0)).rounded_full().bg(cx.theme().sidebar_foreground))
                    .child(
                        div().id("server-icon").relative().size(size)
                            .context_menu({ let target=ContextTarget::Server {id:self.data.project_id.clone(),name:self.data.project_name.clone(),path:self.data.project_path.clone()}; menus::to_popup(context_menu(&target,&self.menu_ctx),target) })
                            .child(div().id("server-avatar").size_full().flex().items_center().justify_center()
                                .rounded(self.space(SERVER_ICON_UNITS * 0.3))
                                .hover(move |style| style.rounded(hover_radius))
                                .bg(cx.theme().primary).text_color(cx.theme().primary_foreground)
                                .font_weight(FontWeight::BOLD).child(initials))
                            .when(unread, |icon| icon.child(div().absolute().top_0().right_0().size(self.space(2.0)).rounded_full().border_1().border_color(cx.theme().sidebar).bg(cx.theme().primary)))
                            .when(pending > 0, |icon| icon.child(div().absolute().bottom_0().right_0()
                                .px(self.space(1.0)).rounded_full().bg(cx.theme().warning)
                                .text_color(cx.theme().warning_foreground).text_xs().child(pending.to_string())))
                    )
            )
    }

    fn channel_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let marker = self.window_state.lock().unwrap_or_else(|p| p.into_inner()).state().last_read.get(&self.channel_id).cloned();
        let unread = server_has_unread(self.state.history(), marker.as_deref());
        let pending = self.state.pending_approvals();
        let selected = true;
        let hover = cx.theme().sidebar_accent;
        v_flex()
            .w_full()
            .h_full()
            .flex_none()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .px(self.space(4.0))
                    .py(self.space(3.0))
                    .font_weight(FontWeight::BOLD)
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(self.data.project_name.clone()),
            )
            .child(
                div().relative().p(self.space(2.0))
                    .when(unread && pending == 0, |row| row.child(
                        div().absolute().left_0().top_1_2().size(self.space(1.5)).rounded_full().bg(cx.theme().sidebar_foreground)
                    ))
                    .child(
                    h_flex()
                        .id("channel-row")
                        .w_full()
                        .items_center()
                        .gap(self.space(2.0))
                        .px(self.space(2.0))
                        .py(self.space(1.0))
                        .rounded_md()
                        .when(selected, |row| row.bg(cx.theme().sidebar_accent).text_color(cx.theme().sidebar_accent_foreground))
                        .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                        .child(div().flex_1().min_w_0().when(unread, |name| name.font_weight(FontWeight::BOLD)).child(format!("# {}", self.data.channel_name)))
                        .when(pending > 0, |row| row.child(div().px(self.space(1.0)).rounded_full().bg(cx.theme().warning)
                            .text_color(cx.theme().warning_foreground).text_xs().child(pending.to_string())))
                        .context_menu({ let target = ContextTarget::Channel { id: self.channel_id.clone() }; menus::to_popup(context_menu(&target, &self.menu_ctx), target) }),
                ),
            )
            .child(div().flex_1())
            .child(
                // Rust draws these entry points, so a plugin cannot hide the way to review plugins or open Settings.
                v_flex().p(self.space(2.0)).gap(self.space(2.0)).child(
                    Button::new("plugins-open")
                        .outline()
                        .small()
                        .w_full()
                        .label(match self.plugins.pending().len() {
                            0 => "Plugins".to_string(),
                            n => format!("Plugins ({n} to review)"),
                        })
                        .on_click(|_, _, cx| dispatch::run(Action::ReviewPlugins, cx)),
                )
                .child(
                    Button::new("settings-open")
                        .outline()
                        .small()
                        .w_full()
                        .label("Settings")
                        .on_click(|_, _, cx| dispatch::run(Action::OpenSettings, cx)),
                ),
            )
    }

    fn member_list(&self, cx: &App) -> impl IntoElement {
        let status = self.state.agent_status();
        v_flex()
            .w_full()
            .h_full()
            .flex_none()
            .gap(self.space(2.0))
            .p(self.space(3.0))
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("AGENTS"))
            .child(
                h_flex()
                    .id("agent-row")
                    .gap(self.space(2.0))
                    .items_center()
                    .context_menu({ let target=ContextTarget::Agent {id:self.data.agent_id.clone(),name:self.data.agent_name.clone()}; menus::to_popup(context_menu(&target,&self.menu_ctx),target) })
                    .child(Avatar::new().name(self.data.agent_name.clone()).small())
                    .child(
                        v_flex()
                            .child(div().child(self.data.agent_name.clone()))
                            .child(
                                h_flex()
                                    .gap(self.space(1.0))
                                    .items_center()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(div().size_2().rounded_full().bg(status_color(status, cx)))
                                    .child(format!("{} · {}", status_label(status), self.data.provider_label)),
                            ),
                    ),
            )
    }

    fn header(&self, cx: &App) -> impl IntoElement {
        let pending = self.state.pending_approvals();
        h_flex()
            .px(self.space(4.0))
            .py(self.space(3.0))
            .gap(self.space(3.0))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::BOLD).child(format!("# {}", self.data.channel_name)))
            .child(div().flex_1())
            .when(pending > 0, |row| {
                row.child(
                    h_flex()
                        .gap(self.space(2.0))
                        .items_center()
                        .text_color(cx.theme().warning)
                        .child(Badge::new().count(pending))
                        .child("Needs approval"),
                )
            })
    }

    /// `text` one line at a time, colored by diff line kind. The added and removed colors come from the
    /// Ghostty theme and are pushed until they stay readable on what sits behind the text:
    /// the code block's muted fill when `on_block` is true, the window background otherwise.
    fn diff_lines(&self, text: &str, on_block: bool, cx: &App) -> AnyElement {
        let appearance = self.appearance.lock().unwrap_or_else(|p| p.into_inner());
        let ui = appearance.effective();
        let behind = if on_block { ui.muted } else { ui.background };
        let palette = ProtectedPalette {
            background: appearance::to_lua(behind),
            foreground: appearance::to_lua(ui.foreground),
            border: appearance::to_lua(ui.border),
        };
        let clamp = |color| hsla(protected_accent(&palette, appearance::to_lua(color), MIN_CONTRAST));
        let (added, removed) = (clamp(ui.diff_added), clamp(ui.diff_removed));
        let lines = text.lines().map(|line| {
            let color = match classify_line(line) {
                LineKind::Added => Some(added),
                LineKind::Removed => Some(removed),
                LineKind::Hunk => Some(cx.theme().muted_foreground),
                LineKind::Context => None,
            };
            // An empty line keeps its height.
            let shown = if line.is_empty() { " " } else { line };
            div().when_some(color, |row, color| row.text_color(color)).child(shown.to_string())
        });
        v_flex().children(lines).into_any_element()
    }

    /// `owner` is the message ID and the block's index in that message. The live row has no message ID,
    /// so it passes `None` and its blocks get no context menu.
    fn render_block(&self, block: &Block, index: usize, owner: Option<(&str, usize)>, cx: &mut Context<Self>) -> AnyElement {
        let target = owner.and_then(|(message_id, block_index)| {
            let message_id = message_id.to_string();
            match block {
                Block::Code { .. } => Some(ContextTarget::CodeBlock { message_id, block: block_index }),
                Block::Diff { .. } => Some(ContextTarget::DiffBlock { message_id, block: block_index }),
                Block::ToolCall { output, .. } => Some(ContextTarget::ToolCall { message_id, block: block_index, has_output: output.is_some() }),
                Block::Markdown { .. } | Block::Approval { .. } => None,
            }
        });
        let with_menu = |element: Stateful<Div>| -> AnyElement {
            match target.clone() {
                Some(target) => element.context_menu(menus::to_popup(context_menu(&target, &self.menu_ctx), target)).into_any_element(),
                None => element.into_any_element(),
            }
        };
        match block {
            Block::Markdown { text } => div().child(markdown(text.clone())).into_any_element(),
            Block::Code { language, text } => {
                let diff = language.as_deref().is_some_and(|lang| lang.eq_ignore_ascii_case("diff") || lang.eq_ignore_ascii_case("patch"));
                let content = if diff { self.diff_lines(text, true, cx) } else { div().child(text.clone()).into_any_element() };
                let copy = target.clone();
                let header = h_flex().justify_between().items_center().child(language.clone().unwrap_or_else(|| "Code".into())).when_some(copy, |header, target| header.child(Button::new(("copy-code", index)).small().label("Copy").on_click(move |_, _, cx| dispatch::run_on(Action::CopyCode, Some(target.clone()), cx))));
                with_menu(div().id(("code-block", index)).p(self.space(2.0)).rounded_md().bg(cx.theme().muted).text_sm().child(header).child(div().font_family("Menlo").child(content)))
            }
            Block::Diff { text } => {
                let copy = target.clone();
                let header = h_flex().justify_between().items_center().child("Diff").when_some(copy, |header, target| header.child(Button::new(("copy-diff", index)).small().label("Copy").on_click(move |_, _, cx| dispatch::run_on(Action::CopyDiff, Some(target.clone()), cx))));
                with_menu(div().id(("diff-block", index)).p(self.space(2.0)).rounded_md().bg(cx.theme().muted).text_sm().child(header).child(div().font_family("Menlo").child(self.diff_lines(text, true, cx))))
            },
            Block::ToolCall { name, input, output, .. } => {
                let summary = discard_core::timeline::summarize_input(name, input);
                let element = v_flex()
                    .id(("tool-call", index))
                    .gap(self.space(1.0))
                    .p(self.space(2.0))
                    .rounded_md()
                    .border_l_2()
                    .border_color(cx.theme().primary)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{name}  {summary}")),
                    )
                    .when_some(output.as_ref(), |col, out| {
                        let shown: String = out.chars().take(400).collect();
                        let content = if looks_like_unified_diff(out) { self.diff_lines(&shown, false, cx) } else { div().child(shown).into_any_element() };
                        col.child(div().text_xs().text_color(cx.theme().muted_foreground).child(content))
                    });
                with_menu(element)
            }
            Block::Approval { approval_id, tool, summary } => {
                let prompt = self.state.prompts().iter().find(|p| p.approval_id.0 == *approval_id).cloned();
                self.render_approval(index, approval_id, tool, summary, prompt, cx)
            }
        }
    }

    fn render_approval(
        &self,
        index: usize,
        approval_id: &str,
        tool: &str,
        summary: &str,
        prompt: Option<ApprovalPrompt>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = prompt.as_ref().map(|p| p.view);
        // Protected surface: plugin themes can recolor it, but it always stays readable.
        let palette = protected_palette(&self.protected_theme(), PROTECTED_DEFAULT);
        let card = v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(hsla(palette.border))
            .bg(hsla(palette.background))
            .text_color(hsla(palette.foreground))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{tool} wants to run")))
            .child(div().font_family("Menlo").text_sm().child(summary.to_string()));
        match view {
            Some(ApprovalView::Pending) => {
                let (a, d, al) = (approval_id.to_string(), approval_id.to_string(), approval_id.to_string());
                card.child(
                    h_flex()
                        .gap_2()
                        .child(Button::new(("approve", index)).success().small().label("Approve").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.decide(&a, Decision::Approve);
                                cx.notify();
                            }),
                        ))
                        .child(Button::new(("deny", index)).danger().small().label("Deny").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.decide(&d, Decision::Deny);
                                cx.notify();
                            },
                        )))
                        .child(Button::new(("always", index)).outline().small().label("Always allow").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.decide(&al, Decision::AlwaysAllow);
                                cx.notify();
                            }),
                        )),
                )
                .into_any_element()
            }
            Some(ApprovalView::Approved) => card.child(self.muted("Approved", cx)).into_any_element(),
            Some(ApprovalView::Denied) => card.child(self.muted("Denied", cx)).into_any_element(),
            Some(ApprovalView::Stale) => card.child(self.muted("Expired: the turn ended before this was answered", cx)).into_any_element(),
            None => card.child(self.muted("Answered", cx)).into_any_element(),
        }
    }

    fn muted(&self, text: &str, cx: &App) -> Div {
        div().text_sm().text_color(cx.theme().muted_foreground).child(text.to_string())
    }

    fn message_row(&self, message: &Message, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let author = match message.role {
            Role::User => "You".to_string(),
            Role::Agent => self.data.agent_name.clone(),
            Role::System => "System".to_string(),
        };
        let blocks: Vec<AnyElement> = message
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| self.render_block(b, row * 1000 + i, Some((message.id.0.as_str(), i)), cx))
            .collect();
        let grouped = continues_group(row.checked_sub(1).and_then(|i| self.state.history().get(i)).map(|m| m.role), message.role);
        let target = ContextTarget::Message { id: message.id.0.clone(), has_text: !message_text(&message.blocks).is_empty() };
        let copy_target = target.clone();
        let row_element = h_flex()
            .id(("message-row", row))
            .relative()
            .group("message-row")
            .hover(|style| style.bg(cx.theme().list_hover))
            .gap(self.space(3.0))
            .px(self.space(4.0))
            .py(self.space(2.0))
            .items_start()
            .child(div().w(self.space(10.0)).flex_none().when(!grouped, |el| el.child(Avatar::new().name(author.clone()))))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(self.space(1.0))
                    .when(!grouped, |col| col.child(div().font_weight(FontWeight::BOLD).child(author)))
                    .children(blocks),
            )
            .child(div().absolute().top_0().right_0().invisible().group_hover("message-row", |style| style.visible()).child(Button::new(("copy-message", row)).small().label("Copy").disabled(!matches!(copy_target, ContextTarget::Message { has_text: true, .. })).on_click(move |_, _, cx| dispatch::run_on(Action::CopyMessageText, Some(copy_target.clone()), cx))))
            .context_menu(menus::to_popup(context_menu(&target, &self.menu_ctx), target));
        let marker = self.window_state.lock().unwrap_or_else(|p| p.into_inner()).state().last_read.get(&self.channel_id).cloned();
        let unread = first_unread_index(self.state.history(), marker.as_deref()) == Some(row);
        let row_element = if row >= self.initial_history_len && !message.blocks.iter().any(|block| matches!(block, Block::Approval { .. })) && !cx.reduce_motion() {
            row_element.with_animation(("message-fade", row), Animation::new(Duration::from_millis(150)), |element, delta| element.opacity(delta)).into_any_element()
        } else { row_element.into_any_element() };
        v_flex().when(unread, |col| col.child(h_flex().gap(self.space(2.0)).px(self.space(4.0)).items_center().text_sm().text_color(cx.theme().primary).child(div().flex_1().border_t_1().border_color(cx.theme().primary)).child("New"))).child(row_element).into_any_element()
    }

    fn live_row(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.state.turn_running() {
            return None;
        }
        let base = self.state.history().len() + 1;
        let blocks: Vec<AnyElement> = self
            .state
            .live_blocks()
            .iter()
            .enumerate()
            .map(|(i, b)| self.render_block(b, base * 1000 + i, None, cx))
            .collect();
        let grouped = continues_group(self.state.history().last().map(|message| message.role), Role::Agent);
        let live = h_flex()
                .id("live-message-row")
                .hover(|style| style.bg(cx.theme().list_hover))
                .gap(self.space(3.0))
                .px(self.space(4.0))
                .py(self.space(2.0))
                .items_start()
                .child(div().w(self.space(10.0)).flex_none().when(!grouped, |el| el.child(Avatar::new().name(self.data.agent_name.clone()))))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(self.space(1.0))
                        .when(!grouped, |col| col.child(div().font_weight(FontWeight::BOLD).child(self.data.agent_name.clone())))
                        .children(blocks)
                        .child(div().text_color(cx.theme().muted_foreground).child("▍")),
                )
                ;
        let reduced = self.state.live_blocks().iter().any(|block| matches!(block, Block::Approval { .. })) || cx.reduce_motion();
        Some(if reduced { live.into_any_element() } else { live.with_animation(("live-fade", base), Animation::new(Duration::from_millis(150)), |element, delta| element.opacity(delta)).into_any_element() })
    }
}

impl Render for DiscardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.is_window_active() && !self.scroller.read(cx).is_scrolled_up()
            && let Some(last) = self.state.history().last() {
            let mut store = self.window_state.lock().unwrap_or_else(|p| p.into_inner());
            if store.state().last_read.get(&self.channel_id) != Some(&last.id.0)
                && let Err(error) = store.set_last_read(&self.channel_id, &last.id.0) {
                tracing::warn!(%error, "cannot save read state");
            }
        }
        self.scales = self.appearance.lock().unwrap_or_else(|p| p.into_inner()).scales();
        window.set_rem_size(px(16.0 * self.scales.0));
        // Give `ui_inspect` the state plugins draw with, and revert plugins that keep failing.
        self.plugins.set_ui_state(self.state.slot_state(&self.labels()));
        let reverted = self.plugins.maintain();
        if !reverted.is_empty() {
            *self.plugin_notice.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(format!("Reverted after repeated failures: {}", reverted.join(", ")));
            self.after_plugin_change(cx);
            // Tell the Plugins page. The main window's poller runs `after_plugin_change` once more, which is harmless.
            self.plugin_epoch.fetch_add(1, Ordering::SeqCst);
        }
        let header = match self.plugin_slot("channel_header", cx) {
            Some(element) => element,
            None => self.header(cx).into_any_element(),
        };
        let members = match self.plugin_slot("member_list", cx) {
            Some(element) => element,
            None => self.member_list(cx).into_any_element(),
        };
        let plugin_problem_line = self.plugin_problem_line();
        let review_banner = self.review_banner();
        let confirm_notice = self.confirm_notice();
        let weak = cx.weak_entity();
        let scroller = MessageScroller::new("messages", self.scroller.clone(), move |index, _, cx| {
            weak.update(cx, |view, cx| {
                if let Some(message) = view.state.history().get(index).cloned() {
                    view.message_row(&message, index, cx)
                } else {
                    view.live_row(cx).unwrap_or_else(|| div().into_any_element())
                }
            }).unwrap_or_else(|_| div().into_any_element())
        }).scrollbar(true).with_jump_button_renderer(|button| button.label("Jump to latest"));
        let running = self.state.turn_running();
        let notice = self.state.notice().map(str::to_string);
        let composer_disabled = self.archived || self.archive_busy || self.pending_archive.is_some();
        let can_send = may_send(composer_disabled, self.composer.read(cx).value().as_ref());

        let main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(header)
            .child(div().flex_1().min_h_0().child(scroller))
            .children(confirm_notice)
            .children(self.archive_notice())
            .children(review_banner)
            .children(plugin_problem_line)
            .when_some(notice, |col, text| {
                col.child(
                    div()
                        .mx_4()
                        .mb_2()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().warning)
                        .text_sm()
                        .child(text),
                )
            })
            .child(
                h_flex()
                    .gap(self.space(2.0))
                    .p(self.space(3.0))
                    .items_center()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().min_w_0().child(
                        Textarea::new(&self.composer)
                            .bg(cx.theme().muted)
                            .border_color(cx.theme().border)
                            .p(self.space(2.0))
                            .disabled(composer_disabled),
                    ))
                    .child(if running {
                        Button::new("cancel")
                            .danger()
                            .label("Stop")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel();
                                cx.notify();
                            }))
                            .into_any_element()
                    } else {
                        Button::new("send")
                            .primary()
                            .label("Send")
                            .disabled(!can_send)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                            .into_any_element()
                    }),
            );

        let widths = self.window_state.lock().unwrap_or_else(|p| p.into_inner()).state().panels;
        let store = Arc::clone(&self.window_state);
        let panels = h_resizable("main-panels")
            .on_resize(move |state, _, cx| {
                let sizes = state.read(cx).sizes();
                if sizes.len() == 3 {
                    let panels = PanelWidths { channel_list: f32::from(sizes[0]), member_list: f32::from(sizes[2]) };
                    if let Err(error) = store.lock().unwrap_or_else(|p| p.into_inner()).set_panels(panels) {
                        tracing::warn!(%error, "cannot save panel widths");
                    }
                }
            })
            .child(resizable_panel().size(px(widths.channel_list)).size_range(px(MIN_PANEL_WIDTH)..px(MAX_PANEL_WIDTH)).fixed().child(self.channel_list(cx)))
            .child(resizable_panel().child(main))
            .child(resizable_panel().size(px(widths.member_list)).size_range(px(MIN_PANEL_WIDTH)..px(MAX_PANEL_WIDTH)).fixed().child(members));
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.server_rail(cx))
            .child(div().flex_1().min_w_0().h_full().child(panels))
    }
}

/// The rows the message pane shows: each message, plus the live row while a turn runs.
fn row_count(history: usize, turn_running: bool) -> usize { history + usize::from(turn_running) }

/// A Quit or relaunch confirm only makes sense while a turn runs. Drop it when the turn has ended.
fn should_clear_confirm(turn_running: bool, pending: bool) -> bool { !turn_running && pending }

#[cfg(test)]
mod tests {
    #[test]
    fn server_initials_use_words_or_the_first_two_letters() {
        assert_eq!(super::server_initials("discard.ui-juice"), "DU");
        assert_eq!(super::server_initials("My Project"), "MP");
        assert_eq!(super::server_initials("discard"), "DI");
        assert_eq!(super::server_initials("x"), "X");
        assert_eq!(super::server_initials("  ..  "), "");
        assert_eq!(super::server_initials(""), "");
    }

    #[test]
    fn server_unread_dot_requires_an_agent_message_after_a_known_marker() {
        use discard_core::model::{ChannelId, Message, MessageId, Role};
        let channel = ChannelId::new();
        let make = |id: &str, role| Message { id: MessageId(id.into()), channel_id: channel.clone(), role, blocks: vec![] };
        let messages = vec![make("a", Role::User), make("b", Role::Agent), make("c", Role::User)];
        assert!(!super::server_has_unread(&[], Some("a")));
        assert!(!super::server_has_unread(&messages[..1], Some("a")));
        assert!(!super::server_has_unread(&messages, None));
        assert!(!super::server_has_unread(&messages, Some("missing")));
        assert!(super::server_has_unread(&messages, Some("a")));
        assert!(!super::server_has_unread(&messages, Some("b")));
        assert!(!super::server_has_unread(&messages, Some("c")));
    }
    #[test]
    fn send_is_ignored_after_archive() {
        assert!(!super::may_send(true, "hello"));
        assert!(!super::may_send(false, "  "));
        assert!(super::may_send(false, "hello"));
    }
    use super::{row_count, should_clear_confirm};

    #[test]
    fn counts_history_and_live_row() {
        assert_eq!(row_count(0, false), 0);
        assert_eq!(row_count(3, false), 3);
        assert_eq!(row_count(3, true), 4);
    }

    #[test]
    fn clears_pending_confirmation_when_turn_ends() {
        assert!(should_clear_confirm(false, true));
        assert!(!should_clear_confirm(true, true));
        assert!(!should_clear_confirm(false, false));
    }
}
