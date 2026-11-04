//! The General page: default provider, confirm while working, and the issues found in `settings.toml`.

use super::{SettingsView, soon_tag};
use crate::problems::theme_problem_lines;
use discard_core::bootstrap::PROVIDERS;
use discard_core::settings::{Change, SettingsIssue, SettingsStore};
use discard_core::settings_page::{
    LABEL_CONFIRM_WHILE_WORKING, LABEL_DEFAULT_PROVIDER, LABEL_REOPEN_LAST_CHANNEL,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// One line for each problem found while reading the file.
fn issue_lines(issues: &[SettingsIssue]) -> Vec<String> {
    issues
        .iter()
        .map(|issue| format!("{}: {}", issue.key, issue.message))
        .collect()
}

/// Write one change. The store reads the file again first, because `apply` does not check for edits made outside the window.
pub(super) fn apply_change(store: &mut SettingsStore, change: Change) -> Result<(), String> {
    store.reload_if_changed();
    store.apply(change).map_err(|e| e.to_string())
}

/// Apply a change from the page. An error shows inline until the next change succeeds.
fn apply(view: &mut SettingsView, change: Change, cx: &mut Context<SettingsView>) {
    let result = {
        let mut store = view
            .shared
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        apply_change(&mut store, change)
    };
    view.general_error = result.err();
    cx.notify();
}

pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let (general, issues) = {
        let store = view
            .shared
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        (
            store.settings().general.clone(),
            issue_lines(store.issues()),
        )
    };
    let theme_problems = theme_problem_lines(
        view.shared.appearance.lock().unwrap_or_else(|p| p.into_inner()).problems(),
    );
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

    let providers = h_flex()
        .gap_2()
        .children(PROVIDERS.iter().copied().enumerate().map(|(i, name)| {
            Button::new(("provider", i))
                .small()
                .label(name)
                .when(general.default_provider == name, |b| b.primary())
                .when(general.default_provider != name, |b| b.outline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    apply(this, Change::DefaultProvider(name.to_string()), cx);
                }))
        }));
    let this = cx.entity();
    let confirm = Switch::new("confirm-while-working")
        .checked(general.confirm_while_working)
        .accessibility_label(LABEL_CONFIRM_WHILE_WORKING)
        .on_click(move |on, _, cx| {
            let on = *on;
            this.update(cx, |view, cx| {
                apply(view, Change::ConfirmWhileWorking(on), cx);
            });
        });

    v_flex()
        .gap_4()
        .when(!issues.is_empty(), |c| {
            c.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_sm()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Problems in settings.toml"),
                    )
                    .children(issues),
            )
        })
        .when(!theme_problems.is_empty(), |c| {
            c.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_sm()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Problems with the theme"))
                    .children(theme_problems),
            )
        })
        .when_some(view.general_error.clone(), |c, error| {
            c.child(
                div()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_sm()
                    .child(error),
            )
        })
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(LABEL_DEFAULT_PROVIDER),
                )
                .child(providers)
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("Applies at next launch"),
                ),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(LABEL_CONFIRM_WHILE_WORKING),
                        )
                        .child(confirm),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("Confirm Quit and Safe Mode relaunch while a turn runs"),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .text_color(muted)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(LABEL_REOPEN_LAST_CHANNEL),
                )
                .child(soon_tag(cx)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Import by name: `use super::*` would bring in GPUI's `test` macro and shadow the built-in `#[test]`.
    use super::{apply_change, issue_lines};
    use discard_core::settings::{Change, SettingsIssue, SettingsStore};
    use std::path::PathBuf;

    /// A settings path in a fresh temp directory. The caller removes the directory.
    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("discard-ui-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.toml")
    }

    #[test]
    fn no_issues_gives_no_lines() {
        assert!(issue_lines(&[]).is_empty());
    }

    #[test]
    fn each_issue_becomes_one_line_with_its_key() {
        let issues = vec![SettingsIssue {
            key: "general.confirm_while_working".to_string(),
            message: "expected a boolean".to_string(),
        }];
        assert_eq!(
            issue_lines(&issues),
            vec!["general.confirm_while_working: expected a boolean".to_string()]
        );
    }

    #[test]
    fn a_valid_change_is_written_to_the_file() {
        let path = temp_path("valid");
        let mut store = SettingsStore::open(&path);
        apply_change(&mut store, Change::ConfirmWhileWorking(false)).unwrap();
        assert!(!store.settings().general.confirm_while_working);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("confirm_while_working = false"), "{text}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_unknown_provider_is_an_error_and_changes_nothing() {
        let path = temp_path("unknown");
        let mut store = SettingsStore::open(&path);
        let error =
            apply_change(&mut store, Change::DefaultProvider("nope".to_string())).unwrap_err();
        assert!(error.contains("nope"), "{error}");
        assert_eq!(store.settings().general.default_provider, "claude");
        assert!(!path.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_outside_edit_is_read_before_the_change_is_written() {
        let path = temp_path("outside");
        let mut store = SettingsStore::open(&path);
        std::fs::write(
            &path,
            "version = 1\n[general]\ndefault_provider = \"codex\"\n",
        )
        .unwrap();
        apply_change(&mut store, Change::ConfirmWhileWorking(false)).unwrap();
        assert_eq!(store.settings().general.default_provider, "codex");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("codex"), "{text}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
