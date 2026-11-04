use super::SettingsView;
use discard_core::advanced::{self, AdvancedAction};
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
fn execute(view: &mut SettingsView, action: AdvancedAction, cx: &mut Context<SettingsView>) {
    view.advanced_confirm = None;
    if action == AdvancedAction::ResetSettings {
        let result = view
            .shared
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .reset()
            .map(|p| {
                p.map_or("Settings reset".into(), |p| {
                    format!("Backup: {}", p.display())
                })
            })
            .unwrap_or_else(|e| e.to_string());
        view.advanced_result = Some((action, result));
        cx.notify();
        return;
    }
    let memory = view.shared.memory.clone();
    let home = view.shared.discard_home.clone();
    let task = cx.background_spawn(async move {
        match action {
            AdvancedAction::RebuildMemoryIndex => memory
                .rebuild_index()
                .map(|s| format!("Indexed {} files", s.indexed))
                .map_err(|e| e.to_string()),
            AdvancedAction::ClearLogs => {
                advanced::clear_logs(&home).map(|n| format!("Cleared {n} files"))
            }
            AdvancedAction::ResetSettings => unreachable!(),
        }
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _ = this.update(cx, |this, cx| {
            if action == AdvancedAction::ClearLogs && result.is_ok() {
                this.logs.clear_cached(&this.shared.discard_home);
            }
            this.advanced_result = Some((action, result.unwrap_or_else(|e| e)));
            cx.notify();
        });
    })
    .detach();
}
pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let cards = [
        AdvancedAction::ResetSettings,
        AdvancedAction::RebuildMemoryIndex,
        AdvancedAction::ClearLogs,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, action)| {
        let mut card = v_flex()
            .gap_1()
            .child(action.title())
            .children(advanced::confirm_lines(action, &view.shared.discard_home));
        if view.advanced_confirm == Some(action) {
            card = card.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(("advanced-confirm", i))
                            .label("Confirm")
                            .on_click(cx.listener(move |this, _, _, cx| execute(this, action, cx))),
                    )
                    .child(
                        Button::new(("advanced-cancel", i))
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.advanced_confirm = None;
                                cx.notify();
                            })),
                    ),
            );
        } else {
            card = card.child(
                Button::new(("advanced-action", i))
                    .label(action.button_label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.advanced_confirm = Some(action);
                        cx.notify();
                    })),
            );
        }
        if let Some((_, result)) = view.advanced_result.as_ref().filter(|(a, _)| *a == action) {
            card = card.child(result.clone());
        }
        card
    })
    .collect::<Vec<_>>();
    v_flex().gap_4().children(cards).into_any_element()
}
