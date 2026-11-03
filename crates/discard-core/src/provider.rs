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

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub enforcement: EnforcementMode,
    /// Provider-native session ID to resume. `None` starts a new session.
    pub resume_handle: Option<String>,
    /// Per-session MCP config file (mode 0600, outside the project). Used from milestone M3.
    pub mcp_config: Option<PathBuf>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> &CapabilityMatrix;
    async fn check_setup(&self) -> SetupStatus;
    /// Start a provider session. Fails closed if the provider cannot enforce `spec.enforcement`.
    async fn start_session(&self, spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError>;
}

#[async_trait]
pub trait ProviderSession: Send {
    /// The provider-native session ID, once known. Stored as a resume handle only.
    fn resume_handle(&self) -> Option<String>;
    /// Send one user message and begin a turn.
    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError>;
    /// Next normalized event. `None` means the provider process exited or the stream closed.
    async fn next_event(&mut self) -> Option<RawEvent>;
    /// Answer a pending approval request.
    async fn respond_approval(&mut self, id: &ApprovalId, decision: Decision) -> Result<(), ProviderError>;
    /// Stop the current turn. Kills the whole process group of the provider process.
    async fn cancel(&mut self) -> Result<(), ProviderError>;
}
