//! State shared between engine updates and desktop commands.
use super::dto::*;
use discard_core::approval::Decision;
use discard_core::event::{AgentStatus, ApprovalId};
use discard_core::timeline::ApprovalView;
use discard_core::ui_state::{Command, SlotLabels, UiState, Update};
use discard_lua::PluginService;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc::UnboundedSender};

pub struct CoreInit {
    pub state: UiState,
    pub commands: UnboundedSender<Command>,
    pub project: String,
    pub channel: String,
    pub agent: String,
    pub provider: String,
}
struct State {
    ui: UiState,
    rev: u64,
    history_rev: u64,
    theme: ThemeDto,
}
pub struct AppCore {
    state: Mutex<State>,
    commands: UnboundedSender<Command>,
    project: String,
    channel: String,
    agent: String,
    provider: String,
    plugins: Mutex<Option<Arc<PluginService>>>,
    pub(crate) changed: Notify,
}
impl AppCore {
    pub fn new(init: CoreInit) -> Self {
        Self {
            state: Mutex::new(State {
                ui: init.state,
                rev: 0,
                history_rev: 0,
                theme: ThemeDto {
                    mode: "dark".into(),
                    tokens: BTreeMap::new(),
                    radius: None,
                    font_scale: 1.0,
                    spacing_scale: 1.0,
                    protected_bg: "#1e1e1e".into(),
                    protected_fg: "#f0f0f0".into(),
                    protected_border: "#ffaa00".into(),
                },
            }),
            commands: init.commands,
            project: init.project,
            channel: init.channel,
            agent: init.agent,
            provider: init.provider,
            plugins: Mutex::new(None),
            changed: Notify::new(),
        }
    }
    pub fn snapshot(&self) -> SnapshotDto {
        let (mut snapshot, slot_state) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ui = &state.ui;
            let rev = state.rev;
            let history_rev = state.history_rev;
            let theme = state.theme.clone();
            let labels = SlotLabels {
                project: self.project.clone(),
                channel: self.channel.clone(),
                agent_name: self.agent.clone(),
                provider: self.provider.clone(),
            };
            let slot_state = ui.slot_state(&labels);
            let snapshot = SnapshotDto {
                rev,
                history_rev,
                project: self.project.clone(),
                channel: self.channel.clone(),
                agent: self.agent.clone(),
                provider: self.provider.clone(),
                agent_status: match ui.agent_status() {
                    AgentStatus::Idle => "idle",
                    AgentStatus::Working => "working",
                    AgentStatus::AwaitingApproval => "awaiting_approval",
                    AgentStatus::Offline => "offline",
                }
                .into(),
                turn_running: ui.turn_running(),
                notice: ui.notice().map(str::to_owned),
                live_blocks: ui.live_blocks().to_vec(),
                prompts: ui
                    .prompts()
                    .iter()
                    .map(|p| PromptDto {
                        approval_id: p.approval_id.to_string(),
                        tool: p.tool.clone(),
                        summary: p.summary.clone(),
                        view: match p.view {
                            ApprovalView::Pending => "pending",
                            ApprovalView::Approved => "approved",
                            ApprovalView::Denied => "denied",
                            ApprovalView::Stale => "stale",
                        }
                        .into(),
                    })
                    .collect(),
                pending_approvals: ui.pending_approvals(),
                unread: false,
                archived: false,
                archive_busy: false,
                member_list_visible: true,
                panels: PanelsDto {
                    channel_list: 240.0,
                    member_list: 200.0,
                },
                branches: vec![],
                slots: BTreeMap::new(),
                banners: BannersDto {
                    review_count: 0,
                    problem_lines: vec![],
                    confirm: None,
                    archive: None,
                    safe_mode: false,
                },
                theme,
            };
            (snapshot, slot_state)
        };
        let plugins = self.plugins.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots = BTreeMap::new();
        if let Some(plugins) = plugins.as_ref() {
            let mut manager = plugins.manager().lock().unwrap_or_else(|e| e.into_inner());
            for slot in ["channel_header", "member_list"] {
                if let Some(rendered) = manager.render_slot(slot, &slot_state) {
                    slots.insert(
                        slot.to_string(),
                        SlotDto {
                            plugin: rendered.plugin,
                            element: rendered.element,
                        },
                    );
                }
            }
        }
        snapshot.slots = slots;
        snapshot
    }

    pub fn set_plugins(&self, plugins: Arc<PluginService>) {
        *self.plugins.lock().unwrap_or_else(|e| e.into_inner()) = Some(plugins);
    }
    pub fn set_theme(&self, theme: ThemeDto) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.theme != theme {
            s.theme = theme;
            s.rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn history(&self) -> Vec<MessageDto> {
        history_dto(
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .ui
                .history(),
        )
    }
    pub fn send(&self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.ui.turn_running() {
            return;
        }
        if self.commands.send(Command::Send(text.clone())).is_ok() {
            s.ui.user_sent(&text);
            s.rev += 1;
            s.history_rev += 1;
            self.changed.notify_one();
        }
    }
    pub fn decide(&self, approval_id: String, decision: String) {
        let decision = match decision.as_str() {
            "approve" => Decision::Approve,
            "deny" => Decision::Deny,
            "always_allow" => Decision::AlwaysAllow,
            _ => return,
        };
        let _ = self.commands.send(Command::Decide {
            approval_id: ApprovalId(approval_id),
            decision,
        });
    }
    pub fn cancel(&self) {
        let _ = self.commands.send(Command::Cancel);
    }
    pub fn apply(&self, update: Update) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let old_len = s.ui.history().len();
        s.ui.apply(update);
        s.rev += 1;
        if old_len != s.ui.history().len() {
            s.history_rev += 1;
        }
    }
}
