//! Process management for the Claude CLI.

use crate::claude::parse::{Item, PendingRequest, StreamParser};
use async_trait::async_trait;
use discard_core::approval::Decision;
use discard_core::capability::{CapabilityMatrix, EnforcementMode};
use discard_core::event::{ApprovalId, EventKind, RawEvent};
use discard_core::provider::{Provider, ProviderError, ProviderSession, SessionMode, SessionSpec, SetupStatus};
use discard_core::redact::redact;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const STDERR_TAIL_LINES: usize = 20;
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const TERM_GRACE: Duration = Duration::from_secs(2);

const MATRIX_JSON: &str = include_str!("../../fixtures/claude.capabilities.json");

pub struct ClaudeProvider {
    executable: PathBuf,
    matrix: CapabilityMatrix,
    env: Vec<(OsString, OsString)>,
}

impl ClaudeProvider {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        let matrix = CapabilityMatrix::from_json(MATRIX_JSON).expect("bundled claude capability matrix is valid");
        Self::with_matrix(executable, matrix)
    }

    pub fn with_matrix(executable: impl Into<PathBuf>, matrix: CapabilityMatrix) -> Self {
        Self { executable: executable.into(), matrix, env: Vec::new() }
    }

    /// Extra environment for the child process (used by tests).
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// Command-line arguments for a Claude session. Never includes credentials.
pub fn build_args(spec: &SessionSpec) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "manual",
    ]
    .map(String::from)
    .into();
    match spec.enforcement {
        EnforcementMode::HostPrompts => {
            args.extend(["--permission-prompts", "host", "--permission-prompt-tool", "stdio"].map(String::from));
        }
        EnforcementMode::ProviderDenies => {
            args.extend(["--permission-prompts", "none"].map(String::from));
        }
    }
    if spec.mode == SessionMode::Conversation {
        // These tools enforce scoped memory and shared-write approval in Discard.
        // Permit only these exact names; native tools remain absent.
        args.extend([
            "--allowedTools".into(),
            crate::conversation::MEMORY_TOOLS.iter().map(|name| format!("mcp__discard-memory__{name}")).collect::<Vec<_>>().join(","),
        ]);
        args.extend(
            ["--tools", "", "--strict-mcp-config", "--disable-slash-commands", "--restricted", "--system-prompt-snapshot", "off"]
                .map(String::from),
        );
        args.extend([
            "--settings".into(),
            json!({
                "disableAllHooks":true, "autoMemoryEnabled":false,
                "env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"1", "CLAUDE_CODE_DISABLE_CLAUDE_MDS":"1", "CLAUDE_CODE_PLUGIN_DIRS":""},
                "pluginConfigs":{"cc-plugin-agents-md@builtin":{"options":{"instructionFiles":"managed-only"}}}
            })
            .to_string(),
        ]);
    }
    if let Some(effort) = &spec.reasoning_effort {
        args.extend(["--effort".into(), effort.clone()]);
    }
    if let Some(model) = &spec.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(prompt) = &spec.system_prompt {
        args.extend(["--append-system-prompt".into(), prompt.clone()]);
    }
    if let Some(handle) = &spec.resume_handle {
        args.extend(["--resume".into(), handle.clone()]);
    }
    if let Some(mcp) = &spec.mcp_config {
        args.extend(["--mcp-config".into(), mcp.to_string_lossy().into_owned()]);
    }
    args
}

impl ClaudeProvider {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.envs(self.env.iter().cloned());
        cmd
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> &str {
        "claude"
    }

    fn capabilities(&self) -> &CapabilityMatrix {
        &self.matrix
    }

    /// Checks that the CLI exists and reports a login. Reads only the `loggedIn` flag.
    /// Discard never reads, stores, or logs credentials.
    async fn models(&self) -> Result<Vec<discard_core::provider::ModelInfo>, ProviderError> {
        crate::models::claude(self.command()).await
    }

    async fn check_setup(&self) -> SetupStatus {
        let version =
            self.command().arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).status();
        match tokio::time::timeout(SETUP_TIMEOUT, version).await {
            Ok(Ok(_)) => {}
            _ => {
                return SetupStatus::NotInstalled {
                    install_hint: "Install the Claude Code CLI and make sure `claude` is on your PATH.".into(),
                };
            }
        }
        let status = self.command().args(["auth", "status"]).stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).output();
        let logged_in = match tokio::time::timeout(SETUP_TIMEOUT, status).await {
            Ok(Ok(out)) => {
                serde_json::from_slice::<Value>(&out.stdout).ok().and_then(|v| v.get("loggedIn").and_then(Value::as_bool)).unwrap_or(false)
            }
            _ => false,
        };
        if logged_in { SetupStatus::Ready } else { SetupStatus::NotLoggedIn { login_command: "claude auth login".into() } }
    }

    async fn start_session(&self, mut spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError> {
        // Fail closed before any process starts.
        self.matrix.supports(spec.enforcement)?;
        let files = if spec.mode == SessionMode::Conversation {
            crate::conversation::read_memory_config(&spec)?;
            if spec.model.as_deref().filter(|s| !s.is_empty()).is_none() {
                return Err(crate::conversation::setup("select an exact Claude model"));
            }
            if spec.resume_handle.is_some() {
                return Err(crate::conversation::setup("Claude conversation sessions require a fresh transcript replay"));
            }
            let version = tokio::time::timeout(SETUP_TIMEOUT, self.command().arg("--version").kill_on_drop(true).output())
                .await
                .map_err(|_| crate::conversation::setup("Claude version check timed out"))?
                .map_err(|e| crate::conversation::setup(e.to_string()))?;
            if !version.status.success() || !String::from_utf8_lossy(&version.stdout).starts_with("2.1.318 ") {
                return Err(crate::conversation::setup("Claude Code 2.1.318 is required for the verified conversation controls"));
            }
            let help = tokio::time::timeout(SETUP_TIMEOUT, self.command().arg("--help").kill_on_drop(true).output())
                .await
                .map_err(|_| crate::conversation::setup("Claude control check timed out"))?
                .map_err(|e| crate::conversation::setup(e.to_string()))?;
            let help_text = String::from_utf8_lossy(&help.stdout);
            if !help.status.success()
                || [
                    "--tools",
                    "--allowedTools",
                    "--strict-mcp-config",
                    "--restricted",
                    "--disable-slash-commands",
                    "--system-prompt-snapshot",
                    "--settings",
                ]
                .iter()
                .any(|flag| !help_text.contains(flag))
            {
                return Err(crate::conversation::setup("Claude is missing a required conversation control"));
            }
            // Managed policy remains authoritative. Reject known local conflicts; never change policy.
            crate::conversation::check_claude_managed_files()?;
            let owned = crate::conversation::ConversationFiles::new()?;
            spec.cwd = owned.cwd.clone();
            Some(owned)
        } else {
            None
        };
        let mut command = self.command();
        if spec.mode == SessionMode::Conversation {
            command
                .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
                .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
                .env_remove("CLAUDE_CODE_PLUGIN_DIRS");
        }
        let mut child = command
            .args(build_args(&spec))
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            // Own process group, so cancel can kill the CLI and everything it started.
            .process_group(0)
            .spawn()
            .map_err(|e| ProviderError::Spawn(format!("{}: {e}", self.executable.display())))?;
        let pgid = child.id().and_then(|p| i32::try_from(p).ok());
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| ProviderError::Spawn("no stdout pipe".into()))?;
        let stderr = child.stderr.take().ok_or_else(|| ProviderError::Spawn("no stderr pipe".into()))?;

        let shared = Arc::new(Shared::default());
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();

        let stderr_task = {
            let tail = Arc::clone(&tail);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = redact(&line);
                    debug!(target: "claude_stderr", "{line}");
                    let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            })
        };

        let (tx, events) = mpsc::unbounded_channel();
        {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                let mut parser = StreamParser::new();
                let mut lines = BufReader::new(stdout).lines();
                let mut saw_init = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    for item in parser.parse_line(&line) {
                        match item {
                            Item::SessionId(id) => {
                                saw_init = true;
                                *shared.session_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
                            }
                            Item::Request(req) => {
                                shared.requests.lock().unwrap_or_else(|p| p.into_inner()).insert(req.approval_id.clone(), req);
                            }
                            Item::Event(ev) => {
                                if tx.send(ev).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                if !saw_init {
                    // Let the stderr reader finish, then explain why the CLI never started a session.
                    let _ = tokio::time::timeout(Duration::from_millis(500), stderr_task).await;
                    let tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                    let detail = tail.iter().cloned().collect::<Vec<_>>().join("\n");
                    let message = format!("claude exited before starting a session: {detail}");
                    let _ = tx.send(RawEvent { kind: EventKind::Error { message }, raw: None });
                }
            });
        }

        Ok(Box::new(ClaudeSession { child, pgid, stdin, events, shared, _conversation_files: files }))
    }
}

#[derive(Default)]
struct Shared {
    session_id: Mutex<Option<String>>,
    requests: Mutex<HashMap<ApprovalId, PendingRequest>>,
}

struct ClaudeSession {
    _conversation_files: Option<crate::conversation::ConversationFiles>,
    child: Child,
    pgid: Option<i32>,
    stdin: Option<ChildStdin>,
    events: mpsc::UnboundedReceiver<RawEvent>,
    shared: Arc<Shared>,
}

impl ClaudeSession {
    fn kill_group(&self, signal: Signal) {
        if let Some(pgid) = self.pgid {
            // ESRCH just means the group is already gone.
            let _ = killpg(Pid::from_raw(pgid), signal);
        }
    }

    /// Write one JSON line. A closed pipe means the process already exited: the event stream
    /// reports that, so it is not an error here.
    async fn write_line(&mut self, value: &Value) -> Result<(), ProviderError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(());
        };
        let mut line = serde_json::to_string(value).map_err(|e| ProviderError::Protocol(e.to_string()))?;
        line.push('\n');
        match stdin.write_all(line.as_bytes()).await {
            Ok(()) => stdin.flush().await.or_else(ignore_closed_pipe),
            Err(e) => ignore_closed_pipe(e),
        }
    }
}

fn ignore_closed_pipe(e: std::io::Error) -> Result<(), ProviderError> {
    if e.kind() == ErrorKind::BrokenPipe {
        warn!("claude process closed its input; waiting for the event stream to report why");
        Ok(())
    } else {
        Err(ProviderError::Io(e.to_string()))
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        self.kill_group(Signal::SIGKILL);
    }
}

#[async_trait]
impl ProviderSession for ClaudeSession {
    fn resume_handle(&self) -> Option<String> {
        self.shared.session_id.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError> {
        self.write_line(&json!({"type": "user", "message": {"role": "user", "content": text}})).await
    }
    async fn send_user_message_with_images(&mut self, text: &str, images: &[discard_core::attachments::Attachment]) -> Result<(), ProviderError> {
        if images.is_empty() { return self.send_user_message(text).await; }
        let content = crate::attachments::claude_content(text, images)?;
        self.write_line(&json!({"type": "user", "message": {"role": "user", "content": content}})).await
    }

    /// Cancel-safe: this only awaits a channel receive.
    async fn next_event(&mut self) -> Option<RawEvent> {
        self.events.recv().await
    }

    async fn respond_approval(&mut self, id: &ApprovalId, decision: Decision) -> Result<(), ProviderError> {
        let request = self
            .shared
            .requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
            .ok_or_else(|| ProviderError::Protocol(format!("no pending permission request for approval {id}")))?;
        let response = match decision {
            Decision::Approve => json!({"behavior": "allow", "updatedInput": request.input}),
            Decision::AlwaysAllow => json!({
                "behavior": "allow",
                "updatedInput": request.input,
                "updatedPermissions": session_scoped_rules(&request.suggestions),
            }),
            Decision::Deny => {
                json!({"behavior": "deny", "message": "Denied by the user in Discard."})
            }
        };
        self.write_line(&json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request.request_id, "response": response},
        }))
        .await
    }

    async fn cancel(&mut self) -> Result<(), ProviderError> {
        self.stdin = None;
        self.kill_group(Signal::SIGTERM);
        if tokio::time::timeout(TERM_GRACE, self.child.wait()).await.is_err() {
            self.kill_group(Signal::SIGKILL);
            let _ = self.child.wait().await;
        }
        // Sweep anything in the group that ignored SIGTERM.
        self.kill_group(Signal::SIGKILL);
        Ok(())
    }
}

/// "Always allow" lasts for the session only: keep the rule suggestions, drop the destination.
fn session_scoped_rules(suggestions: &[Value]) -> Vec<Value> {
    suggestions
        .iter()
        .filter(|s| s.get("type").and_then(Value::as_str) == Some("addRules"))
        .map(|s| {
            let mut s = s.clone();
            s["destination"] = json!("session");
            s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn has_pair(args: &[String], a: &str, b: &str) -> bool {
        args.windows(2).any(|w| w[0] == a && w[1] == b)
    }

    #[test]
    fn streams_json_both_ways() {
        let args = build_args(&spec(EnforcementMode::HostPrompts));
        assert!(args.contains(&"-p".to_string()));
        assert!(has_pair(&args, "--input-format", "stream-json"));
        assert!(has_pair(&args, "--output-format", "stream-json"));
        assert!(args.contains(&"--include-partial-messages".to_string()));
    }

    #[test]
    fn host_prompts_route_permission_requests_to_the_host() {
        let args = build_args(&spec(EnforcementMode::HostPrompts));
        assert!(has_pair(&args, "--permission-prompts", "host"));
        assert!(has_pair(&args, "--permission-prompt-tool", "stdio"));
    }

    #[test]
    fn provider_denies_mode_never_asks_the_host() {
        let args = build_args(&spec(EnforcementMode::ProviderDenies));
        assert!(has_pair(&args, "--permission-prompts", "none"));
        assert!(!args.contains(&"--permission-prompt-tool".to_string()));
    }

    #[test]
    fn optional_settings_are_passed_through() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.model = Some("haiku".into());
        s.reasoning_effort = Some("high".into());
        s.system_prompt = Some("Be brief.".into());
        s.resume_handle = Some("sess-1".into());
        s.mcp_config = Some(PathBuf::from("/run/discard/mcp.json"));
        let args = build_args(&s);
        assert!(has_pair(&args, "--model", "haiku"));
        assert!(has_pair(&args, "--effort", "high"));
        assert!(has_pair(&args, "--append-system-prompt", "Be brief."));
        assert!(has_pair(&args, "--resume", "sess-1"));
        assert!(has_pair(&args, "--mcp-config", "/run/discard/mcp.json"));
    }

    #[test]
    fn omitted_settings_add_no_flags() {
        let args = build_args(&spec(EnforcementMode::HostPrompts));
        for flag in ["--effort", "--model", "--append-system-prompt", "--resume", "--mcp-config"] {
            assert!(!args.contains(&flag.to_string()), "{flag}");
        }
    }

    #[test]
    fn conversation_memory_permissions_allow_only_the_exact_scoped_tools() {
        for enforcement in [EnforcementMode::HostPrompts, EnforcementMode::ProviderDenies] {
            let mut s = spec(enforcement);
            assert!(!build_args(&s).iter().any(|arg| arg == "--allowedTools"));
            s.mode = SessionMode::Conversation;
            let args = build_args(&s);
            let allowed = args.iter().position(|arg| arg == "--allowedTools").expect("scoped memory permission list");
            assert_eq!(
                args[allowed + 1],
                "mcp__discard-memory__memory_search,mcp__discard-memory__memory_list,mcp__discard-memory__memory_read,mcp__discard-memory__memory_save,mcp__discard-memory__memory_edit,mcp__discard-memory__memory_forget,mcp__discard-memory__worktree_inspect,mcp__discard-memory__worktree_request_removal"
            );
            assert!(args.windows(2).any(|pair| pair == ["--tools", ""]));
        }
    }

    #[test]
    fn conversation_mode_removes_native_tools_and_provider_context() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = discard_core::provider::SessionMode::Conversation;
        let args = build_args(&s);
        for flag in ["--strict-mcp-config", "--restricted", "--disable-slash-commands"] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        assert!(args.windows(2).any(|w| w == ["--tools", ""]));
        assert!(args.windows(2).any(|w| w == ["--system-prompt-snapshot", "off"]));
    }

    #[test]
    fn bundled_capability_matrix_describes_claude() {
        let p = ClaudeProvider::new("claude");
        let c = &p.capabilities().capabilities;
        assert_eq!(p.capabilities().provider, "claude");
        assert!(c.interactive_approvals && c.session_resume && c.cancellation);
    }
}
