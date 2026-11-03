//! The main window: server rail, channel list, message pane, member list, composer.

use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::model::{Block, Message, Role};
use discard_core::timeline::{ApprovalPrompt, ApprovalView};
use crate::plugin_element::{ActionFactory, apply_theme, render_element};
use crate::settings::open_settings;
use crate::shared::Shared;
use discard_core::settings_page::SettingsPage;
use discard_core::ui_state::{Command, SlotLabels, UiState, Update};
use discard_lua::theme::{ProtectedPalette, Rgb, protected_palette};
use discard_lua::{LoadReport, PluginService, PluginState, Rendered, Review, ReviewKind};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::text::markdown;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Static labels the window shows. The engine owns the real data.
#[derive(Debug, Clone)]
pub struct ViewData {
    pub project_name: String,
    pub channel_name: String,
    pub agent_name: String,
    pub provider_label: String,
}

/// Colors for protected surfaces when the theme says nothing.
const PROTECTED_DEFAULT: ProtectedPalette = ProtectedPalette {
    background: Rgb(0x1e, 0x1e, 0x1e),
    foreground: Rgb(0xf0, 0xf0, 0xf0),
    border: Rgb(0xff, 0xaa, 0x00),
};

fn palette_text(p: ProtectedPalette) -> Hsla {
    hsla(p.foreground)
}

fn hsla(c: Rgb) -> Hsla {
    Rgba { r: f32::from(c.0) / 255.0, g: f32::from(c.1) / 255.0, b: f32::from(c.2) / 255.0, a: 1.0 }.into()
}

pub struct DiscardView {
    data: ViewData,
    plugins: Arc<PluginService>,
    /// What happened when user plugins were loaded at startup.
    load_report: Rc<LoadReport>,
    /// The plugin panel (review screen and plugin list) is open. Rust draws it. Plugins cannot change it.
    show_plugins: bool,
    /// Review IDs where the user switched on "trust future edits".
    trusted: HashSet<String>,
    panel_error: Option<String>,
    slot_cache: HashMap<&'static str, (String, Option<Rendered>)>,
    state: UiState,
    commands: UnboundedSender<Command>,
    composer: Entity<InputState>,
    _composer_events: Subscription,
}

/// Everything the window needs from the app to start.
pub struct ViewInit {
    pub data: ViewData,
    pub shared: Shared,
    pub state: UiState,
    pub commands: UnboundedSender<Command>,
    pub updates: UnboundedReceiver<Update>,
}

impl DiscardView {
    pub fn new(init: ViewInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ViewInit { data, shared, state, commands, mut updates } = init;
        let Shared { plugins, load_report, .. } = shared;
        let placeholder = format!("Message #{}", data.channel_name);
        let composer = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let events = cx.subscribe_in(&composer, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.send(window, cx);
            }
        });

        // Pump engine updates into the view. `recv` is runtime-agnostic, so GPUI's executor can await it.
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let alive = this
                    .update(cx, |view, cx| {
                        view.state.apply(update);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        apply_theme(&plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme(), cx);

        // The agent can submit a plugin while the user is looking at something else. Redraw when that happens.
        {
            let plugins = Arc::clone(&plugins);
            cx.spawn(async move |this, cx| {
                let mut seen = 0usize;
                loop {
                    cx.background_executor().timer(Duration::from_millis(700)).await;
                    let now = plugins.pending().len();
                    if now != seen {
                        seen = now;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }
            })
            .detach();
        }

        Self {
            data,
            plugins,
            load_report,
            show_plugins: false,
            trusted: HashSet::new(),
            panel_error: None,
            slot_cache: HashMap::new(),
            state,
            commands,
            composer,
            _composer_events: events,
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        let text = text.trim();
        if text.is_empty() || self.state.turn_running() {
            return;
        }
        self.state.user_sent(text);
        let _ = self.commands.send(Command::Send(text.to_string()));
        self.composer.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn decide(&mut self, id: &str, decision: Decision) {
        let _ = self.commands.send(Command::Decide { approval_id: ApprovalId(id.to_string()), decision });
    }

    fn cancel(&mut self) {
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
        let rendered = match self.slot_cache.get(slot) {
            Some((cached_key, rendered)) if *cached_key == key => rendered.clone(),
            _ => {
                let rendered = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).render_slot(slot, &state);
                self.slot_cache.insert(slot, (key, rendered.clone()));
                rendered
            }
        }?;
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
        let theme = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme();
        apply_theme(&theme, cx);
        cx.notify();
    }

    fn approve_review(&mut self, id: &str, cx: &mut Context<Self>) {
        let trust = self.trusted.remove(id);
        self.panel_error = self.plugins.approve(id, trust).err().map(|e| e.to_string());
        self.after_plugin_change(cx);
    }

    fn reject_review(&mut self, id: &str, cx: &mut Context<Self>) {
        self.plugins.reject(id);
        self.trusted.remove(id);
        self.after_plugin_change(cx);
    }

    fn rollback_plugin(&mut self, name: &str, cx: &mut Context<Self>) {
        self.panel_error = self.plugins.rollback(name).err().map(|e| e.to_string());
        self.after_plugin_change(cx);
    }

    fn toggle_trust(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.trusted.remove(id) {
            self.trusted.insert(id.to_string());
        }
        cx.notify();
    }

    fn review_card(&self, index: usize, review: &Review, palette: ProtectedPalette, cx: &mut Context<Self>) -> AnyElement {
        let (id_a, id_r, id_t) = (review.id.clone(), review.id.clone(), review.id.clone());
        let trusted = self.trusted.contains(&review.id);
        let kind = match review.kind {
            ReviewKind::New => "New plugin",
            ReviewKind::Update => "Update",
        };
        let diff_lines: Vec<AnyElement> = review
            .diff
            .lines()
            .map(|line| {
                let color = if line.starts_with('+') && !line.starts_with("+++") {
                    cx.theme().success
                } else if line.starts_with('-') && !line.starts_with("---") {
                    cx.theme().danger
                } else {
                    palette_text(palette)
                };
                div().text_color(color).child(if line.is_empty() { " ".to_string() } else { line.to_string() }).into_any_element()
            })
            .collect();
        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(hsla(palette.border))
            .bg(hsla(palette.background))
            .text_color(hsla(palette.foreground))
            .child(div().font_weight(FontWeight::BOLD).child(format!("{kind}: {}", review.name)))
            .child(div().text_sm().child(review.manifest.description.clone()))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("This plugin asks to:"))
            .children(review.capability_lines.iter().map(|l| div().text_sm().child(format!("• {l}"))))
            .when(review.capability_lines.is_empty(), |c| c.child(div().text_sm().child("• nothing beyond running in the sandbox")))
            .when(!review.capability_changes.is_empty(), |c| {
                c.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Changed since the approved version:"))
                    .children(review.capability_changes.iter().map(|l| div().text_sm().child(l.clone())))
            })
            .when(review.full_local_access, |c| {
                c.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().danger)
                        .font_weight(FontWeight::BOLD)
                        .child("FULL LOCAL ACCESS: this plugin can run programs. A program can read your files, use the network, and start other programs."),
                )
            })
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Code changes:"))
            .child(
                v_flex()
                    .id(("diff", index))
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(hsla(palette.border))
                    .font_family("Menlo")
                    .text_xs()
                    .children(diff_lines),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(review.trust_offered, |row| {
                        row.child(
                            Button::new(("trust", index))
                                .outline()
                                .small()
                                .label(if trusted { "Trust future edits: ON" } else { "Trust future edits: off" })
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle_trust(&id_t, cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new(("review-reject", index))
                            .danger()
                            .small()
                            .label("Reject")
                            .on_click(cx.listener(move |this, _, _, cx| this.reject_review(&id_r, cx))),
                    )
                    .child(
                        Button::new(("review-approve", index))
                            .success()
                            .small()
                            .label("Approve and run")
                            .on_click(cx.listener(move |this, _, _, cx| this.approve_review(&id_a, cx))),
                    ),
            )
            .into_any_element()
    }

    /// The plugin panel: reviews waiting for the user, and installed plugins. Plugins cannot replace or hide it.
    fn plugin_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme();
        let palette = protected_palette(&theme, PROTECTED_DEFAULT);
        let reviews = self.plugins.pending();
        let cards: Vec<AnyElement> = reviews.iter().enumerate().map(|(i, r)| self.review_card(i, r, palette, cx)).collect();
        let statuses = self.plugins.statuses();
        let rows: Vec<AnyElement> = statuses
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let name = s.name.clone();
                let state = match &s.state {
                    PluginState::Running => "running".to_string(),
                    PluginState::Failed(m) => format!("failed: {}", m.lines().next().unwrap_or("")),
                    PluginState::ModifiedOutsideReview => "disabled: files changed outside review".to_string(),
                    PluginState::DisabledSafeMode => "disabled: safe mode".to_string(),
                };
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(s.name.clone()))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(state))
                    .when(s.trusted_edits, |r| r.child(div().text_xs().child("trusts future edits")))
                    .child(div().flex_1())
                    .child(
                        Button::new(("rollback", i))
                            .outline()
                            .small()
                            .label("Roll back")
                            .on_click(cx.listener(move |this, _, _, cx| this.rollback_plugin(&name, cx))),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .id("plugin-panel")
            .flex_1()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child("Plugins"))
                    .child(div().flex_1())
                    .child(Button::new("plugins-close").outline().small().label("Close").on_click(cx.listener(|this, _, _, cx| {
                        this.show_plugins = false;
                        cx.notify();
                    }))),
            )
            .when_some(self.panel_error.clone(), |c, e| c.child(div().p_2().rounded_md().border_1().border_color(cx.theme().danger).text_sm().child(e)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Waiting for your review"))
            .children(cards)
            .when(reviews.is_empty(), |c| c.child(self.muted("Nothing to review.", cx)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Installed plugins"))
            .children(rows)
            .when(statuses.is_empty(), |c| c.child(self.muted("No user plugins are installed. The built-in interface is a bundled plugin and is not listed here.", cx)))
            .into_any_element()
    }

    /// A banner, drawn by Rust in the main column, when reviews are waiting.
    fn review_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.plugins.pending();
        if pending.is_empty() || self.show_plugins {
            return None;
        }
        let theme = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme();
        let palette = protected_palette(&theme, PROTECTED_DEFAULT);
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
                .child(Button::new("banner-review").primary().small().label("Review").on_click(cx.listener(|this, _, _, cx| {
                    this.show_plugins = true;
                    cx.notify();
                })))
                .into_any_element(),
        )
    }

    /// Rust-rendered list of plugins that failed. Plugins cannot hide or replace it.
    fn plugin_errors(&self) -> Option<AnyElement> {
        let (errors, theme) = {
            let manager = self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner());
            (manager.errors(), manager.theme())
        };
        let mut lines: Vec<String> = errors.iter().map(|(name, msg)| format!("{name}: {}", msg.lines().next().unwrap_or(""))).collect();
        for status in self.plugins.statuses() {
            match status.state {
                PluginState::ModifiedOutsideReview => lines.push(format!("{}: disabled because its files changed outside review", status.name)),
                PluginState::Failed(reason) if !errors.contains_key(&status.name) => {
                    lines.push(format!("{}: {}", status.name, reason.lines().next().unwrap_or("")));
                }
                _ => {}
            }
        }
        if let Some(problem) = &self.load_report.key_problem {
            lines.push(format!("user plugins are off: {problem}"));
        }
        if self.load_report.safe_mode {
            lines.push("safe mode: user plugins are disabled".to_string());
        }
        if lines.is_empty() {
            return None;
        }
        let palette = protected_palette(&theme, PROTECTED_DEFAULT);
        Some(
            v_flex()
                .mx_4()
                .mb_2()
                .p_2()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(hsla(palette.border))
                .bg(hsla(palette.background))
                .text_color(hsla(palette.foreground))
                .text_sm()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Plugin problems"))
                .children(lines)
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

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "Working",
        AgentStatus::AwaitingApproval => "Waiting for approval",
        AgentStatus::Idle => "Idle",
        AgentStatus::Offline => "Offline",
    }
}

impl DiscardView {
    fn server_rail(&self, cx: &App) -> impl IntoElement {
        v_flex()
            .w(px(72.))
            .h_full()
            .flex_none()
            .items_center()
            .py_3()
            .gap_2()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(Avatar::new().name(self.data.project_name.clone()))
    }

    fn channel_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(240.))
            .h_full()
            .flex_none()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .px_4()
                    .py_3()
                    .font_weight(FontWeight::BOLD)
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(self.data.project_name.clone()),
            )
            .child(
                div().p_2().child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                        .child(format!("# {}", self.data.channel_name)),
                ),
            )
            .child(div().flex_1())
            .child(
                // Rust draws these entry points, so a plugin cannot hide the way to review plugins or open Settings.
                v_flex().p_2().gap_2().child(
                    Button::new("plugins-open")
                        .outline()
                        .small()
                        .w_full()
                        .label(match self.plugins.pending().len() {
                            0 => "Plugins".to_string(),
                            n => format!("Plugins ({n} to review)"),
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_plugins = !this.show_plugins;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("settings-open")
                        .outline()
                        .small()
                        .w_full()
                        .label("Settings")
                        .on_click(|_, _, cx| open_settings(SettingsPage::General, cx)),
                ),
            )
    }

    fn member_list(&self, cx: &App) -> impl IntoElement {
        let status = self.state.agent_status();
        v_flex()
            .w(px(240.))
            .h_full()
            .flex_none()
            .gap_2()
            .p_3()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("AGENTS"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Avatar::new().name(self.data.agent_name.clone()).small())
                    .child(
                        v_flex()
                            .child(div().child(self.data.agent_name.clone()))
                            .child(
                                h_flex()
                                    .gap_1()
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
            .px_4()
            .py_3()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::BOLD).child(format!("# {}", self.data.channel_name)))
            .child(div().flex_1())
            .when(pending > 0, |row| {
                row.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(cx.theme().warning)
                        .child(Badge::new().count(pending))
                        .child("Needs approval"),
                )
            })
    }

    fn render_block(&self, block: &Block, index: usize, cx: &mut Context<Self>) -> AnyElement {
        match block {
            Block::Markdown { text } => div().child(markdown(text.clone())).into_any_element(),
            Block::Code { text, .. } | Block::Diff { text } => div()
                .p_2()
                .rounded_md()
                .bg(cx.theme().muted)
                .font_family("Menlo")
                .text_sm()
                .child(text.clone())
                .into_any_element(),
            Block::ToolCall { name, input, output, .. } => {
                let summary = discard_core::timeline::summarize_input(name, input);
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{name}  {summary}")),
                    )
                    .when_some(output.as_ref(), |col, out| {
                        let shown: String = out.chars().take(400).collect();
                        col.child(div().text_xs().text_color(cx.theme().muted_foreground).child(shown))
                    })
                    .into_any_element()
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
        let palette = protected_palette(&self.plugins.manager().lock().unwrap_or_else(|p| p.into_inner()).theme(), PROTECTED_DEFAULT);
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
            .map(|(i, b)| self.render_block(b, row * 1000 + i, cx))
            .collect();
        h_flex()
            .gap_3()
            .px_4()
            .py_2()
            .items_start()
            .child(Avatar::new().name(author.clone()))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().font_weight(FontWeight::BOLD).child(author))
                    .children(blocks),
            )
            .into_any_element()
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
            .map(|(i, b)| self.render_block(b, base * 1000 + i, cx))
            .collect();
        Some(
            h_flex()
                .gap_3()
                .px_4()
                .py_2()
                .items_start()
                .child(Avatar::new().name(self.data.agent_name.clone()))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().font_weight(FontWeight::BOLD).child(self.data.agent_name.clone()))
                        .children(blocks),
                )
                .into_any_element(),
        )
    }
}

impl Render for DiscardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Give `ui_inspect` the state plugins draw with, and revert plugins that keep failing.
        self.plugins.set_ui_state(self.state.slot_state(&self.labels()));
        let reverted = self.plugins.maintain();
        if !reverted.is_empty() {
            self.panel_error = Some(format!("Reverted after repeated failures: {}", reverted.join(", ")));
            self.after_plugin_change(cx);
        }
        let history: Vec<Message> = self.state.history().to_vec();
        let header = match self.plugin_slot("channel_header", cx) {
            Some(element) => element,
            None => self.header(cx).into_any_element(),
        };
        let members = match self.plugin_slot("member_list", cx) {
            Some(element) => element,
            None => self.member_list(cx).into_any_element(),
        };
        let plugin_errors = self.plugin_errors();
        let review_banner = self.review_banner(cx);
        let panel = if self.show_plugins { Some(self.plugin_panel(cx)) } else { None };
        let rows: Vec<AnyElement> = history.iter().enumerate().map(|(i, m)| self.message_row(m, i, cx)).collect();
        let live = self.live_row(cx);
        let running = self.state.turn_running();
        let notice = self.state.notice().map(str::to_string);

        let main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(header)
            .child(match panel {
                Some(panel) => panel,
                None => v_flex()
                    .id("messages")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_2()
                    .children(rows)
                    .children(live)
                    .into_any_element(),
            })
            .children(review_banner)
            .children(plugin_errors)
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
                    .gap_2()
                    .p_3()
                    .items_center()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().child(Input::new(&self.composer)))
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
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                            .into_any_element()
                    }),
            );

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.server_rail(cx))
            .child(self.channel_list(cx))
            .child(main)
            .child(members)
    }
}
