//! Engine: turns UI commands into provider turns and persists the result.

use crate::approval::{ApprovalState, Decider};
use crate::capability::ApprovalPolicy;
use crate::driver::{DecisionMsg, TurnContext, TurnInputs, run_turn};
use crate::event::{Sequencer, SessionId};
use crate::model::{Block, ChannelId, Message, MessageId, Role};
use crate::provider::{Provider, ProviderError, SessionSpec, SetupStatus};
use crate::redact::redact;
use crate::store::Store;
use crate::timeline::{ApprovalView, Timeline};
use crate::ui_state::{Command, Update};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::warn;

pub struct Engine {
    pub store: Arc<Mutex<Store>>,
    pub provider: Arc<dyn Provider>,
    pub channel_id: ChannelId,
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub policy: ApprovalPolicy,
}

fn send(updates: &mpsc::UnboundedSender<Update>, update: Update) {
    // The UI may have closed already. Nothing else to do then.
    let _ = updates.send(update);
}

fn policy_name(policy: ApprovalPolicy) -> &'static str {
    match policy {
        ApprovalPolicy::Ask => "ask",
        ApprovalPolicy::DenyAll => "deny_all",
    }
}

impl Engine {
    /// Handle commands until the UI drops its sender.
    pub async fn run(
        self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        updates: mpsc::UnboundedSender<Update>,
    ) {
        let mut sequencer = Sequencer::new();
        match crate::driver::lock(&self.store).last_seq(&self.session_id) {
            Ok(last) => sequencer.resume(&self.session_id, last),
            Err(e) => warn!(error = %e, "could not read the last event sequence"),
        }
        while let Some(command) = commands.recv().await {
            // Decisions and cancels have no meaning without a running turn.
            if let Command::Send(text) = command {
                self.turn(&mut sequencer, text, &mut commands, &updates).await;
            }
        }
    }

    fn store_message(&self, role: Role, blocks: Vec<Block>) {
        let message = Message { id: MessageId::new(), channel_id: self.channel_id.clone(), role, blocks };
        if let Err(e) = crate::driver::lock(&self.store).insert_message(&message) {
            warn!(error = %e, "could not store a message");
        }
    }

    async fn turn(
        &self,
        sequencer: &mut Sequencer,
        text: String,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        updates: &mpsc::UnboundedSender<Update>,
    ) {
        self.store_message(Role::User, vec![Block::Markdown { text: text.clone() }]);

        let enforcement = match self.provider.capabilities().enforcement_for(self.policy) {
            Ok(mode) => mode,
            Err(e) => {
                send(updates, Update::Error(e.to_string()));
                send(updates, Update::TurnFinished(crate::driver::TurnOutcome::Failed));
                return;
            }
        };

        match self.provider.check_setup().await {
            SetupStatus::Ready => {}
            SetupStatus::NotInstalled { install_hint } => {
                send(updates, Update::SetupNeeded(install_hint));
                return;
            }
            SetupStatus::NotLoggedIn { login_command } => {
                send(
                    updates,
                    Update::SetupNeeded(format!("Run `{login_command}` in a terminal, then send your message again.")),
                );
                return;
            }
        }

        let resume_handle = crate::driver::lock(&self.store).resume_handle(&self.session_id).ok().flatten();
        let spec = SessionSpec {
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            enforcement,
            resume_handle,
            mcp_config: None,
        };
        let mut session = match self.provider.start_session(spec).await {
            Ok(session) => session,
            Err(e) => {
                let message = match e {
                    ProviderError::ResumeRejected(why) => format!(
                        "The provider could not resume this session ({}). Start a new session to continue.",
                        redact(&why)
                    ),
                    other => redact(&other.to_string()),
                };
                send(updates, Update::Error(message));
                send(updates, Update::TurnFinished(crate::driver::TurnOutcome::Failed));
                return;
            }
        };

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (decisions_tx, decisions_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
        let ctx = TurnContext {
            store: &self.store,
            sequencer,
            session_id: &self.session_id,
            policy_name: policy_name(self.policy),
            updates: events_tx,
        };
        let driver = run_turn(ctx, &mut *session, &text, TurnInputs { decisions: decisions_rx, cancel: cancel_rx });

        let mut timeline = Timeline::new();
        let pump = async {
            let mut commands_open = true;
            loop {
                tokio::select! {
                    event = events_rx.recv() => {
                        let Some(event) = event else { break };
                        timeline.apply(&event.kind);
                        send(updates, Update::Event(Box::new(event)));
                    }
                    command = commands.recv(), if commands_open => match command {
                        None => commands_open = false,
                        Some(Command::Decide { approval_id, decision }) => {
                            let pending = timeline
                                .prompts()
                                .iter()
                                .any(|p| p.approval_id == approval_id && p.view == ApprovalView::Pending);
                            let _ = decisions_tx.send(DecisionMsg {
                                approval_id: approval_id.clone(),
                                decision,
                                decider: Decider::User,
                            });
                            if pending {
                                let state = ApprovalState::Decided(decision);
                                timeline.mark_approval(&approval_id, state);
                                send(updates, Update::ApprovalState { approval_id, state });
                            }
                        }
                        Some(Command::Cancel) => {
                            let _ = cancel_tx.send(());
                        }
                        Some(Command::Send(_)) => send(
                            updates,
                            Update::Error("A turn is already running. Wait for it to finish or cancel it.".into()),
                        ),
                    },
                }
            }
        };
        let (result, ()) = tokio::join!(driver, pump);

        if !timeline.blocks().is_empty() {
            self.store_message(Role::Agent, timeline.blocks().to_vec());
        }
        match result {
            Ok((_turn, outcome)) => {
                if let Some(handle) = session.resume_handle()
                    && let Err(e) = crate::driver::lock(&self.store).set_resume_handle(&self.session_id, Some(&handle))
                {
                    warn!(error = %e, "could not save the resume handle");
                }
                send(updates, Update::TurnFinished(outcome));
            }
            Err(e) => {
                send(updates, Update::Error(redact(&e.to_string())));
                send(updates, Update::TurnFinished(crate::driver::TurnOutcome::Failed));
            }
        }
    }
}
