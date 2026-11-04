//! The keybindings settings page.
use super::{SettingsView, general};
use crate::dispatch;
use discard_core::{
    action::Action,
    keymap::{self, KeyRule},
    settings::Change,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

fn apply(view: &mut SettingsView, action: Action, change: Change, cx: &mut Context<SettingsView>) {
    let result = {
        let mut store = view
            .shared
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        general::apply_change(&mut store, change)
    };
    view.key_error = result.err().map(|error| (action, error));
    if view.key_error.is_none() {
        view.key_saved = true;
    }
    cx.notify();
}
pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let map = view
        .shared
        .settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .settings()
        .keymap
        .clone();
    let rows = keymap::rows(&map)
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let action = row.action;
            let capture = view.key_capture == Some(action);
            let error = view
                .key_error
                .as_ref()
                .filter(|(a, _)| *a == action)
                .map(|(_, e)| e.clone());
            let chord = row
                .chord
                .as_ref()
                .map(keymap::display_chord)
                .unwrap_or_else(|| "Not set".into());
            let mut line = h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().child(action.label()))
                .child(if capture {
                    "Press a key (Escape to cancel)".to_string()
                } else {
                    chord
                });
            if row.rule == KeyRule::Locked {
                line = line.child("Locked");
            } else {
                line = line.child(
                    Button::new(("record-key", i))
                        .small()
                        .label("Record")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.key_capture = Some(action);
                            this.key_error = None;
                            this.key_focus.focus(window, cx);
                            cx.notify();
                        })),
                );
                if row.custom {
                    line = line.child(
                        Button::new(("reset-key", i))
                            .small()
                            .label("Reset")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                apply(this, action, Change::ResetKeybinding(action), cx)
                            })),
                    );
                }
                if row.rule == KeyRule::Optional && row.chord.is_some() {
                    line = line.child(
                        Button::new(("unbind-key", i))
                            .small()
                            .label("Unbind")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                apply(
                                    this,
                                    action,
                                    Change::Keybinding {
                                        action,
                                        chord: None,
                                    },
                                    cx,
                                )
                            })),
                    );
                }
            }
            v_flex().gap_1().child(line).when_some(error, |e, error| {
                e.child(div().text_color(cx.theme().danger).child(error))
            })
        })
        .collect::<Vec<_>>();
    v_flex()
        .track_focus(&view.key_focus)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            let Some(action) = this.key_capture.take() else {
                return;
            };
            cx.stop_propagation();
            if event.keystroke.key == "escape" {
                cx.notify();
                return;
            }
            let text = event.keystroke.unparse(OperatingSystem::Mac);
            apply(
                this,
                action,
                Change::Keybinding {
                    action,
                    chord: Some(text),
                },
                cx,
            );
        }))
        .gap_3()
        .children(rows)
        .when(view.key_saved, |page| {
            page.child("Applies at next launch").child(
                Button::new("key-relaunch")
                    .label("Relaunch")
                    .on_click(|_, _, cx| dispatch::relaunch_same_mode(cx)),
            )
        })
        .into_any_element()
}
