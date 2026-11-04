use super::SettingsView;
use crate::dispatch;
use discard_core::agent::AgentConfig;
use discard_core::agent_edit::{self, AgentDraft};
use discard_core::capability::ApprovalPolicy;
use discard_core::model::AgentId;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) struct Editor {
    pub model: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub selected: Option<(AgentId, AgentConfig, AgentDraft)>,
    confirm: bool,
    saved: bool,
    error: Option<String>,
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let model = cx.new(|cx| InputState::new(window, cx));
        let prompt = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8));
        Self { model, prompt, selected: None, confirm: false, saved: false, error: None }
    }
    pub fn changed(&mut self) { self.confirm = false; self.saved = false; self.error = None; }
}

fn save(view: &mut SettingsView, confirmed: bool, cx: &mut Context<SettingsView>) {
    let Some((id, cfg, draft)) = &view.agent_editor.selected else { return; };
    let Some(provider) = view.shared.providers.iter().find(|p| p.id == cfg.provider) else {
        view.agent_editor.error = Some("Provider is unavailable".into()); cx.notify(); return;
    };
    let result = agent_edit::save(&view.shared.store.lock().unwrap_or_else(|p| p.into_inner()), id, draft, &provider.matrix, confirmed);
    match result {
        Ok(cfg) => { let draft = AgentDraft::from(&cfg); view.agent_editor.selected = Some((id.clone(), cfg, draft)); view.agent_editor.confirm = false; view.agent_editor.saved = true; view.agent_editor.error = None; }
        Err(agent_edit::SaveError::NeedsConfirm) => view.agent_editor.confirm = true,
        Err(error) => view.agent_editor.error = Some(error.to_string()),
    }
    cx.notify();
}

pub(super) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let agents = view.shared.store.lock().unwrap_or_else(|p| p.into_inner()).list_agents();
    let agents = match agents { Ok(agents) => agents, Err(e) => return div().child(e.to_string()).into_any_element() };
    let buttons = agents.into_iter().enumerate().map(|(i, (id, cfg))| {
        let label = format!("{} · {}", cfg.name, cfg.provider);
        Button::new(("agent-select", i)).label(label).on_click(cx.listener(move |this, _, window, cx| {
            let draft = AgentDraft::from(&cfg);
            this.agent_editor.model.update(cx, |input, cx| input.set_value(draft.model.clone().unwrap_or_default(), window, cx));
            this.agent_editor.prompt.update(cx, |input, cx| input.set_value(draft.system_prompt.clone(), window, cx));
            this.agent_editor.selected = Some((id.clone(), cfg.clone(), draft));
            this.agent_editor.changed(); cx.notify();
        }))
    }).collect::<Vec<_>>();
    let mut page = v_flex().gap_3().child(h_flex().gap_2().children(buttons));
    let Some((_, cfg, draft)) = view.agent_editor.selected.clone() else { return page.into_any_element(); };
    let Some(provider) = view.shared.providers.iter().find(|p| p.id == cfg.provider) else { return page.child("Provider is unavailable").into_any_element(); };
    let review = agent_edit::review(&cfg, &draft, &provider.matrix);
    let choices = agent_edit::policy_choices(&provider.matrix);
    let policy_buttons = choices.into_iter().enumerate().map(|(i, choice)| {
        let label = match choice.policy { ApprovalPolicy::Ask => "Ask before tool calls", ApprovalPolicy::DenyAll => "Deny all" };
        let policy = choice.policy;
        let reason = choice.reason;
        v_flex().child(Button::new(("agent-policy", i)).label(label).disabled(!choice.enabled).when(draft.policy == policy, |b| b.primary()).when(draft.policy != policy, |b| b.outline()).on_click(cx.listener(move |this, _, _, cx| {
            if let Some((_, _, draft)) = &mut this.agent_editor.selected { draft.policy = policy; }
            this.agent_editor.changed(); cx.notify();
        }))).children(reason.map(|reason| div().text_sm().child(reason)))
    }).collect::<Vec<_>>();
    let entity = cx.entity();
    let toggle = Switch::new("agent-plugin-write").checked(draft.plugin_write).on_click(move |on, _, cx| {
        let on = *on;
        entity.update(cx, |this, cx| { if let Some((_, _, draft)) = &mut this.agent_editor.selected { draft.plugin_write = on; } this.agent_editor.changed(); cx.notify(); });
    });
    page = page.child(format!("Agent: {} · {}", cfg.name, cfg.provider)).child("Model").child(Input::new(&view.agent_editor.model)).child("System prompt").child(Textarea::new(&view.agent_editor.prompt)).child("Approval policy").child(h_flex().gap_2().children(policy_buttons)).child(h_flex().gap_2().child("plugin.write").child(toggle))
        .children(review.changes.into_iter().map(|change| div().child(format!("{}: {} → {}", change.field, change.before, change.after))))
        .children(review.blocked.map(|reason| div().child(reason)))
        .children(view.agent_editor.error.clone().map(|error| div().child(error)))
        .child(Button::new("agent-save").label("Save").on_click(cx.listener(|this, _, _, cx| save(this, false, cx))));
    if view.agent_editor.confirm { page = page.child("This lets the agent do more without asking.").child(Button::new("agent-confirm").danger().label("Confirm and save").on_click(cx.listener(|this, _, _, cx| save(this, true, cx)))); }
    if view.agent_editor.saved { page = page.child("Applies at next launch").child(Button::new("agent-relaunch").label("Relaunch").on_click(|_, _, cx| dispatch::relaunch_same_mode(cx))); }
    page.into_any_element()
}
