use super::SettingsView;
use crate::shared::Shared;
use discard_core::{
    memory::Scope,
    memory_page::{self, ScopeSummary},
};
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
pub(super) fn load(shared: &Shared) -> Vec<ScopeSummary> {
    let store = shared.store.lock().unwrap_or_else(|p| p.into_inner());
    memory_page::summaries(
        &shared.memory,
        &store.list_agents().unwrap_or_default(),
        &store.list_projects().unwrap_or_default(),
    )
}
pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let rows = view
        .memory_rows
        .clone()
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let scope = row.scope.clone();
            let path = match &scope {
                Scope::Global => view.shared.discard_home.join("memory/global"),
                Scope::Project(id) => view.shared.discard_home.join("memory/projects").join(&id.0),
                Scope::Agent(id) => view.shared.discard_home.join("memory/agents").join(&id.0),
            };
            let buttons = row
                .files
                .into_iter()
                .enumerate()
                .map(|(j, file)| {
                    let scope = scope.clone();
                    Button::new(("memory-file", i * 1000 + j))
                        .label(file.path.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.memory_rows = load(&this.shared);
                            this.memory_text =
                                Some(match this.shared.memory.read(&scope, &file.path) {
                                    Ok(content) => {
                                        let mut text: String =
                                            content.text.chars().take(20_000).collect();
                                        if content.text.chars().count() > 20_000 {
                                            text.push_str(
                                                "\nText is limited to 20,000 characters.",
                                            );
                                        }
                                        text
                                    }
                                    Err(e) => e.to_string(),
                                });
                            cx.notify();
                        }))
                })
                .collect::<Vec<_>>();
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(format!("{} ({})", row.title, buttons.len()))
                        .child(
                            Button::new(("memory-reveal", i))
                                .label("Reveal")
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        ),
                )
                .children(buttons)
        })
        .collect::<Vec<_>>();
    v_flex()
        .gap_3()
        .children(rows)
        .children(view.memory_text.clone().map(|text| div().child(text)))
        .into_any_element()
}
