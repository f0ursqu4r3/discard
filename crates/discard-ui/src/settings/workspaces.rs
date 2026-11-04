use super::SettingsView;
use discard_core::{model::WorkspaceMode, workspace_page};
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
pub(super) fn render(view: &mut SettingsView, _cx: &mut Context<SettingsView>) -> AnyElement {
    let rows = match workspace_page::rows(
        &view.shared.store.lock().unwrap_or_else(|p| p.into_inner()),
        &view.shared.channel_id,
    ) {
        Ok(rows) => rows,
        Err(e) => return div().child(e.to_string()).into_any_element(),
    };
    let cards = rows
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let path = std::path::PathBuf::from(&p.path);
            let mut card = v_flex().gap_1().child(
                h_flex()
                    .gap_2()
                    .child(format!(
                        "{} · {}{}",
                        p.name,
                        p.path,
                        if p.exists { "" } else { " · missing" }
                    ))
                    .child(
                        Button::new(("project-reveal", i))
                            .label("Reveal")
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                    ),
            );
            for (j, c) in p.channels.into_iter().enumerate() {
                let mode = match c.mode {
                    WorkspaceMode::MainCheckout => "Main checkout",
                    WorkspaceMode::Worktree => "Worktree",
                };
                card = card.child(format!(
                    "{} · {}{}{}",
                    c.name,
                    mode,
                    if c.archived { " · Archived" } else { "" },
                    if c.current { " · Current" } else { "" }
                ));
                if let Some(w) = c.worktree {
                    let path = std::path::PathBuf::from(&w.path);
                    card = card.child(
                        h_flex()
                            .gap_2()
                            .child(format!(
                                "{} · {}{}",
                                w.branch,
                                w.path,
                                if w.exists { "" } else { " · missing" }
                            ))
                            .child(
                                Button::new(("worktree-reveal", i * 1000 + j))
                                    .label("Reveal")
                                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
                            ),
                    );
                }
            }
            card
        })
        .collect::<Vec<_>>();
    v_flex().gap_4().children(cards).into_any_element()
}
