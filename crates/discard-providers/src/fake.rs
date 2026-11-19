//! Scripted provider for tests. It runs no process.

use async_trait::async_trait;
use discard_core::approval::Decision;
use discard_core::capability::{Capabilities, CapabilityMatrix};
use discard_core::event::{AgentStatus, ApprovalId, EventKind, RawEvent};
use discard_core::provider::{Provider, ProviderError, ProviderSession, SandboxAutorun, SessionSpec, SetupStatus, TextRequest};
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
    /// Create this directory (and its parents), emit nothing, and play the next step. Simulates a command
    /// that writes to the worktree during the turn.
    CreateDir(std::path::PathBuf),
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
    pub text_requests: Arc<Mutex<Vec<TextRequest>>>,
    /// Every session spec the provider was started with.
    pub specs: Arc<Mutex<Vec<SessionSpec>>>,
    /// The resume handle each started session was given.
    pub started_with: Arc<Mutex<Vec<Option<String>>>>,
    /// The `resumable` flag each started session was given.
    pub started_resumable: Arc<Mutex<Vec<bool>>>,
    pub responses: Arc<Mutex<Vec<(ApprovalId, Decision)>>>,
    pub cancelled: Arc<AtomicBool>,
    pub sent: Arc<Mutex<Vec<String>>>,
    pub images: Arc<Mutex<Vec<Vec<discard_core::attachments::Attachment>>>>,
}

pub struct FakeProvider {
    generated_text: Option<Result<String, String>>,
    main_checkout_writes: bool,
    workspace_error: Option<String>,
    sandbox_autorun: Option<SandboxAutorun>,
    matrix: CapabilityMatrix,
    script: Vec<Step>,
    /// The scripts of the sessions after the first, in order. A session with no script of its own plays `script`.
    later_scripts: Vec<Vec<Step>>,
    sessions_started: std::sync::atomic::AtomicUsize,
    setup: SetupStatus,
    resume_handle: Option<String>,
    notice: Option<String>,
    rejects_resume: bool,
    rejects_resume_on_first_send: bool,
    /// How long the rejected first message takes, as the Claude CLI's start wait does.
    first_send_delay: std::time::Duration,
    /// Start calls, counted from 0 over every call, that fail with this protocol error.
    failed_starts: Vec<(usize, String)>,
    compaction: bool,
    pub probe: Probe,
}

impl FakeProvider {
    pub fn new(script: Vec<Step>) -> Self {
        Self::with_capabilities(script, true)
    }

    pub fn with_capabilities(script: Vec<Step>, interactive_approvals: bool) -> Self {
        Self {
            generated_text: None,
            main_checkout_writes: false,
            workspace_error: None,
            sandbox_autorun: None,
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
            later_scripts: Vec::new(),
            sessions_started: std::sync::atomic::AtomicUsize::new(0),
            setup: SetupStatus::Ready,
            resume_handle: None,
            notice: None,
            rejects_resume: false,
            rejects_resume_on_first_send: false,
            first_send_delay: std::time::Duration::ZERO,
            failed_starts: Vec::new(),
            compaction: false,
            probe: Probe::default(),
        }
    }

    /// The script of the next session after the ones that are already scripted: the first session plays the script
    /// of `new`, the second plays the first script given here, and so on.
    pub fn then_session(mut self, script: Vec<Step>) -> Self { self.later_scripts.push(script); self }

    pub fn with_notice(mut self, notice: impl Into<String>) -> Self { self.notice = Some(notice.into()); self }

    /// What `sandbox_autorun()` reports. Without this, the fake keeps the trait default (unsupported).
    pub fn with_sandbox_autorun(mut self, answer: SandboxAutorun) -> Self { self.sandbox_autorun = Some(answer); self }

    pub fn with_workspace_error(mut self, error: &str) -> Self { self.workspace_error = Some(error.into()); self }

    pub fn with_generated_text(mut self, result: Result<String, String>) -> Self {
        self.generated_text = Some(result);
        self
    }

    pub fn with_main_checkout_writes(mut self, supported: bool) -> Self { self.main_checkout_writes = supported; self }

    /// What `check_setup` reports.
    pub fn with_setup(mut self, setup: SetupStatus) -> Self {
        self.setup = setup;
        self
    }

    /// Whether the provider reports tool calls to the host.
    pub fn with_tool_call_visibility(mut self, visible: bool) -> Self {
        self.matrix.capabilities.tool_call_visibility = visible;
        self
    }

    /// Starting a session with a resume handle fails with `ResumeRejected`. Starting without one works.
    pub fn with_rejecting_resume(mut self) -> Self { self.rejects_resume = true; self }

    /// A session started with a resume handle starts, then its first message fails with `ResumeRejected`, as the
    /// Claude CLI does. A session started without one works.
    pub fn with_rejecting_resume_on_first_send(mut self) -> Self { self.rejects_resume_on_first_send = true; self }

    /// As `with_rejecting_resume_on_first_send`, but the rejected message takes `delay` to fail.
    pub fn with_slow_rejecting_resume_on_first_send(mut self, delay: std::time::Duration) -> Self {
        self.first_send_delay = delay;
        self.with_rejecting_resume_on_first_send()
    }

    /// Start call `index` (from 0, counting every call) fails with a protocol error that holds `message`.
    pub fn with_failed_start(mut self, index: usize, message: &str) -> Self { self.failed_starts.push((index, message.into())); self }

    /// Every started session reports that the CLI compacted it.
    pub fn with_compaction(mut self) -> Self { self.compaction = true; self }

    /// The resume handle every session reports once started.
    pub fn with_resume_handle(mut self, handle: &str) -> Self {
        self.resume_handle = Some(handle.to_string());
        self
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn supports_main_checkout_writes(&self) -> bool { self.main_checkout_writes }
    fn sandbox_autorun(&self) -> SandboxAutorun {
        self.sandbox_autorun.clone().unwrap_or_else(|| SandboxAutorun::Unsupported("this provider has not been verified for autorun".into()))
    }
    fn id(&self) -> &str {
        "fake"
    }
    fn capabilities(&self) -> &CapabilityMatrix {
        &self.matrix
    }
    async fn check_setup(&self) -> SetupStatus {
        self.setup.clone()
    }
    async fn generate_text(&self, request: TextRequest) -> Result<String, ProviderError> {
        self.probe.text_requests.lock().expect("probe lock").push(request);
        match &self.generated_text {
            Some(result) => result.clone().map_err(ProviderError::Protocol),
            None => Err(ProviderError::Setup("This provider cannot generate text.".into())),
        }
    }
    async fn start_session(&self, spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError> {
        self.matrix.supports(spec.enforcement)?;
        let call = {
            let mut started_with = self.probe.started_with.lock().expect("probe lock");
            started_with.push(spec.resume_handle.clone());
            started_with.len() - 1
        };
        self.probe.started_resumable.lock().expect("probe lock").push(spec.resumable);
        self.probe.specs.lock().expect("probe lock").push(spec.clone());
        if self.rejects_resume && spec.resume_handle.is_some() { return Err(ProviderError::ResumeRejected("the fake rejects every resume handle".into())); }
        if let Some((_, message)) = self.failed_starts.iter().find(|(index, _)| *index == call) { return Err(ProviderError::Protocol(message.clone())); }
        if spec.mode == discard_core::provider::SessionMode::Workspace && let Some(error) = &self.workspace_error { return Err(ProviderError::SandboxUnavailable(error.clone())); }
        let (decisions_tx, decisions_rx) = mpsc::unbounded_channel();
        let number = self.sessions_started.fetch_add(1, Ordering::SeqCst);
        let script = if number == 0 { &self.script } else { self.later_scripts.get(number - 1).unwrap_or(&self.script) };
        Ok(Box::new(FakeSession {
            script: script.iter().cloned().collect(),
            probe: self.probe.clone(),
            started: false,
            decisions_tx,
            decisions_rx,
            waiting_for: None,
            resume_handle: self.resume_handle.clone(),
            notice: self.notice.clone(),
            compacted: self.compaction,
            reject_first_send: self.rejects_resume_on_first_send && spec.resume_handle.is_some(),
            first_send_delay: self.first_send_delay,
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
    notice: Option<String>,
    compacted: bool,
    /// The first message fails with `ResumeRejected`.
    reject_first_send: bool,
    first_send_delay: std::time::Duration,
}

#[async_trait]
impl ProviderSession for FakeSession {
    fn compacted(&self) -> bool { self.compacted }
    fn notice(&self) -> Option<String> { self.notice.clone() }
    fn resume_handle(&self) -> Option<String> {
        self.resume_handle.clone()
    }
    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError> {
        self.probe.sent.lock().expect("probe lock").push(text.to_string());
        if std::mem::take(&mut self.reject_first_send) {
            tokio::time::sleep(self.first_send_delay).await;
            return Err(ProviderError::ResumeRejected("the fake rejects the resumed session".into()));
        }
        self.started = true;
        Ok(())
    }
    async fn send_user_message_with_images(&mut self, text: &str, images: &[discard_core::attachments::Attachment]) -> Result<(), ProviderError> {
        self.probe.images.lock().expect("probe lock").push(images.to_vec());
        self.send_user_message(text).await
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
        // A step that only changes the disk produces no event: play it, then look at the next one.
        while let Some(Step::CreateDir(path)) = self.script.front() {
            let _ = std::fs::create_dir_all(path);
            self.script.pop_front();
        }
        // Cancel-safety: a hang is never consumed, so a dropped future leaves it in place.
        if matches!(self.script.front(), Some(Step::Hang)) {
            return std::future::pending().await;
        }
        match self.script.pop_front()? {
            Step::CreateDir(_) => None, // unreachable: handled above
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
        kind: EventKind::ApprovalRequest { kind: Default::default(), paths: input.get("file_path").and_then(Value::as_str).map(|p| vec![p.to_owned()]).unwrap_or_default(), call_id: format!("call-{id}"), approval_id: id, tool, input },
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
            reasoning_effort: None,
            mode: discard_core::provider::SessionMode::Coding,
            cwd: PathBuf::from("."),
            model: None,
            system_prompt: None,
            enforcement,
            resume_handle: None,
            mcp_config: None,
            sandbox_autorun: false,
            resumable: false,
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
    async fn records_resumable_flag() {
        let p = FakeProvider::new(vec![]);
        let mut sp = spec(EnforcementMode::HostPrompts);
        sp.resumable = true;
        p.start_session(sp).await.unwrap();
        p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        assert_eq!(p.probe.started_resumable.lock().unwrap().as_slice(), [true, false]);
    }

    #[tokio::test]
    async fn a_rejecting_fake_refuses_a_handle_and_accepts_none() {
        let p = FakeProvider::new(vec![]).with_rejecting_resume();
        let mut sp = spec(EnforcementMode::HostPrompts);
        sp.resume_handle = Some("old".into());
        assert!(matches!(p.start_session(sp).await, Err(ProviderError::ResumeRejected(_))));
        assert!(p.start_session(spec(EnforcementMode::HostPrompts)).await.is_ok());
        assert_eq!(p.probe.started_with.lock().unwrap().as_slice(), [Some("old".to_string()), None]);
    }

    #[tokio::test]
    async fn a_first_send_rejecting_fake_starts_then_rejects_the_first_message() {
        let p = FakeProvider::new(vec![]).with_rejecting_resume_on_first_send().with_failed_start(2, "broke");
        let mut sp = spec(EnforcementMode::HostPrompts);
        sp.resume_handle = Some("old".into());
        let mut s = p.start_session(sp).await.unwrap();
        assert!(matches!(s.send_user_message("go").await, Err(ProviderError::ResumeRejected(_))));
        let mut fresh = p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        assert!(fresh.send_user_message("go").await.is_ok());
        assert!(matches!(p.start_session(spec(EnforcementMode::HostPrompts)).await, Err(ProviderError::Protocol(m)) if m == "broke"));
    }

    #[tokio::test]
    async fn the_fake_can_report_a_compaction() {
        let plain = FakeProvider::new(vec![]).start_session(spec(EnforcementMode::HostPrompts)).await.unwrap();
        assert!(!plain.compacted());
        let p = FakeProvider::new(vec![]).with_compaction();
        assert!(p.start_session(spec(EnforcementMode::HostPrompts)).await.unwrap().compacted());
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

#[cfg(test)]
mod text_tests {
    use super::*;
    use discard_core::provider::TextRequest;

    #[tokio::test]
    async fn generated_text_records_requests_without_starting_sessions() {
        let request = TextRequest { model: "draft-model".into(), reasoning_effort: Some("high".into()), instructions: "style".into(), prompt: "description".into() };
        let provider = FakeProvider::new(vec![]).with_generated_text(Ok("generated".into()));
        assert_eq!(provider.generate_text(request.clone()).await.unwrap(), "generated");
        assert_eq!(*provider.probe.text_requests.lock().unwrap(), std::slice::from_ref(&request));
        assert!(provider.probe.specs.lock().unwrap().is_empty());
        let failed = FakeProvider::new(vec![]).with_generated_text(Err("failed".into()));
        assert!(matches!(failed.generate_text(request.clone()).await, Err(ProviderError::Protocol(message)) if message == "failed"));
        assert!(matches!(FakeProvider::new(vec![]).generate_text(request).await, Err(ProviderError::Setup(message)) if message == "This provider cannot generate text."));
    }
}
