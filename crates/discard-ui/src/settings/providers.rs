use super::SettingsView;
use discard_core::capability::HookMode;
use discard_core::provider_page::{SetupLine, provider_card};
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

pub(super) fn render(view: &SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let entity = cx.entity();
    let cards = view.shared.providers.iter().enumerate().map(|(index, info)| {
        let setup = info.setup.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let card = provider_card(&info.id, &info.matrix, setup.as_ref());
        let (status, command) = match card.setup {
            SetupLine::Checking => ("Checking", None), SetupLine::Ready => ("Ready", None),
            SetupLine::NotInstalled { install_hint } => ("Not installed", Some(install_hint)),
            SetupLine::NotLoggedIn { login_command } => ("Not logged in", Some(login_command)),
        };
        let id = info.id.clone();
        let recheck = view.shared.recheck.clone();
        let entity = entity.clone();
        v_flex().gap_2().p_3().border_1().rounded_md()
            .child(h_flex().gap_3().child(div().flex_1().font_weight(FontWeight::BOLD).child(card.id)).child(Button::new(("provider-recheck", index)).label("Re-check").on_click(move |_, _, cx| { recheck(&id); entity.update(cx, |_, cx| cx.notify()); })))
            .child(format!("Setup status: {status}"))
            .children(command.map(|text| div().font_family("Menlo").child(text)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Capabilities"))
            .children(card.capabilities.into_iter().map(|row| div().child(format!("{}: {}", row.label, if row.supported { "Yes" } else { "No" }))))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Hook enforcement"))
            .children(card.hooks.into_iter().map(|row| div().child(format!("{}: {}", row.label, match row.mode { HookMode::Active => "Active", HookMode::ObserveOnly => "Observe only", HookMode::Unsupported => "Unsupported" }))))
            .children(card.ask_policy_blocked.map(|reason| div().child(reason)))
    }).collect::<Vec<_>>();
    v_flex().gap_4().children(cards).into_any_element()
}
