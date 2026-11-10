//! The `Provider` trait. Providers wrap a CLI and emit the normalized event stream.

use crate::approval::Decision;
use crate::capability::{CapabilityMatrix, EnforcementMode};
use crate::event::{ApprovalId, RawEvent};
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider setup problem: {0}")]
    Setup(String),
    #[error("provider sandbox unavailable: {0}")]
    SandboxUnavailable(String),
    #[error("failed to start provider process: {0}")]
    Spawn(String),
    #[error("provider I/O error: {0}")]
    Io(String),
    #[error("provider protocol error: {0}")]
    Protocol(String),
    #[error("resume handle rejected: {0}")]
    ResumeRejected(String),
    #[error(transparent)]
    Policy(#[from] crate::capability::PolicyError),
}

/// Result of checking that the provider CLI is installed and logged in.
/// Discard never handles credentials: it only shows the command the user must run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupStatus {
    Ready,
    NotInstalled { install_hint: String },
    NotLoggedIn { login_command: String },
}

/// The conversation profile must remove coding tools and provider-owned memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionMode {
    #[default]
    Coding,
    Conversation,
    Workspace,
}

impl SessionMode {
    pub fn replays_transcript(self) -> bool { matches!(self, Self::Conversation | Self::Workspace) }
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub mode: SessionMode,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub system_prompt: Option<String>,
    pub enforcement: EnforcementMode,
    /// Provider-native session ID to resume. `None` starts a new session.
    pub resume_handle: Option<String>,
    /// Per-session MCP config file (mode 0600, outside the project). Used from milestone M3.
    pub mcp_config: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub reasoning_efforts: Vec<ReasoningOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReasoningOption {
    pub value: String,
    pub description: String,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> &CapabilityMatrix;
    /// The provider can protect main-checkout Git metadata during write responses.
    fn supports_main_checkout_writes(&self) -> bool { false }
    async fn check_setup(&self) -> SetupStatus;
    /// Available models reported by this provider, without starting a conversation.
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Err(ProviderError::Setup("This provider does not expose a model list.".into()))
    }
    /// Catalog context limit for the exact model, when the provider publishes one.
    fn model_context_window(&self, _model: &str) -> Option<u64> {
        None
    }
    /// Start a provider session. Fails closed if the provider cannot enforce `spec.enforcement`.
    async fn start_session(&self, spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError>;
}

#[async_trait]
pub trait ProviderSession: Send {
    /// The provider-native session ID, once known. Stored as a resume handle only.
    fn resume_handle(&self) -> Option<String>;
    /// Send one user message and begin a turn.
    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError>;
    /// Supply image content without enabling file-reading tools in conversation mode.
    async fn send_user_message_with_images(&mut self, text: &str, images: &[crate::attachments::Attachment]) -> Result<(), ProviderError> {
        if !images.is_empty() { return Err(ProviderError::Setup("This provider does not support image attachments.".into())); }
        self.send_user_message(text).await
    }
    /// Next normalized event. `None` means the provider process exited or the stream closed.
    async fn next_event(&mut self) -> Option<RawEvent>;
    /// Answer a pending approval request.
    async fn respond_approval(&mut self, id: &ApprovalId, decision: Decision) -> Result<(), ProviderError>;
    /// Stop the current turn. Kills the whole process group of the provider process.
    async fn cancel(&mut self) -> Result<(), ProviderError>;
}
