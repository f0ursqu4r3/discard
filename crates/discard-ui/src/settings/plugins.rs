//! The Plugins page: reviews waiting for the user, installed plugins, rollback, and plugin problems.
//! Rust draws it in the Settings window. Plugins cannot hide or replace it.

use super::SettingsView;
use crate::dispatch;
use crate::problems::problem_lines;
use discard_core::action::Action;
use crate::shared::{PROTECTED_DEFAULT, hsla};
use discard_lua::theme::{ProtectedPalette, protected_palette};
use discard_lua::{PluginState, Review, ReviewKind};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::sync::atomic::Ordering;

/// Tell every window that plugin state changed, and redraw this page.
fn changed(view: &SettingsView, cx: &mut Context<SettingsView>) {
    view.shared.plugin_epoch.fetch_add(1, Ordering::SeqCst);
    cx.notify();
}

fn set_notice(view: &SettingsView, notice: Option<String>) {
    *view
        .shared
        .plugin_notice
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = notice;
}

fn approve_review(view: &mut SettingsView, id: &str, cx: &mut Context<SettingsView>) {
    let trust = view.trusted.remove(id);
    let error = view
        .shared
        .plugins
        .approve(id, trust)
        .err()
        .map(|e| e.to_string());
    set_notice(view, error);
    changed(view, cx);
}

fn reject_review(view: &mut SettingsView, id: &str, cx: &mut Context<SettingsView>) {
    // A rejected review is a user action that resolves the notice.
    if view.shared.plugins.reject(id) {
        set_notice(view, None);
    }
    view.trusted.remove(id);
    changed(view, cx);
}

fn dismiss_notice(view: &mut SettingsView, cx: &mut Context<SettingsView>) {
    set_notice(view, None);
    changed(view, cx);
}

fn rollback_plugin(view: &mut SettingsView, name: &str, cx: &mut Context<SettingsView>) {
    let error = view
        .shared
        .plugins
        .rollback(name)
        .err()
        .map(|e| e.to_string());
    set_notice(view, error);
    changed(view, cx);
}

fn toggle_trust(view: &mut SettingsView, id: &str, cx: &mut Context<SettingsView>) {
    if !view.trusted.remove(id) {
        view.trusted.insert(id.to_string());
    }
    cx.notify();
}

fn muted(text: &str, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
}

fn review_card(
    view: &SettingsView,
    index: usize,
    review: &Review,
    palette: ProtectedPalette,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let (id_a, id_r, id_t) = (review.id.clone(), review.id.clone(), review.id.clone());
    let trusted = view.trusted.contains(&review.id);
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
                hsla(palette.foreground)
            };
            div()
                .text_color(color)
                .child(if line.is_empty() {
                    " ".to_string()
                } else {
                    line.to_string()
                })
                .into_any_element()
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
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .child(format!("{kind}: {}", review.name)),
        )
        .child(div().text_sm().child(review.manifest.description.clone()))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("This plugin asks to:"),
        )
        .children(
            review
                .capability_lines
                .iter()
                .map(|l| div().text_sm().child(format!("• {l}"))),
        )
        .when(review.capability_lines.is_empty(), |c| {
            c.child(
                div()
                    .text_sm()
                    .child("• nothing beyond running in the sandbox"),
            )
        })
        .when(!review.capability_changes.is_empty(), |c| {
            c.child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Changed since the approved version:"),
            )
            .children(
                review
                    .capability_changes
                    .iter()
                    .map(|l| div().text_sm().child(l.clone())),
            )
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
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Code changes:"),
        )
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
                            .label(if trusted {
                                "Trust future edits: ON"
                            } else {
                                "Trust future edits: off"
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| toggle_trust(this, &id_t, cx)),
                            ),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new(("review-reject", index))
                        .danger()
                        .small()
                        .label("Reject")
                        .on_click(
                            cx.listener(move |this, _, _, cx| reject_review(this, &id_r, cx)),
                        ),
                )
                .child(
                    Button::new(("review-approve", index))
                        .success()
                        .small()
                        .label("Approve and run")
                        .on_click(
                            cx.listener(move |this, _, _, cx| approve_review(this, &id_a, cx)),
                        ),
                ),
        )
        .into_any_element()
}

/// The safe-mode notice. "Relaunch normally" relaunches without `--safe-mode`.
fn safe_mode_row(palette: ProtectedPalette) -> AnyElement {
    h_flex()
        .gap_2()
        .p_2()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(hsla(palette.border))
        .bg(hsla(palette.background))
        .text_color(hsla(palette.foreground))
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Safe mode: user plugins disabled"),
        )
        .child(
            Button::new("relaunch-normally")
                .outline()
                .small()
                .label("Relaunch normally")
                .on_click(|_, _, cx| dispatch::run(Action::ToggleSafeMode, cx)),
        )
        .into_any_element()
}

pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let plugins = view.shared.plugins.clone();
    let (errors, theme) = {
        let manager = plugins.manager().lock().unwrap_or_else(|p| p.into_inner());
        (manager.errors(), manager.theme())
    };
    let palette = protected_palette(&theme, PROTECTED_DEFAULT);
    let reviews = plugins.pending();
    let statuses = plugins.statuses();
    let problems = problem_lines(&errors, &statuses, &view.shared.load_report);
    let notice = view
        .shared
        .plugin_notice
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let cards: Vec<AnyElement> = reviews
        .iter()
        .enumerate()
        .map(|(i, r)| review_card(view, i, r, palette, cx))
        .collect();
    let rows: Vec<AnyElement> = statuses
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let name = s.name.clone();
            let state = match &s.state {
                PluginState::Running => "running".to_string(),
                PluginState::Failed(m) => format!("failed: {}", m.lines().next().unwrap_or("")),
                PluginState::ModifiedOutsideReview => {
                    "disabled: files changed outside review".to_string()
                }
                PluginState::DisabledSafeMode => "disabled: safe mode".to_string(),
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(s.name.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(state),
                )
                .when(s.trusted_edits, |r| {
                    r.child(div().text_xs().child("trusts future edits"))
                })
                .child(div().flex_1())
                .child(
                    Button::new(("rollback", i))
                        .outline()
                        .small()
                        .label("Roll back")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            rollback_plugin(this, &name, cx);
                        })),
                )
                .into_any_element()
        })
        .collect();

    v_flex()
        .gap_4()
        .when(view.shared.load_report.safe_mode, |c| {
            c.child(safe_mode_row(palette))
        })
        .when(!problems.is_empty(), |c| {
            c.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(hsla(palette.border))
                    .bg(hsla(palette.background))
                    .text_color(hsla(palette.foreground))
                    .text_sm()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Plugin problems"),
                    )
                    .children(problems),
            )
        })
        .when_some(notice, |c, text| {
            c.child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().danger)
                    .text_sm()
                    .child(div().flex_1().child(text))
                    .child(
                        Button::new("dismiss-notice")
                            .outline()
                            .small()
                            .label("Dismiss")
                            .on_click(cx.listener(|this, _, _, cx| dismiss_notice(this, cx))),
                    ),
            )
        })
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Waiting for your review"),
        )
        .children(cards)
        .when(reviews.is_empty(), |c| {
            c.child(muted("Nothing to review.", cx))
        })
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Installed plugins"),
        )
        .children(rows)
        .when(statuses.is_empty(), |c| {
            c.child(muted(
                "No user plugins are installed. The built-in interface is a bundled plugin and is not listed here.",
                cx,
            ))
        })
        .into_any_element()
}
