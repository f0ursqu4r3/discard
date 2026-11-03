//! Scripted provider for tests. It runs no process.

use async_trait::async_trait;
use discard_core::approval::Decision;
use discard_core::capability::{Capabilities, CapabilityMatrix};
use discard_core::event::{AgentStatus, ApprovalId, EventKind, RawEvent};
use discard_core::provider::{Provider, ProviderError, ProviderSession, SessionSpec, SetupStatus};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Step {
    Emit(EventKind),
    /// Emit an approval request, then wait until the host answers it.
    Approval { tool: String, input: Value },
    /// Emit an approval request, then exit without waiting (simulates a crash).
    ApprovalThenCrash { tool: String, input: Value },
    /// The process exits: the event stream ends.
    Crash,
    /// Never produce another event.
    Hang,
}

impl Step {
    pub fn working() -> Self {
        Step::Emit(EventKind::StatusChange { status: AgentStatus::Working })
    }
    pub fn idle() -> Self {
        Step::Emit(EventKind::StatusChange { status: AgentStatus::Idle })
    }
    pub fn text(s: &str) -> Self {
        Step::Emit(EventKind::TextDelta { text: s.into() })
    }
}

/// What the fake observed. Tests assert on this.
#[derive(Debug, Default, Clone)]
pub struct Probe {
    /// The resume handle each started session was given.
    pub started_with: Arc<Mutex<Vec<Option<String>>>>,
    pub responses: Arc<Mutex<Vec<(ApprovalId, Decision)>>>,
    pub cancelled: Arc<AtomicBool>,
    pub sent: Arc<Mutex<Vec<String>>>,
}

pub struct FakeProvider {
    matrix: CapabilityMatrix,
    script: Vec<Step>,
    setup: SetupStatus,
    resume_handle: Option<String>,
    pub probe: Probe,
}

impl FakeProvider {
    pub fn new(script: Vec<Step>) -> Self {
        Self::with_capabilities(script, true)
    }

    pub fn with_capabilities(script: Vec<Step>, interactive_approvals: bool) -> Self {
        Self {
            matrix: CapabilityMatrix {
                provider: "fake".into(),
                matrix_version: 1,
                cli_version: "fake".into(),
                capabilities: Capabilities {
                    interactive_approvals,
                    tool_call_visibility: true,
                    per_session_mcp: false,
                    session_resume: false,
                    cancellation: true,
                    pre_execution_blocking: interactive_approvals,
                },
            },
            script,
            setup: SetupStatus::Ready,
            resume_handle: None,
            probe: Probe::default(),
        }
    }

    /// What `check_setup` reports.
    pub fn with_setup(mut self, setup: SetupStatus) -> Self {
        self.setup = setup;
        self
    }

    /// The resume handle every session reports once started.
    pub fn with_resume_handle(mut self, handle: &str) -> Self {
        self.resume_handle = Some(handle.to_string());
        self
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> &str {
        "fake"
    }
    fn capabilities(&self) -> &CapabilityMatrix {
        &self.matrix
    }
    async fn check_setup(&self) -> SetupStatus {
        self.setup.clone()
    }
    async fn start_session(&self, spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError> {
        self.matrix.supports(spec.enforcement)?;
        self.probe.started_with.lock().expect("probe lock").push(spec.resume_handle.clone());
        let (decisions_tx, decisions_rx) = mpsc::unbounded_channel();
        Ok(Box::new(FakeSession {
            script: self.script.iter().cloned().collect(),
            probe: self.probe.clone(),
            started: false,
            decisions_tx,
            decisions_rx,
            waiting_for: None,
            resume_handle: self.resume_handle.clone(),
        }))
    }
}

pub struct FakeSession {
    script: VecDeque<Step>,
    probe: Probe,
    started: bool,
    decisions_tx: mpsc::UnboundedSender<(ApprovalId, Decision)>,
    decisions_rx: mpsc::UnboundedReceiver<(ApprovalId, Decision)>,
    waiting_for: Option<ApprovalId>,
    resume_handle: Option<String>,
}

#[async_trait]
impl ProviderSession for FakeSession {
    fn resume_handle(&self) -> Option<String> {
        self.resume_handle.clone()
    }
    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError> {
        self.probe.sent.lock().expect("probe lock").push(text.to_string());
        self.started = true;
        Ok(())
    }
    async fn next_event(&mut self) -> Option<RawEvent> {
        if !self.started {
            return None;
        }
        if self.waiting_for.is_some() {
            // Cancel-safe: if this future is dropped, `waiting_for` stays set.
            self.decisions_rx.recv().await?;
            self.waiting_for = None;
        }
        // Cancel-safety: a hang is never consumed, so a dropped future leaves it in place.
        if matches!(self.script.front(), Some(Step::Hang)) {
            return std::future::pending().await;
        }
        match self.script.pop_front()? {
            Step::Emit(kind) => Some(RawEvent { kind, raw: None }),
            Step::Approval { tool, input } => {
                let id = ApprovalId::new();
                self.waiting_for = Some(id.clone());
                Some(approval_event(id, tool, input))
            }
            Step::ApprovalThenCrash { tool, input } => {
                self.script.clear();
                Some(approval_event(ApprovalId::new(), tool, input))
            }
            Step::Crash => {
                self.script.clear();
                None
            }
            Step::Hang => None, // unreachable: handled above
        }
    }
    async fn respond_approval(&mut self, id: &ApprovalId, decision: Decision) -> Result<(), ProviderError> {
        self.probe.responses.lock().expect("probe lock").push((id.clone(), decision));
        self.decisions_tx.send((id.clone(), decision)).map_err(|e| ProviderError::Io(e.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), ProviderError> {
        self.probe.cancelled.store(true, Ordering::SeqCst);
        self.script.clear();
        Ok(())
    }
}

fn approval_event(id: ApprovalId, tool: String, input: Value) -> RawEvent {
    RawEvent {
        kind: EventKind::ApprovalRequest { call_id: format!("call-{id}"), approval_id: id, tool, input },
        raw: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_core::capability::EnforcementMode;
    use std::path::PathBuf;

    fn spec(enforcement: EnforcementMode) -> SessionSpec {
        SessionSpec {
            cwd: PathBuf::from("."),
            model: None,
            system_prompt: None,
            enforcement,
            resume_handle: None,
            mcp_config: None,
        }
    }

    #[tokio::test]
    async fn plays_its_script_in_order_after_a_user_message() {
        let p = FakeProvider::new(vec![Step::working(), Step::text("hi"), Step::idle()]);
        let mut s = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        s.send_user_message("go").await.unwrap();
        let mut kinds = Vec::new();
        for _ in 0..3 {
            kinds.push(s.next_event().await.unwrap().kind);
        }
        assert_eq!(kinds[1], EventKind::TextDelta { text: "hi".into() });
        assert_eq!(p.probe.sent.lock().unwrap().as_slice(), ["go"]);
    }

    #[tokio::test]
    async fn script_end_closes_the_stream() {
        let p = FakeProvider::new(vec![Step::text("x")]);
        let mut s = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        s.send_user_message("go").await.unwrap();
        assert!(s.next_event().await.is_some());
        assert!(s.next_event().await.is_none());
    }

    #[tokio::test]
    async fn approval_step_blocks_until_answered() {
        let p = FakeProvider::new(vec![
            Step::Approval { tool: "Bash".into(), input: serde_json::json!({}) },
            Step::text("after"),
        ]);
        let mut s = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        s.send_user_message("go").await.unwrap();
        let ev = s.next_event().await.unwrap();
        let EventKind::ApprovalRequest { approval_id, .. } = ev.kind else { panic!("expected approval request") };
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(50), s.next_event()).await;
        assert!(blocked.is_err(), "must block until the approval is answered");
        s.respond_approval(&approval_id, Decision::Approve).await.unwrap();
        assert_eq!(s.next_event().await.unwrap().kind, EventKind::TextDelta { text: "after".into() });
    }

    #[tokio::test]
    async fn fails_closed_when_it_cannot_enforce_host_prompts() {
        let p = FakeProvider::with_capabilities(vec![], false);
        let err = p.start_session(spec(EnforcementMode::HostPrompts)).await.err().unwrap();
        assert!(matches!(err, ProviderError::Policy(_)));
    }

    #[tokio::test]
    async fn a_hanging_step_survives_dropped_next_event_futures() {
        // The driver drops `next_event` futures when another branch wins. That must not consume steps.
        let p = FakeProvider::new(vec![Step::Hang]);
        let mut s = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        s.send_user_message("go").await.unwrap();
        for _ in 0..3 {
            let r = tokio::time::timeout(std::time::Duration::from_millis(10), s.next_event()).await;
            assert!(r.is_err(), "a hanging provider must stay hanging, not close the stream");
        }
    }

    #[tokio::test]
    async fn records_the_resume_handle_it_was_started_with_and_reports_its_own() {
        let p = FakeProvider::new(vec![]).with_resume_handle("sess-9");
        let mut sp = spec(EnforcementMode::HostPrompts);
        sp.resume_handle = Some("old".into());
        let s = p.start_session(sp).await.unwrap();
        assert_eq!(s.resume_handle().as_deref(), Some("sess-9"));
        assert_eq!(p.probe.started_with.lock().unwrap().as_slice(), [Some("old".to_string())]);
    }

    #[tokio::test]
    async fn setup_status_is_configurable() {
        let p = FakeProvider::new(vec![]).with_setup(SetupStatus::NotLoggedIn { login_command: "x login".into() });
        assert_eq!(p.check_setup().await, SetupStatus::NotLoggedIn { login_command: "x login".into() });
    }

    #[tokio::test]
    async fn cancel_is_recorded() {
        let p = FakeProvider::new(vec![Step::Hang]);
        let mut s = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        s.cancel().await.unwrap();
        assert!(p.probe.cancelled.load(Ordering::SeqCst));
    }
}
