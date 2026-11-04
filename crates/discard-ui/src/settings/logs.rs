use super::SettingsView;
use discard_core::log_view::{self, Component, Level, LogFilter, LogLine, LogTail, MAX_LINES};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::Path;
pub(super) struct Logs {
    tail: LogTail,
    lines: Vec<LogLine>,
    pub filter: LogFilter,
    pub id: Entity<InputState>,
    follow: bool,
    scroll: ScrollHandle,
}
impl Logs {
    pub fn new(home: &Path, window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        Self {
            tail: LogTail::open(&home.join("logs")),
            lines: Vec::new(),
            filter: LogFilter::default(),
            id: cx.new(|cx| InputState::new(window, cx).placeholder("Session, turn, or plugin ID")),
            follow: true,
            scroll: ScrollHandle::new(),
        }
    }
    pub fn clear_cached(&mut self, home: &Path) {
        self.lines.clear();
        self.tail = LogTail::open(&home.join("logs"));
    }
    pub fn poll(&mut self) -> bool {
        let generation = self.tail.generation();
        let lines = self.tail.poll();
        let cleared = generation != self.tail.generation();
        if cleared {
            self.lines.clear();
        }
        if lines.is_empty() {
            return cleared;
        }
        self.lines
            .extend(lines.iter().map(|s| log_view::parse_line(s)));
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - MAX_LINES);
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        true
    }
}
pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let levels = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ];
    let level_buttons = levels
        .into_iter()
        .enumerate()
        .map(|(i, level)| {
            Button::new(("log-level", i))
                .label(format!("{level:?}"))
                .when(view.logs.filter.min_level == level, |b| b.primary())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.logs.filter.min_level = level;
                    cx.notify();
                }))
        })
        .collect::<Vec<_>>();
    let components = [
        None,
        Some(Component::Provider),
        Some(Component::Plugin),
        Some(Component::Approval),
        Some(Component::Storage),
        Some(Component::Mcp),
        Some(Component::Workspace),
        Some(Component::Other),
    ];
    let component_buttons = components
        .into_iter()
        .enumerate()
        .map(|(i, component)| {
            Button::new(("log-component", i))
                .label(component.map_or("All", Component::label))
                .when(view.logs.filter.component == component, |b| b.primary())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.logs.filter.component = component;
                    cx.notify();
                }))
        })
        .collect::<Vec<_>>();
    let entity = cx.entity();
    let follow = Switch::new("logs-follow")
        .checked(view.logs.follow)
        .on_click(move |on, _, cx| {
            let on = *on;
            entity.update(cx, |this, cx| {
                this.logs.follow = on;
                cx.notify();
            });
        });
    let lines = view
        .logs
        .lines
        .iter()
        .filter(|l| view.logs.filter.matches(l))
        .map(|l| {
            div().child(log_view::shorten(&format!(
                "{} {:?} {} {}",
                l.time,
                l.level,
                l.component.label(),
                l.message
            )))
        })
        .collect::<Vec<_>>();
    v_flex()
        .gap_2()
        .child("Level")
        .child(h_flex().gap_1().children(level_buttons))
        .child("Component")
        .child(h_flex().gap_1().children(component_buttons))
        .child(Input::new(&view.logs.id))
        .child(h_flex().gap_2().child("Follow").child(follow))
        .child(
            v_flex()
                .id("log-lines")
                .h(px(400.))
                .overflow_y_scroll()
                .track_scroll(&view.logs.scroll)
                .children(lines),
        )
        .into_any_element()
}
