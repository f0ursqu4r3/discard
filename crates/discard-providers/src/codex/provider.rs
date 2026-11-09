//! Process management and the JSON-RPC client for `codex app-server`.

use crate::codex::parse::{Item, Mapper, PendingRequest, approval_response};
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
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const MATRIX_JSON: &str = include_str!("../../fixtures/codex.capabilities.json");
const STDERR_TAIL_LINES: usize = 20;
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const TERM_GRACE: Duration = Duration::from_secs(2);

pub struct CodexProvider {
    executable: PathBuf,
    matrix: CapabilityMatrix,
    env: Vec<(OsString, OsString)>,
}

impl CodexProvider {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        let matrix = CapabilityMatrix::from_json(MATRIX_JSON).expect("bundled codex capability matrix is valid");
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

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.executable);
        cmd.envs(self.env.iter().cloned());
        cmd
    }
}

impl CodexProvider {
    fn codex_home(&self) -> Result<PathBuf, ProviderError> {
        let value = |key: &str| self.env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone()).or_else(|| std::env::var_os(key));
        let home = value("CODEX_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| value("HOME").map(|p| PathBuf::from(p).join(".codex")))
            .ok_or_else(|| crate::conversation::setup("could not resolve Codex home"))?;
        if !home.is_absolute() {
            return Err(crate::conversation::setup("Codex home must be absolute"));
        }
        Ok(home)
    }

    async fn conversation_catalog(&self, home: &Path, model: &str, tools: crate::conversation::CatalogTools) -> Result<(Value, Option<u64>), ProviderError> {
        use crate::conversation::{load_catalog, setup};
        if let Ok(catalog) = load_catalog(home, model, tools) {
            return Ok(catalog);
        }
        // A model-list probe starts no thread and sends no user message. Let the
        // verified CLI rebuild its own cache, then validate the full records again.
        debug!("refreshing Codex model cache before conversation setup");
        let mut command = self.command();
        command.env("CODEX_HOME", home);
        crate::models::codex(command).await
            .map_err(|error| setup(format!("Automatic Codex model refresh failed: {error}")))?;
        load_catalog(home, model, tools)
            .map_err(|error| setup(format!("Codex model cache is still unavailable after one automatic refresh: {error}")))
    }

    async fn read_effective_config(&self, cwd: &Path, args: &[String]) -> Result<Value, ProviderError> {
        let mut child = self
            .command()
            .args(args)
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .map_err(|e| crate::conversation::setup(e.to_string()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| crate::conversation::setup("no configuration probe input"))?;
        let mut lines =
            BufReader::new(child.stdout.take().ok_or_else(|| crate::conversation::setup("no configuration probe output"))?).lines();
        let result = async {
            send_raw(
                &mut stdin,
                &json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"discard","version":env!("CARGO_PKG_VERSION")}}}),
            )
            .await?;
            response_for(&mut lines, 1).await?;
            send_raw(&mut stdin, &json!({"method":"initialized"})).await?;
            send_raw(&mut stdin, &json!({"id":2,"method":"config/read","params":{"cwd":cwd,"includeLayers":true}})).await?;
            response_for(&mut lines, 2).await
        }
        .await;
        if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = child.wait().await;
        result
            .map_err(|_| crate::conversation::setup("Codex could not report effective configuration"))
            .and_then(|v| crate::conversation::effective_config(&v))
    }
}

/// The parameters for `thread/start` (or `thread/resume` when `spec.resume_handle` is set).
/// Returns the method name and its params.
pub fn thread_params(spec: &SessionSpec) -> (&'static str, Value) {
    let (policy, sandbox) = match (spec.mode, spec.enforcement) {
        (SessionMode::Coding | SessionMode::Workspace, EnforcementMode::HostPrompts) => ("untrusted", "workspace-write"),
        _ => ("never", "read-only"),
    };
    let mut params = json!({"cwd": spec.cwd.to_string_lossy(), "approvalPolicy": policy, "sandbox": sandbox});
    if let Some(model) = &spec.model {
        params["model"] = json!(model);
    }
    if let Some(prompt) = &spec.system_prompt {
        params["developerInstructions"] = json!(prompt);
    }
    match &spec.resume_handle {
        Some(handle) => {
            params["threadId"] = json!(handle);
            ("thread/resume", params)
        }
        None => {
            // Saved threads can be resumed in a later process.
            params["ephemeral"] = json!(spec.mode.replays_transcript());
            if spec.mode.replays_transcript() {
                params["allowProviderModelFallback"] = json!(false);
            }
            ("thread/start", params)
        }
    }
}

/// Turn a Claude-format MCP config file (`{"mcpServers": {name: {url, headers}}}`) into Codex thread config.
fn mcp_thread_config(path: &Path) -> Result<Value, ProviderError> {
    let setup = |m: String| ProviderError::Setup(m);
    let text = std::fs::read_to_string(path).map_err(|e| setup(format!("could not read the MCP config: {e}")))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| setup(format!("the MCP config is not valid JSON: {e}")))?;
    let servers = doc.get("mcpServers").and_then(Value::as_object).ok_or_else(|| setup("the MCP config has no mcpServers".into()))?;
    let mut out = serde_json::Map::new();
    for (name, server) in servers {
        let url = server.get("url").and_then(Value::as_str).ok_or_else(|| setup(format!("MCP server `{name}` has no url")))?;
        let mut entry = json!({"url": url});
        if let Some(headers) = server.get("headers").filter(|h| h.is_object()) {
            entry["http_headers"] = headers.clone();
        }
        out.insert(name.clone(), entry);
    }
    Ok(json!({"mcp_servers": out}))
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> &str {
        "codex"
    }

    fn capabilities(&self) -> &CapabilityMatrix {
        &self.matrix
    }

    /// Checks that the CLI exists and reports a login through its exit status.
    /// Discard never reads, stores, or logs credentials.
    async fn models(&self) -> Result<Vec<discard_core::provider::ModelInfo>, ProviderError> {
        crate::models::codex(self.command()).await
    }

    async fn check_setup(&self) -> SetupStatus {
        let version =
            self.command().arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).status();
        match tokio::time::timeout(SETUP_TIMEOUT, version).await {
            Ok(Ok(_)) => {}
            _ => {
                return SetupStatus::NotInstalled { install_hint: "Install the Codex CLI and make sure `codex` is on your PATH.".into() };
            }
        }
        let status = self
            .command()
            .args(["login", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status();
        match tokio::time::timeout(SETUP_TIMEOUT, status).await {
            Ok(Ok(s)) if s.success() => SetupStatus::Ready,
            _ => SetupStatus::NotLoggedIn { login_command: "codex login".into() },
        }
    }

    fn model_context_window(&self, model: &str) -> Option<u64> {
        crate::conversation::load_catalog(&self.codex_home().ok()?, model, crate::conversation::CatalogTools::None).ok()?.1
    }

    async fn start_session(&self, mut spec: SessionSpec) -> Result<Box<dyn ProviderSession>, ProviderError> {
        let workspace = spec.mode == SessionMode::Workspace;
        if workspace && (spec.enforcement != EnforcementMode::HostPrompts || !spec.cwd.is_absolute() || !spec.cwd.is_dir()) {
            return Err(ProviderError::SandboxUnavailable("workspace sessions require host approvals and an existing absolute directory".into()));
        }
        // Fail closed before any process starts.
        self.matrix.supports(spec.enforcement)?;
        let mut files = None;
        let mut startup_args = Vec::new();
        let mut expected_controls = None;
        if spec.mode.replays_transcript() {
            use crate::conversation::*;
            read_memory_config(&spec)?;
            if spec.resume_handle.is_some() {
                return Err(setup("Codex conversation sessions require a fresh replay, not a native resume"));
            }
            let model = spec.model.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| setup("select an exact Codex model"))?;
            let version = tokio::time::timeout(SETUP_TIMEOUT, self.command().arg("--version").kill_on_drop(true).output())
                .await
                .map_err(|_| setup("Codex version check timed out"))?
                .map_err(|e| setup(e.to_string()))?;
            if !version.status.success() || String::from_utf8_lossy(&version.stdout).trim() != "codex-cli 0.166.0" {
                return Err(setup("Codex 0.166.0 is required for the verified conversation controls"));
            }
            let owned = ConversationFiles::new()?;
            if !workspace { spec.cwd = owned.cwd.clone(); }
            let tools = if workspace { CatalogTools::Workspace } else { CatalogTools::None };
            let (catalog, _) = self.conversation_catalog(&self.codex_home()?, model, tools).await?;
            let catalog_path = owned.root.path().join("models.json");
            std::fs::write(&catalog_path, catalog.to_string()).map_err(|e| setup(e.to_string()))?;
            startup_args.extend(["-c".into(), format!("model_catalog_json={}", json!(catalog_path))]);
            // A config-only app-server starts no thread, model request, or external MCP client.
            let initial = codex_controls(&json!({}), tools)?;
            let mut probe_args = startup_args.clone();
            probe_args.extend(config_args(&initial));
            let effective = self.read_effective_config(&spec.cwd, &probe_args).await?;
            let mut controls = codex_controls(&effective, tools)?;
            controls["model_catalog_json"] = json!(catalog_path);
            startup_args.extend(config_args(&controls));
            expected_controls = Some(controls);
            files = Some(owned);
        }
        let (method, mut params) = thread_params(&spec);
        if let Some(path) = &spec.mcp_config {
            params["config"] = mcp_thread_config(path)?;
            if let Some(controls) = &expected_controls {
                let memory = params["config"]["mcp_servers"]["discard-memory"].clone();
                // Per-thread top-level overrides replace matching startup overrides.
                // Carry the complete disabled server map so lower layers cannot revive it.
                params["config"] = controls.clone();
                params["config"]["mcp_servers"]["discard-memory"] = memory;
            }
            if spec.mode.replays_transcript() {
                params["config"]["mcp_servers"]["discard-memory"]["enabled"] = json!(true);
                // The scoped memory service owns shared-write approval. Default auto
                // mode would require a CLI prompt that never policy cannot answer.
                params["config"]["mcp_servers"]["discard-memory"]["default_tools_approval_mode"] = json!("approve");
                params["config"]["mcp_servers"]["discard-memory"]["omit_tools_from"] = json!(["deferred", "code_mode"]);
                params["config"]["mcp_servers"]["discard-memory"]["enabled_tools"] = json!(crate::conversation::MEMORY_TOOLS.iter().chain(if workspace { crate::conversation::WORKSPACE_TOOLS } else { &[] }).collect::<Vec<_>>());
            }
        }

        let mut command = self.command();
        if workspace { command.env("GIT_OPTIONAL_LOCKS", "0"); }
        let mut child = command
            .args(&startup_args)
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            // Own process group, so cancel can kill codex and everything it started.
            .process_group(0)
            .spawn()
            .map_err(|e| ProviderError::Spawn(format!("{}: {e}", self.executable.display())))?;
        let pgid = child.id().and_then(|p| i32::try_from(p).ok());
        let mut stdin = child.stdin.take().ok_or_else(|| ProviderError::Spawn("no stdin pipe".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| ProviderError::Spawn("no stdout pipe".into()))?;
        let stderr = child.stderr.take().ok_or_else(|| ProviderError::Spawn("no stderr pipe".into()))?;

        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let stderr_task = {
            let tail = Arc::clone(&tail);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = redact(&line);
                    debug!(target: "codex_stderr", "{line}");
                    let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            })
        };

        let mut lines = BufReader::new(stdout).lines();
        let handshake = handshake(&mut stdin, &mut lines, method, params, expected_controls.as_ref()).await;
        let thread_id = match handshake {
            Ok(id) => id,
            Err(failure) => {
                let _ = tokio::time::timeout(Duration::from_millis(500), stderr_task).await;
                let detail = tail.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");
                if let Some(pgid) = pgid {
                    let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
                }
                return Err(match failure {
                    Failure::Controls(message) if workspace => ProviderError::SandboxUnavailable(redact(&message)),
                    Failure::Controls(message) => ProviderError::Protocol(redact(&message)),
                    Failure::Rejected(message) if method == "thread/resume" => ProviderError::ResumeRejected(redact(&message)),
                    Failure::Rejected(message) => ProviderError::Protocol(redact(&message)),
                    Failure::Closed => ProviderError::Protocol(format!("codex exited before the handshake finished: {detail}")),
                    Failure::Timeout => ProviderError::Protocol("codex did not answer the handshake in time".into()),
                });
            }
        };

        let stdin = Arc::new(tokio::sync::Mutex::new(Some(stdin)));
        let shared = Arc::new(Shared::default());
        let (tx, events) = mpsc::unbounded_channel();
        {
            let (stdin, shared) = (Arc::clone(&stdin), Arc::clone(&shared));
            tokio::spawn(async move {
                let mut mapper = Mapper::with_mode(spec.mode);
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if msg.get("method").is_none() && msg.get("id").is_some() {
                        // An answer to one of our requests. Only failures matter.
                        if let Some(message) = msg.pointer("/error/message").and_then(Value::as_str) {
                            let event = RawEvent { kind: EventKind::Error { message: redact(message) }, raw: None };
                            if tx.send(event).is_err() {
                                return;
                            }
                        }
                        continue;
                    }
                    for item in mapper.on_message(&msg) {
                        match item {
                            Item::TurnStarted(_) => {}
                            Item::Event(event) => {
                                if tx.send(event).is_err() {
                                    return;
                                }
                            }
                            Item::Request(req) => {
                                shared.requests.lock().unwrap_or_else(|p| p.into_inner()).insert(req.approval_id.clone(), req);
                            }
                            Item::RejectRequest { rpc_id, message } => {
                                let reply = json!({"jsonrpc": "2.0", "id": rpc_id, "error": {"code": -32601, "message": message}});
                                if let Err(e) = write_line(&stdin, &reply).await {
                                    warn!(error = %e, "could not decline an unsupported codex request");
                                }
                            }
                        }
                    }
                }
            });
        }

        Ok(Box::new(CodexSession { child, pgid, stdin, events, shared, thread_id, reasoning_effort: spec.reasoning_effort, next_id: 10, _conversation_files: files }) as Box<dyn ProviderSession>)
    }
}

enum Failure {
    Controls(String),
    /// The server answered with a JSON-RPC error.
    Rejected(String),
    Closed,
    Timeout,
}

async fn write_line(stdin: &tokio::sync::Mutex<Option<ChildStdin>>, value: &Value) -> Result<(), ProviderError> {
    let mut guard = stdin.lock().await;
    let Some(pipe) = guard.as_mut() else {
        return Ok(());
    };
    let mut line = serde_json::to_string(value).map_err(|e| ProviderError::Protocol(e.to_string()))?;
    line.push('\n');
    match pipe.write_all(line.as_bytes()).await {
        Ok(()) => pipe.flush().await.or_else(ignore_closed_pipe),
        Err(e) => ignore_closed_pipe(e),
    }
}

/// A closed pipe means the process already exited. The event stream reports that.
fn ignore_closed_pipe(e: std::io::Error) -> Result<(), ProviderError> {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        warn!("codex closed its input; waiting for the event stream to report why");
        Ok(())
    } else {
        Err(ProviderError::Io(e.to_string()))
    }
}

/// Read lines until the response with `id` arrives. Other messages before the handshake ends are ignored.
async fn response_for(lines: &mut tokio::io::Lines<BufReader<ChildStdout>>, id: i64) -> Result<Value, Failure> {
    let read = async {
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                _ => return Err(Failure::Closed),
            };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("method").is_some() || msg.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(message) = msg.pointer("/error/message").and_then(Value::as_str) {
                return Err(Failure::Rejected(message.to_string()));
            }
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, read).await.unwrap_or(Err(Failure::Timeout))
}

async fn send_raw(stdin: &mut ChildStdin, value: &Value) -> Result<(), Failure> {
    let mut line = value.to_string();
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.map_err(|_| Failure::Closed)?;
    stdin.flush().await.map_err(|_| Failure::Closed)
}

/// `initialize`, `initialized`, then `thread/start` or `thread/resume`. Returns the thread ID.
async fn handshake(
    stdin: &mut ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<ChildStdout>>,
    method: &str,
    params: Value,
    expected_controls: Option<&Value>,
) -> Result<String, Failure> {
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"clientInfo": {"name": "discard", "title": "Discard", "version": env!("CARGO_PKG_VERSION")}}});
    send_raw(stdin, &init).await?;
    response_for(lines, 1).await?;
    send_raw(stdin, &json!({"jsonrpc": "2.0", "method": "initialized"})).await?;
    if let Some(expected) = expected_controls {
        send_raw(stdin, &json!({"id":3,"method":"config/read","params":{"cwd":params["cwd"],"includeLayers":true}})).await?;
        let effective = response_for(lines, 3).await?;
        crate::conversation::validate_controls(
            &crate::conversation::effective_config(&effective).map_err(|e| Failure::Rejected(e.to_string()))?,
            expected,
        )
        .map_err(|e| Failure::Controls(e.to_string()))?;
        // A newly appearing inherited server must not initialize through thread/start.
        let actual_servers = effective["config"]["mcp_servers"]
            .as_object()
            .ok_or_else(|| Failure::Rejected("effective MCP configuration is malformed".into()))?;
        if actual_servers.values().any(|s| s.get("enabled") != Some(&json!(false))) {
            return Err(Failure::Controls("an inherited MCP server remains enabled".into()));
        }
    }
    send_raw(stdin, &json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params})).await?;
    let result = response_for(lines, 2).await?;
    result
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Failure::Rejected("codex did not return a thread id".into()))
}

#[derive(Default)]
struct Shared {
    requests: Mutex<HashMap<ApprovalId, PendingRequest>>,
}

struct CodexSession {
    child: Child,
    pgid: Option<i32>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    events: mpsc::UnboundedReceiver<RawEvent>,
    shared: Arc<Shared>,
    thread_id: String,
    reasoning_effort: Option<String>,
    next_id: i64,
    _conversation_files: Option<crate::conversation::ConversationFiles>,
}

impl CodexSession {
    fn kill_group(&self, signal: Signal) {
        if let Some(pgid) = self.pgid {
            // ESRCH just means the group is already gone.
            let _ = killpg(Pid::from_raw(pgid), signal);
        }
    }
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        self.kill_group(Signal::SIGKILL);
    }
}

#[async_trait]
impl ProviderSession for CodexSession {
    fn resume_handle(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    async fn send_user_message(&mut self, text: &str) -> Result<(), ProviderError> {
        self.send_user_message_with_images(text, &[]).await
    }
    async fn send_user_message_with_images(&mut self, text: &str, images: &[discard_core::attachments::Attachment]) -> Result<(), ProviderError> {
        self.next_id += 1;
        let input = crate::attachments::codex_input(text, images)?;
        let mut request = json!({"jsonrpc": "2.0", "id": self.next_id, "method": "turn/start",
            "params": {"threadId": self.thread_id, "input": input}});
        if let Some(effort) = &self.reasoning_effort { request["params"]["effort"] = json!(effort); }
        write_line(&self.stdin, &request).await
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
            .ok_or_else(|| ProviderError::Protocol(format!("no pending codex request for approval {id}")))?;
        let result = approval_response(request.kind, decision, &request.available);
        write_line(&self.stdin, &json!({"jsonrpc": "2.0", "id": request.rpc_id, "result": result})).await
    }

    async fn cancel(&mut self) -> Result<(), ProviderError> {
        *self.stdin.lock().await = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn spec(enforcement: EnforcementMode) -> SessionSpec {
        SessionSpec {
            reasoning_effort: None,
            mode: discard_core::provider::SessionMode::Coding,
            cwd: PathBuf::from("/work/proj"),
            model: None,
            system_prompt: None,
            enforcement,
            resume_handle: None,
            mcp_config: None,
        }
    }

    #[test]
    fn workspace_thread_uses_untrusted_and_workspace_write() {
        let mut s = spec(EnforcementMode::HostPrompts); s.mode = SessionMode::Workspace;
        let (_, p) = thread_params(&s);
        assert_eq!(p["approvalPolicy"], "untrusted"); assert_eq!(p["sandbox"], "workspace-write");
        assert_eq!(p["ephemeral"], true); assert_eq!(p["allowProviderModelFallback"], false);
        assert_eq!(p["cwd"], s.cwd.to_string_lossy().as_ref());
    }

    #[test]
    fn a_new_thread_asks_before_acting_in_a_workspace_sandbox() {
        let (method, p) = thread_params(&spec(EnforcementMode::HostPrompts));
        assert_eq!(method, "thread/start");
        assert_eq!(p["approvalPolicy"], "untrusted");
        assert_eq!(p["sandbox"], "workspace-write");
        assert_eq!(p["cwd"], "/work/proj");
        assert_eq!(p["ephemeral"], false, "the thread must be saved so it can be resumed");
    }

    #[test]
    fn provider_denies_mode_never_asks_and_cannot_write() {
        let (_, p) = thread_params(&spec(EnforcementMode::ProviderDenies));
        assert_eq!(p["approvalPolicy"], "never");
        assert_eq!(p["sandbox"], "read-only");
    }

    #[test]
    fn a_resume_handle_selects_thread_resume() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.resume_handle = Some("th-1".into());
        let (method, p) = thread_params(&s);
        assert_eq!((method, p["threadId"].clone()), ("thread/resume", json!("th-1")));
        assert!(p.get("ephemeral").is_none(), "resume params have no ephemeral flag");
    }

    #[test]
    fn model_and_system_prompt_are_passed_when_set_and_omitted_otherwise() {
        let (_, bare) = thread_params(&spec(EnforcementMode::HostPrompts));
        assert!(bare.get("model").is_none() && bare.get("developerInstructions").is_none());
        let mut s = spec(EnforcementMode::HostPrompts);
        s.model = Some("gpt-x".into());
        s.system_prompt = Some("Be brief.".into());
        let (_, p) = thread_params(&s);
        assert_eq!((p["model"].clone(), p["developerInstructions"].clone()), (json!("gpt-x"), json!("Be brief.")));
    }

    #[test]
    fn conversation_mode_never_uses_model_fallback_or_coding_sandbox() {
        let mut s = spec(EnforcementMode::HostPrompts);
        s.mode = discard_core::provider::SessionMode::Conversation;
        s.model = Some("exact-model".into());
        let (_, p) = thread_params(&s);
        assert_eq!(p["allowProviderModelFallback"], false);
        assert_eq!(p["approvalPolicy"], "never");
        assert_eq!(p["sandbox"], "read-only");
        assert_eq!(p["model"], "exact-model");
    }

    #[test]
    fn bundled_capability_matrix_describes_codex_with_verified_approvals() {
        let p = CodexProvider::new("codex");
        let c = &p.capabilities().capabilities;
        assert_eq!(p.capabilities().provider, "codex");
        assert!(c.interactive_approvals && c.pre_execution_blocking && c.session_resume && c.cancellation && c.per_session_mcp);
    }
}
