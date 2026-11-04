//! The Settings window: a second OS window with a sidebar, a search box, and one page at a time.

mod appearance;
mod general;
mod plugins;
mod providers;
mod agents;

use crate::shared::Shared;
use discard_core::settings_page::{SettingsPage, search};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// How often the window checks `settings.toml` for edits made outside it.
const POLL_INTERVAL: Duration = Duration::from_millis(700);

/// One sidebar row. A page with no body shows greyed, tagged "Soon", and cannot be clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SidebarEntry {
    page: SettingsPage,
    clickable: bool,
}

/// The pages the search keeps, in sidebar order.
fn sidebar_entries(query: &str) -> Vec<SidebarEntry> {
    search(query)
        .into_iter()
        .map(|page| SidebarEntry {
            page,
            clickable: page.is_available(),
        })
        .collect()
}

pub struct SettingsView {
    shared: Shared,
    page: SettingsPage,
    search: Entity<InputState>,
    query: String,
    /// The last error from a General page change. `None` after a change succeeds.
    general_error: Option<String>,
    appearance_error: Option<String>,
    pickers: appearance::Pickers,
    /// Review IDs where the user switched on "trust future edits".
    trusted: HashSet<String>,
    _search_events: Subscription,
    agent_editor: agents::Editor,
    _agent_input_events: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(
        shared: Shared,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let events = cx.subscribe_in(&search, window, |this, input, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.query = input.read(cx).value().to_string();
                cx.notify();
            }
        });

        // The file can change on disk, and a review can arrive while this window is open.
        // Read the file again, and redraw when it changed, when plugin state changed, or when the review count changed.
        let settings = shared.settings.clone();
        // Read the file now, so the first frame does not show values that are up to one poll old.
        settings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .reload_if_changed();
        let plugins = shared.plugins.clone();
        let epoch = shared.plugin_epoch.clone();
        let appearance = shared.appearance.clone();
        let provider_statuses = shared.providers.iter().map(|p| p.setup.clone()).collect::<Vec<_>>();
        let mut seen_statuses = Vec::new();
        let mut seen_scales = appearance.lock().unwrap_or_else(|p| p.into_inner()).scales();
        let mut seen_epoch = epoch.load(Ordering::SeqCst);
        let mut seen_pending = plugins.pending().len();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let changed = settings
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .reload_if_changed();
                let now_epoch = epoch.load(Ordering::SeqCst);
                let now_pending = plugins.pending().len();
                let now_scales = appearance.lock().unwrap_or_else(|p| p.into_inner()).scales();
                let statuses = provider_statuses.iter().map(|s| s.lock().unwrap_or_else(|p| p.into_inner()).clone()).collect::<Vec<_>>();
                let providers_changed = statuses != seen_statuses;
                seen_statuses = statuses;
                let plugins_changed = now_epoch != seen_epoch || now_pending != seen_pending || now_scales != seen_scales;
                seen_scales = now_scales;
                seen_epoch = now_epoch;
                seen_pending = now_pending;
                let alive = this
                    .update(cx, |_, cx| {
                        if changed || plugins_changed || providers_changed {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        let pickers = appearance::Pickers::new(&shared, window, cx);
        let agent_editor = agents::Editor::new(window, cx);
        let model_events = cx.subscribe_in(&agent_editor.model, window, |this, input, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().to_string();
                if let Some((_, _, draft)) = &mut this.agent_editor.selected { draft.model = if value.is_empty() { None } else { Some(value) }; }
                this.agent_editor.changed(); cx.notify();
            }
        });
        let prompt_events = cx.subscribe_in(&agent_editor.prompt, window, |this, input, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                if let Some((_, _, draft)) = &mut this.agent_editor.selected { draft.system_prompt = input.read(cx).value().to_string(); }
                this.agent_editor.changed(); cx.notify();
            }
        });
        Self {
            shared,
            page,
            search,
            query: String::new(),
            general_error: None,
            appearance_error: None,
            pickers,
            trusted: HashSet::new(),
            _search_events: events,
            agent_editor,
            _agent_input_events: vec![model_events, prompt_events],
        }
    }

    /// Show `page` and clear the search, so the page is in the sidebar. A page with no body is ignored.
    pub fn show_page(&mut self, page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) {
        if !page.is_available() {
            return;
        }
        self.page = page;
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = sidebar_entries(&self.query);
        let none = entries.is_empty();
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let SidebarEntry { page, clickable } = entry;
                let selected = page == self.page;
                let row = h_flex()
                    .id(("settings-page", i))
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .rounded_md()
                    .child(div().flex_1().child(page.title()));
                if clickable {
                    row.cursor_pointer()
                        .when(selected, |r| {
                            r.bg(cx.theme().sidebar_accent)
                                .text_color(cx.theme().sidebar_accent_foreground)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = page;
                            cx.notify();
                        }))
                        .into_any_element()
                } else {
                    row.text_color(cx.theme().muted_foreground)
                        .child(soon_tag(cx))
                        .into_any_element()
                }
            })
            .collect();
        v_flex()
            .w(px(220.))
            .h_full()
            .flex_none()
            .gap_2()
            .p_2()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(Input::new(&self.search))
            .children(rows)
            .when(none, |c| {
                c.child(
                    div()
                        .px_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No matching settings"),
                )
            })
    }

    fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.page {
            SettingsPage::General => general::render(self, cx),
            SettingsPage::Appearance => appearance::render(self, window, cx),
            SettingsPage::Plugins => plugins::render(self, cx),
            SettingsPage::Providers => providers::render(self, cx),
            SettingsPage::Agents => agents::render(self, cx),
            _ => div()
                .text_color(cx.theme().muted_foreground)
                .child("Soon")
                .into_any_element(),
        };
        let spacing = self.shared.appearance.lock().unwrap_or_else(|p| p.into_inner()).scales().1;
        v_flex()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p(px(24.0 * spacing))
            .gap(px(16.0 * spacing))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(self.page.title()),
            )
            .child(body)
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font_scale = self.shared.appearance.lock().unwrap_or_else(|p| p.into_inner()).scales().0;
        window.set_rem_size(px(16.0 * font_scale));
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.sidebar(cx))
            .child(self.content(window, cx))
    }
}

/// The small "Soon" tag on a greyed row.
fn soon_tag(cx: &App) -> Div {
    div()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .child("Soon")
}

/// The open Settings window. A second `open_settings` call reuses it.
struct OpenSettings {
    handle: AnyWindowHandle,
    /// Weak, so the closed window's view can drop and stop its polling task.
    view: WeakEntity<SettingsView>,
}

impl Global for OpenSettings {}

/// What `open_settings` does with an existing window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reuse {
    /// The window took the page. Nothing more to do.
    Done,
    /// The window is open but is being updated now. Switch its page after the current update.
    Defer,
    /// The window is closed. Open a new one.
    OpenNew,
}

/// `alive`: the window is still open. `updated`: the update call succeeded.
/// A failed update alone does not mean the window closed: GPUI also fails an update
/// of a window that is being updated right now, for example when a deep link fires from inside Settings.
fn plan_reuse(alive: bool, updated: bool) -> Reuse {
    match (alive, updated) {
        (false, _) => Reuse::OpenNew,
        (true, true) => Reuse::Done,
        (true, false) => Reuse::Defer,
    }
}

/// Activate the window and switch its page. Returns `false` when the window cannot be updated now.
fn switch_page(
    handle: AnyWindowHandle,
    view: &WeakEntity<SettingsView>,
    page: SettingsPage,
    cx: &mut App,
) -> bool {
    let result = handle.update(cx, |_, window, cx| {
        window.activate_window();
        view.update(cx, |view, cx| view.show_page(page, window, cx))
    });
    matches!(result, Ok(Ok(())))
}

/// Open the Settings window on `page`. When one is open, activate it and switch its page.
pub fn open_settings(page: SettingsPage, cx: &mut App) {
    let existing = cx
        .try_global::<OpenSettings>()
        .map(|open| (open.handle, open.view.clone()));
    if let Some((handle, view)) = existing {
        let alive = cx.windows().contains(&handle) && view.upgrade().is_some();
        let updated = alive && switch_page(handle, &view, page, cx);
        match plan_reuse(alive, updated) {
            Reuse::Done => return,
            Reuse::Defer => {
                cx.defer(move |cx| {
                    switch_page(handle, &view, page, cx);
                });
                return;
            }
            Reuse::OpenNew => {}
        }
    }

    let Some(shared) = cx.try_global::<Shared>().cloned() else {
        tracing::warn!("cannot open Settings: the shared state is missing");
        return;
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(820.), px(560.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| SettingsView::new(shared, page, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.set_global(OpenSettings {
                handle,
                view: view.downgrade(),
            });
            cx.activate(true);
        }
        Err(e) => tracing::warn!(error = %e, "could not open the Settings window"),
    }
}

#[cfg(test)]
mod tests {
    // Import by name: `use super::*` would bring in GPUI's `test` macro and shadow the built-in `#[test]`.
    use super::{Reuse, SidebarEntry, plan_reuse, sidebar_entries};
    use discard_core::settings_page::SettingsPage;

    #[test]
    fn empty_query_lists_every_page_and_only_available_ones_click() {
        let entries = sidebar_entries("");
        assert_eq!(entries.len(), SettingsPage::all().len());
        for entry in entries {
            assert_eq!(entry.clickable, entry.page.is_available());
        }
    }

    #[test]
    fn appearance_search_finds_a_clickable_page() {
        assert_eq!(sidebar_entries("theme source"), vec![SidebarEntry { page: SettingsPage::Appearance, clickable: true }]);
    }

    #[test]
    fn search_keeps_unavailable_pages_not_clickable() {
        assert_eq!(
            sidebar_entries("provider"),
            vec![
                SidebarEntry {
                    page: SettingsPage::General,
                    clickable: true
                },
                SidebarEntry {
                    page: SettingsPage::Providers,
                    clickable: true
                },
            ]
        );
    }

    #[test]
    fn a_closed_window_opens_a_new_one() {
        assert_eq!(plan_reuse(false, false), Reuse::OpenNew);
    }

    #[test]
    fn an_open_window_that_took_the_update_is_done() {
        assert_eq!(plan_reuse(true, true), Reuse::Done);
    }

    #[test]
    fn a_busy_open_window_defers_and_never_opens_a_second_one() {
        assert_eq!(plan_reuse(true, false), Reuse::Defer);
    }

    #[test]
    fn no_match_gives_no_entries() {
        assert!(sidebar_entries("zzz").is_empty());
    }
}
