//! Process management for the Claude CLI.

use discard_core::process::spawn_tokio;
use crate::version::{self, CLAUDE_TESTED};
use crate::claude::parse::{Item, PendingRequest, StreamParser};
use async_trait::async_trait;
use discard_core::approval::Decision;
use discard_core::capability::{CapabilityMatrix, EnforcementMode};
use discard_core::event::{ApprovalId, EventKind, RawEvent};
use discard_core::provider::{Provider, ProviderError, ProviderSession, SandboxAutorun, SessionMode, SessionSpec, SetupStatus};
use discard_core::sandbox_reads::ReadDenials;
use discard_core::redact::redact;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

const STDERR_TAIL_LINES: usize = 20;
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const TERM_GRACE: Duration = Duration::from_secs(2);

/// How long the first message of a resumed session waits for the CLI to start or reject the session.
const RESUME_START_TIMEOUT: Duration = Duration::from_secs(60);
/// The CLI's message for a session ID it cannot find.
const UNKNOWN_SESSION: &str = "No conversation found with session ID";

const MATRIX_JSON: &str = include_str!("../../fixtures/claude.capabilities.json");

/// Git settings for autorun sessions. They stop Git from asking for credentials or reading
/// a credential helper, such as the macOS Keychain helper.
const AUTORUN_GIT_ENV: &[(&str, &str)] =
    &[("GIT_TERMINAL_PROMPT", "0"), ("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "credential.helper"), ("GIT_CONFIG_VALUE_0", "")];

/// Token variables that autorun sessions must not inherit. They stop `gh` and Git from
/// finding a GitHub token in the environment.
const AUTORUN_TOKEN_ENV: &[&str] = &["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"];

/// Environment variables to remove from the Claude child process for a session.
fn env_removals(autorun: bool) -> &'static [&'static str] {
    if autorun { AUTORUN_TOKEN_ENV } else { &[] }
}

pub struct ClaudeProvider {
    executable: PathBuf,
    matrix: CapabilityMatrix,
    env: Vec<(OsString, OsString)>,
    /// `DISCARD_HOME`. Autorun sessions deny reads of its files except the session's own.
    discard_home: Option<PathBuf>,
    /// The CLI version last read. Part of the session fingerprint.
    epoch: Mutex<Option<String>>,
}

impl ClaudeProvider {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        let matrix = CapabilityMatrix::from_json(MATRIX_JSON).expect("bundled claude capability matrix is valid");
        Self::with_matrix(executable, matrix)
    }

    pub fn with_matrix(executable: impl Into<PathBuf>, matrix: CapabilityMatrix) -> Self {
        Self { executable: executable.into(), matrix, env: Vec::new(), discard_home: None, epoch: Mutex::new(None) }
    }

    /// Extra environment for the child process (used by tests).
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The Discard data directory. Autorun needs it to protect files outside the session root.
    pub fn with_discard_home(mut self, discard_home: impl Into<PathBuf>) -> Self {
        self.discard_home = Some(discard_home.into());
        self
    }
}

/// Command-line arguments for a Claude session. Never includes credentials.
/// A Workspace session with `sandbox_autorun` needs `read_denials`, or it fails closed.
pub fn build_args(spec: &SessionSpec, protected_git_paths: &[PathBuf], read_denials: Option<&ReadDenials>) -> Result<Vec<String>, ProviderError> {
    if spec.mode == SessionMode::Workspace && protected_git_paths.is_empty() {
        return Err(ProviderError::SandboxUnavailable("workspace Git write protection is unavailable".into()));
    }
    let autorun = spec.mode == SessionMode::Workspace && spec.sandbox_autorun;
    if autorun && read_denials.is_none_or(|d| d.paths.is_empty()) {
        return Err(ProviderError::SandboxUnavailable("autorun read protection is unavailable".into()));
    }
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
    if spec.mode.replays_transcript() {
        // These tools enforce scoped memory and shared-write approval in Discard.
        // Permit only these exact names; native tools remain absent.
        args.extend([
            "--allowedTools".into(),
            crate::conversation::conversation_tools(spec).iter().map(|name| format!("mcp__discard-memory__{name}")).collect::<Vec<_>>().join(","),
        ]);
        args.extend(
            ["--tools", if spec.mode == SessionMode::Workspace { "Read,Glob,Grep,Edit,Write,Bash" } else { "" }, "--strict-mcp-config", "--disable-slash-commands", "--restricted", "--system-prompt-snapshot", "off"]
                .map(String::from),
        );
        if spec.mode == SessionMode::Workspace {
            // The agent can write `<root>/.claude/settings*.json`. Load no settings files, so
            // only the `--settings` value below configures the sandbox.
            args.extend(["--setting-sources".into(), String::new()]);
        }
        let mut settings = json!({
            "disableAllHooks":true, "autoMemoryEnabled":false,
            "env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"1", "CLAUDE_CODE_DISABLE_CLAUDE_MDS":"1", "CLAUDE_CODE_PLUGIN_DIRS":""},
            "pluginConfigs":{"cc-plugin-agents-md@builtin":{"options":{"instructionFiles":"managed-only"}}}
        });
        if spec.mode == SessionMode::Workspace {
            settings["env"]["GIT_OPTIONAL_LOCKS"] = json!("0");
            settings["sandbox"] = json!({"enabled":true,"allowUnsandboxedCommands":false,"autoAllowBashIfSandboxed":false});
            settings["sandbox"]["filesystem"] = json!({"denyWrite":protected_git_paths});
            if let (true, Some(denials)) = (autorun, read_denials) {
                settings["sandbox"]["autoAllowBashIfSandboxed"] = json!(true);
                for (key, value) in AUTORUN_GIT_ENV {
                    settings["env"][*key] = json!(value);
                }
                settings["sandbox"]["filesystem"]["denyRead"] = json!(denials.paths);
                // The sandbox covers Bash only. These rules block native Read, Glob, and Grep.
                // `Bash(security:*)` is a partial Keychain mitigation: the sandbox always
                // allows the Keychain service, so a script can still reach it directly.
                let mut deny: Vec<String> = denials.rules.iter().map(|r| r.to_string()).collect();
                deny.push("Bash(security:*)".into());
                settings["permissions"] = json!({"deny":deny});
            }
        }
        args.extend(["--settings".into(), settings.to_string()]);
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
    Ok(args)
}

/// A directory from an environment variable. The CLI ignores a relative value, so Discard does too.
fn absolute_dir(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

impl ClaudeProvider {
    fn record_epoch(&self, version_output: &str) {
        if let Some(version) = version::number(version_output) {
            *self.epoch.lock().unwrap_or_else(|p| p.into_inner()) = Some(version);
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.envs(self.env.iter().cloned());
        cmd
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn supports_main_checkout_writes(&self) -> bool { true }
    fn sandbox_autorun(&self) -> SandboxAutorun { SandboxAutorun::Supported }
    fn id(&self) -> &str {
        "claude"
    }

    fn capabilities(&self) -> &CapabilityMatrix {
        &self.matrix
    }

    fn resume_epoch(&self) -> Option<String> {
        self.epoch.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Every model Claude Code offers has a window of at least 200,000 tokens.
    fn model_context_window(&self, _model: &str) -> Option<u64> { Some(200_000) }

    /// Checks that the CLI exists and reports a login. Reads only the `loggedIn` flag.
    /// Discard never reads, stores, or logs credentials.
    async fn models(&self) -> Result<Vec<discard_core::provider::ModelInfo>, ProviderError> {
        crate::models::claude(self.command()).await
    }

    async fn generate_text(&self, request: discard_core::provider::TextRequest) -> Result<String, ProviderError> {
        super::text::generate(self.command(), request).await
    }

    async fn check_setup(&self) -> SetupStatus {
        let version = spawn_tokio(self.command().arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true));
        match tokio::time::timeout(SETUP_TIMEOUT, async { version?.wait_with_output().await }).await {
            Ok(Ok(out)) => {
                if out.status.success() { self.record_epoch(&String::from_utf8_lossy(&out.stdout)); }
            }
            _ => {
                return SetupStatus::NotInstalled {
                    install_hint: "Install the Claude Code CLI and make sure `claude` is on your PATH.".into(),
                };
            }
        }
        let status = spawn_tokio(self.command().args(["auth", "status"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true));
        let logged_in = match tokio::time::timeout(SETUP_TIMEOUT, async { status?.wait_with_output().await }).await {
            Ok(Ok(out)) => {
                serde_json::from_slice::<Value>(&out.stdout).ok().and_then(|v| v.get("loggedIn").and_then(Value::as_bool)).unwrap_or(false)
            }
            _ => false,
        };
        if logged_in { SetupStatus::Ready } else { SetupStatus::NotLoggedIn { login_command: "claude auth login".into() } }
    }

    async fn start_session(&self, mut spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError> {
        super::api::check_api_address(&self.command()).await?;
        let mut notice = None;
        let workspace = spec.mode == SessionMode::Workspace;
        let autorun = workspace && spec.sandbox_autorun;
        let read_denials = if autorun {
            let discard_home = self.discard_home.as_deref().ok_or_else(|| ProviderError::SandboxUnavailable("autorun needs the Discard data directory".into()))?;
            let home = std::env::home_dir().ok_or_else(|| ProviderError::SandboxUnavailable("autorun needs the home directory".into()))?;
            // Relocated credential directories are denied as well as the defaults.
            let claude_config = absolute_dir(std::env::var_os("CLAUDE_CONFIG_DIR")).unwrap_or_else(|| home.join(".claude"));
            let relocated: Vec<PathBuf> = absolute_dir(std::env::var_os("CODEX_HOME")).into_iter().collect();
            Some(ReadDenials::new(&home, discard_home, &spec.cwd, &claude_config, &relocated))
        } else { None };
        let protected_git_paths = if workspace {
            if !cfg!(target_os = "macos") || spec.enforcement != EnforcementMode::HostPrompts {
                return Err(ProviderError::SandboxUnavailable("Claude workspace sessions require macOS and host approvals".into()));
            }
            if !spec.cwd.is_absolute() || !spec.cwd.is_dir() { return Err(ProviderError::SandboxUnavailable("the workspace must be an existing absolute directory".into())); }
            let paths = discard_core::workspace_session::protected_git_paths(&spec.cwd).map_err(ProviderError::SandboxUnavailable)?;
            if !std::path::Path::new("/usr/bin/sandbox-exec").is_file() { return Err(ProviderError::SandboxUnavailable("macOS sandbox-exec is unavailable".into())); }
            paths
        } else { Vec::new() };
        let setup = |message: String| crate::conversation::setup(message);
        // Fail closed before any process starts.
        self.matrix.supports(spec.enforcement)?;
        let files = if spec.mode.replays_transcript() {
            crate::conversation::read_memory_config(&spec)?;
            if spec.model.as_deref().filter(|s| !s.is_empty()).is_none() {
                return Err(setup("select an exact Claude model".into()));
            }
            if spec.resume_handle.is_some() && !spec.resumable {
                return Err(setup("Claude conversation sessions require a fresh transcript replay".into()));
            }
            if spec.resumable && (!spec.cwd.is_absolute() || !spec.cwd.is_dir()) {
                return Err(setup("a resumable session needs an existing absolute working directory".into()));
            }
            let version = spawn_tokio(self.command().arg("--version").kill_on_drop(true).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()));
            let version = tokio::time::timeout(SETUP_TIMEOUT, async { version?.wait_with_output().await })
                .await
                .map_err(|_| setup("Claude Code version could not be read: version check timed out".into()))?
                .map_err(|_| setup("Claude Code version could not be read".into()))?;
            let output = String::from_utf8_lossy(&version.stdout);
            if version.status.success() { self.record_epoch(&output); }
            let running = version::require(if version.status.success() { &output } else { "" }, "Claude Code", CLAUDE_TESTED)?;
            let help = spawn_tokio(self.command().arg("--help").kill_on_drop(true).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()));
            let help = tokio::time::timeout(SETUP_TIMEOUT, async { help?.wait_with_output().await })
                .await
                .map_err(|_| setup(format!("Claude Code {running} control check timed out (tested version {CLAUDE_TESTED})")))?
                .map_err(|_| setup(format!("Claude Code {running} could not verify conversation controls (tested version {CLAUDE_TESTED})")))?;
            let help_text = String::from_utf8_lossy(&help.stdout);
            if !help.status.success() {
                return Err(setup(format!("Claude Code {running} could not verify conversation controls (tested version {CLAUDE_TESTED})")));
            }
            let args = build_args(&spec, &protected_git_paths, read_denials.as_ref())?;
            let minimum = ["--tools", "--allowedTools", "--strict-mcp-config", "--restricted", "--disable-slash-commands", "--system-prompt-snapshot", "--settings"];
            for flag in minimum.into_iter().chain(args.iter().map(String::as_str).filter(|arg| arg.starts_with("--"))) {
                if !help_text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').any(|token| token == flag) {
                    return Err(setup(format!("Claude Code {running} is missing required conversation control {flag} (tested version {CLAUDE_TESTED})")));
                }
            }
            // Managed policy remains authoritative. Reject known local conflicts; never change policy.
            crate::conversation::check_claude_managed_files(workspace).map_err(|e| version::verification_error(e, "Claude Code", &running, CLAUDE_TESTED))?;
            notice = version::notice("Claude Code", &running, CLAUDE_TESTED);
            if workspace || spec.resumable { None } else {
                let owned = crate::conversation::ConversationFiles::new()?;
                spec.cwd = owned.cwd.clone(); Some(owned)
            }
        } else {
            None
        };
        let mut command = self.command();
        if spec.mode.replays_transcript() {
            command
                .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
                .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
                .env_remove("CLAUDE_CODE_PLUGIN_DIRS");
        }
        // Discard owns the size guard of a resumable session, so the CLI must not compact it alone.
        if spec.resumable && spec.mode.replays_transcript() { command.env("DISABLE_AUTO_COMPACT", "1"); }
        if workspace { command.env("GIT_OPTIONAL_LOCKS", "0"); }
        if autorun { command.envs(AUTORUN_GIT_ENV.iter().copied()); }
        for key in env_removals(autorun) { command.env_remove(key); }
        let mut child = spawn_tokio(command
            .args(build_args(&spec, &protected_git_paths, read_denials.as_ref())?)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            // Own process group, so cancel can kill the CLI and everything it started.
            .process_group(0)
        )
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
        // A resumed session learns from its first reply whether the CLI accepted the handle.
        let resuming = spec.resumable && spec.resume_handle.is_some();
        let (start_tx, start_rx) = if resuming {
            let (t, r) = oneshot::channel();
            (Some(t), Some(r))
        } else { (None, None) };
        {
            let shared = Arc::clone(&shared);
            let mut start_tx = start_tx;
            tokio::spawn(async move {
                let mut rejection: Option<String> = None;
                let mut parser = StreamParser::new();
                let mut lines = BufReader::new(stdout).lines();
                let mut saw_init = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    if resuming && !saw_init && let Some(message) = unknown_session_result(&line) {
                        rejection = Some(message);
                        continue;
                    }
                    for item in parser.parse_line(&line) {
                        match item {
                            Item::Compacted => shared.compacted.store(true, Ordering::SeqCst),
                            Item::SessionId(id) => {
                                saw_init = true;
                                if let Some(t) = start_tx.take() { let _ = t.send(Start::Ready); }
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
                    if resuming && rejection.is_none() && detail.contains(UNKNOWN_SESSION) { rejection = Some(redact(&detail)); }
                    if let Some(message) = rejection {
                        if let Some(t) = start_tx.take() { let _ = t.send(Start::Rejected(message)); }
                        return;
                    }
                    if let Some(t) = start_tx.take() { let _ = t.send(Start::Exited); }
                    let message = format!("claude exited before starting a session: {detail}");
                    let _ = tx.send(RawEvent { kind: EventKind::Error { message }, raw: None });
                }
            });
        }

        Ok(Box::new(ClaudeSession { notice, child, pgid, stdin, events, shared, started: start_rx, _conversation_files: files }))
    }
}

#[derive(Default)]
struct Shared {
    session_id: Mutex<Option<String>>,
    requests: Mutex<HashMap<ApprovalId, PendingRequest>>,
    compacted: AtomicBool,
}

/// What the first reply of a resumed session shows.
enum Start {
    Ready,
    Rejected(String),
    Exited,
}

/// The redacted message of a `result` event that reports an unknown session ID.
fn unknown_session_result(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("result") || v.get("subtype").and_then(Value::as_str) != Some("error_during_execution") {
        return None;
    }
    v.get("errors")?.as_array()?.iter().filter_map(Value::as_str).find(|e| e.starts_with(UNKNOWN_SESSION)).map(redact)
}

struct ClaudeSession {
    notice: Option<String>,
    _conversation_files: Option<crate::conversation::ConversationFiles>,
    child: Child,
    pgid: Option<i32>,
    stdin: Option<ChildStdin>,
    events: mpsc::UnboundedReceiver<RawEvent>,
    shared: Arc<Shared>,
    /// Set for a resumed session until its first message shows whether the CLI accepted the handle.
    started: Option<oneshot::Receiver<Start>>,
}

impl ClaudeSession {
    fn kill_group(&self, signal: Signal) {
        if let Some(pgid) = self.pgid {
            // ESRCH just means the group is already gone.
            let _ = killpg(Pid::from_raw(pgid), signal);
        }
    }

    /// After the first message of a resumed session, wait for the CLI to start or reject it.
    async fn await_start(&mut self) -> Result<(), ProviderError> {
        let Some(rx) = self.started.take() else { return Ok(()) };
        match tokio::time::timeout(RESUME_START_TIMEOUT, rx).await {
            Ok(Ok(Start::Rejected(message))) => Err(ProviderError::ResumeRejected(message)),
            Ok(Ok(Start::Ready | Start::Exited)) | Ok(Err(_)) | Err(_) => Ok(()),
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
    fn notice(&self) -> Option<String> { self.notice.clone() }
    fn resume_handle(&self) -> Option<String> {
        self.shared.session_id.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn compacted(&self) -> bool { self.shared.compacted.load(Ordering::SeqCst) }

    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError> {
        self.write_line(&json!({"type": "user", "message": {"role": "user", "content": text}})).await?;
        self.await_start().await
    }
    async fn send_user_message_with_images(&mut self, text: &str, images: &[discard_core::attachments::Attachment]) -> Result<(), ProviderError> {
        if images.is_empty() { return self.send_user_message(text).await; }
        let content = crate::attachments::claude_content(text, images)?;
        self.write_line(&json!({"type": "user", "message": {"role": "user", "content": content}})).await?;
        self.await_start().await
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
            resumable: false,
            reasoning_effort: None,
            mode: discard_core::provider::SessionMode::Coding,
            cwd: PathBuf::from("."),
            model: None,
            system_prompt: None,
            enforcement,
            resume_handle: None,
            mcp_config: None,
            sandbox_autorun: false,
        }
    }

    #[test]
    fn workspace_args_enable_only_the_write_tools_and_the_sandbox() {
        let mut s = spec(EnforcementMode::HostPrompts); s.mode = SessionMode::Workspace;
        let paths = vec![PathBuf::from("/worktree/.git"), PathBuf::from("/repo/.git")];
        let args = build_args(&s, &paths, None).unwrap();
        assert!(has_pair(&args, "--tools", "Read,Glob,Grep,Edit,Write,Bash"));
        let value = |key| &args[args.iter().position(|a| a == key).unwrap() + 1];
        assert!(value("--allowedTools").contains("mcp__discard-memory__worktree_commit"));
        let settings: Value = serde_json::from_str(value("--settings")).unwrap();
        assert_eq!(settings["sandbox"]["filesystem"]["denyWrite"], json!(paths));
        assert_eq!(settings["sandbox"]["enabled"], true);
        assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
        assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
        assert_eq!(settings["disableAllHooks"], true);
        assert_eq!(settings["env"]["GIT_OPTIONAL_LOCKS"], "0");
        assert_eq!(settings["env"]["CLAUDE_CODE_DISABLE_CLAUDE_MDS"], "1");
        assert!(has_pair(&args, "--permission-mode", "manual"));
        assert!(args.contains(&"--restricted".into()));
        assert!(!args.iter().any(|a| a == "acceptEdits" || a == "--resume"));
    }
    fn autorun_settings(autorun: bool) -> Value {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let discard_home = tmp.path().join("discard");
        let root = discard_home.join("worktrees").join("w1");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(discard_home.join("other")).unwrap();
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = SessionMode::Workspace;
        s.cwd = root.clone();
        s.sandbox_autorun = autorun;
        let denials = ReadDenials::new(&home, &discard_home, &root, &home.join(".claude"), &[]);
        let args = build_args(&s, &[root.join(".git")], Some(&denials)).unwrap();
        serde_json::from_str(&args[args.iter().position(|a| a == "--settings").unwrap() + 1]).unwrap()
    }

    #[test]
    fn autorun_session_auto_allows_sandboxed_bash() {
        let settings = autorun_settings(true);
        assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], true);
        assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
        assert_eq!(settings["sandbox"]["enabled"], true);
        assert!(settings["sandbox"]["filesystem"]["denyWrite"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().ends_with(".git")));
        assert!(!settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap().is_empty());
    }

    #[test]
    fn autorun_denies_native_reads_of_credentials() {
        let settings = autorun_settings(true);
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        let reads: Vec<&str> = deny.iter().map(|r| r.as_str().unwrap()).filter(|r| r.starts_with("Read(")).collect();
        assert!(reads.iter().all(|r| r.starts_with("Read(//")));
        assert!(reads.iter().any(|r| r.contains(".ssh")));
        assert_eq!(reads.len(), settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap().len());
    }

    #[test]
    fn autorun_denies_the_security_command() {
        let settings = autorun_settings(true);
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(deny.iter().any(|r| r == "Bash(security:*)"), "{deny:?}");
        assert!(autorun_settings(false).get("permissions").is_none());
    }

    #[test]
    fn autorun_removes_github_token_variables_from_the_child() {
        let expected = ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"];
        assert_eq!(env_removals(true), expected);
        assert!(env_removals(false).is_empty());
        let settings = autorun_settings(true);
        for key in expected {
            assert!(settings["env"].get(key).is_none(), "{key} must not be set through --settings");
        }
    }

    #[test]
    fn workspace_sessions_load_no_settings_files() {
        for autorun in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let mut s = spec(EnforcementMode::HostPrompts);
            s.mode = SessionMode::Workspace;
            s.cwd = tmp.path().to_path_buf();
            s.sandbox_autorun = autorun;
            let denials = ReadDenials::new(tmp.path(), tmp.path(), tmp.path(), &tmp.path().join(".claude"), &[]);
            let args = build_args(&s, &[tmp.path().join(".git")], Some(&denials)).unwrap();
            assert!(has_pair(&args, "--setting-sources", ""), "autorun {autorun}: {args:?}");
            assert!(args.contains(&"--settings".into()));
        }
    }

    #[test]
    fn relative_config_directories_are_ignored() {
        assert_eq!(absolute_dir(Some("/abs/cfg".into())), Some(PathBuf::from("/abs/cfg")));
        assert_eq!(absolute_dir(Some("relative/cfg".into())), None);
        assert_eq!(absolute_dir(Some("".into())), None);
        assert_eq!(absolute_dir(None), None);
    }

    #[test]
    fn autorun_disables_git_credential_helpers() {
        let settings = autorun_settings(true);
        let env = &settings["env"];
        assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
        assert_eq!(env["GIT_CONFIG_COUNT"], "1");
        assert_eq!(env["GIT_CONFIG_KEY_0"], "credential.helper");
        assert_eq!(env["GIT_CONFIG_VALUE_0"], "");
        assert_eq!(env["GIT_OPTIONAL_LOCKS"], "0");
        assert!(autorun_settings(false)["env"].get("GIT_CONFIG_COUNT").is_none());
        let vars: Vec<(&str, &str)> = AUTORUN_GIT_ENV.to_vec();
        assert!(vars.contains(&("GIT_CONFIG_VALUE_0", "")) && vars.contains(&("GIT_TERMINAL_PROMPT", "0")));
    }

    #[test]
    fn workspace_without_autorun_keeps_bash_prompts() {
        let settings = autorun_settings(false);
        assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
        assert!(settings["sandbox"]["filesystem"].get("denyRead").is_none());
        assert!(settings.get("permissions").is_none());
    }

    #[test]
    fn autorun_without_read_denials_fails_closed() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = SessionMode::Workspace;
        s.sandbox_autorun = true;
        assert!(matches!(build_args(&s, &[PathBuf::from("/w/.git")], None), Err(ProviderError::SandboxUnavailable(_))));
    }

    #[tokio::test]
    async fn autorun_without_the_discard_home_fails_closed() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = SessionMode::Workspace;
        s.sandbox_autorun = true;
        assert!(matches!(ClaudeProvider::new("unused").with_env("ANTHROPIC_BASE_URL", "").start_session(s).await, Err(ProviderError::SandboxUnavailable(_))));
    }

    #[test]
    fn claude_supports_sandbox_autorun() {
        assert_eq!(ClaudeProvider::new("unused").sandbox_autorun(), SandboxAutorun::Supported);
    }

    #[test]
    fn workspace_args_refuse_missing_git_protection() {
        let mut s = spec(EnforcementMode::HostPrompts); s.mode = SessionMode::Workspace;
        assert!(matches!(build_args(&s, &[], None), Err(ProviderError::SandboxUnavailable(_))));
    }
    #[tokio::test]
    async fn workspace_mode_requires_host_prompts() {
        let mut s = spec(EnforcementMode::ProviderDenies); s.mode = SessionMode::Workspace;
        assert!(matches!(ClaudeProvider::new("unused").with_env("ANTHROPIC_BASE_URL", "").start_session(s).await, Err(ProviderError::SandboxUnavailable(_))));
    }

    fn has_pair(args: &[String], a: &str, b: &str) -> bool {
        args.windows(2).any(|w| w[0] == a && w[1] == b)
    }

    #[test]
    fn streams_json_both_ways() {
        let args = build_args(&spec(EnforcementMode::HostPrompts), &[], None).unwrap();
        assert!(args.contains(&"-p".to_string()));
        assert!(has_pair(&args, "--input-format", "stream-json"));
        assert!(has_pair(&args, "--output-format", "stream-json"));
        assert!(args.contains(&"--include-partial-messages".to_string()));
    }

    #[test]
    fn host_prompts_route_permission_requests_to_the_host() {
        let args = build_args(&spec(EnforcementMode::HostPrompts), &[], None).unwrap();
        assert!(has_pair(&args, "--permission-prompts", "host"));
        assert!(has_pair(&args, "--permission-prompt-tool", "stdio"));
    }

    #[test]
    fn provider_denies_mode_never_asks_the_host() {
        let args = build_args(&spec(EnforcementMode::ProviderDenies), &[], None).unwrap();
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
        let args = build_args(&s, &[], None).unwrap();
        assert!(has_pair(&args, "--model", "haiku"));
        assert!(has_pair(&args, "--effort", "high"));
        assert!(has_pair(&args, "--append-system-prompt", "Be brief."));
        assert!(has_pair(&args, "--resume", "sess-1"));
        assert!(has_pair(&args, "--mcp-config", "/run/discard/mcp.json"));
    }

    #[test]
    fn omitted_settings_add_no_flags() {
        let args = build_args(&spec(EnforcementMode::HostPrompts), &[], None).unwrap();
        for flag in ["--effort", "--model", "--append-system-prompt", "--resume", "--mcp-config"] {
            assert!(!args.contains(&flag.to_string()), "{flag}");
        }
    }

    #[test]
    fn conversation_memory_permissions_allow_only_the_exact_scoped_tools() {
        for enforcement in [EnforcementMode::HostPrompts, EnforcementMode::ProviderDenies] {
            let mut s = spec(enforcement);
            assert!(!build_args(&s, &[], None).unwrap().iter().any(|arg| arg == "--allowedTools"));
            s.mode = SessionMode::Conversation;
            let args = build_args(&s, &[], None).unwrap();
            let allowed = args.iter().position(|arg| arg == "--allowedTools").expect("scoped memory permission list");
            assert_eq!(
                args[allowed + 1],
                "mcp__discard-memory__memory_search,mcp__discard-memory__memory_list,mcp__discard-memory__memory_read,mcp__discard-memory__memory_save,mcp__discard-memory__memory_edit,mcp__discard-memory__memory_forget,mcp__discard-memory__worktree_inspect,mcp__discard-memory__worktree_request_removal,mcp__discard-memory__worktree_request_creation,mcp__discard-memory__main_checkout_request_write,mcp__discard-memory__conversation_request_move,mcp__discard-memory__project_list_files,mcp__discard-memory__project_read_file,mcp__discard-memory__project_search,mcp__discard-memory__project_git_branches,mcp__discard-memory__project_git_log,mcp__discard-memory__project_git_compare"
            );
            assert!(args.windows(2).any(|pair| pair == ["--tools", ""]));
        }
    }

    #[test]
    fn the_delegation_tools_are_allowed_only_for_a_session_whose_config_has_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let config = |timeout: Option<u64>| {
            let mut entry = json!({"type":"http","url":"http://127.0.0.1:1/mcp","headers":{"Authorization":"Bearer t"}});
            if let Some(ms) = timeout { entry["timeout"] = json!(ms); }
            let path = dir.path().join(format!("{timeout:?}.json"));
            std::fs::write(&path, json!({"mcpServers":{"discard-memory":entry}}).to_string()).unwrap();
            path
        };
        let allowed = |mode: SessionMode, path: PathBuf| {
            let mut s = spec(EnforcementMode::HostPrompts); s.mode = mode; s.mcp_config = Some(path);
            let args = build_args(&s, &[PathBuf::from("/w/.git")], None).unwrap();
            args[args.iter().position(|arg| arg == "--allowedTools").unwrap() + 1].clone()
        };
        for mode in [SessionMode::Conversation, SessionMode::Workspace] {
            let without = allowed(mode, config(None));
            assert!(!without.contains("delegate") && !without.contains("agents_list") && !without.contains("agent_request_add"), "{mode:?}: {without}");
            let with = allowed(mode, config(Some(660_000)));
            for name in ["delegate", "agents_list", "agent_request_add"] { assert!(with.split(',').any(|t| t == format!("mcp__discard-memory__{name}")), "{mode:?} {name}: {with}"); }
            assert!(with.starts_with(&without), "the other tools are unchanged");
        }
    }

    #[test]
    fn conversation_mode_removes_native_tools_and_provider_context() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = discard_core::provider::SessionMode::Conversation;
        let args = build_args(&s, &[], None).unwrap();
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

    #[test]
    fn claude_reports_a_200k_context_window() {
        assert_eq!(ClaudeProvider::new("claude").model_context_window("any-model"), Some(200_000));
    }
}
