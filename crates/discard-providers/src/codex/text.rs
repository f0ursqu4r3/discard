//! Isolated app-server requests. No conversation session or memory MCP is created.
use super::{provider::{CodexProvider, send_raw}, parse::{Mapper, Item, approval_response}};
use crate::{conversation::{self, CatalogTools, ConversationFiles}, text::{TextProcess, TIMEOUT, protocol, timed_out}};
use discard_core::{approval::Decision, provider::{ProviderError, TextRequest}};
use serde_json::{Value, json};
use std::path::Path;
use tokio::{io::{AsyncBufReadExt, BufReader, Lines}, process::{ChildStdin, ChildStdout}};

type Result<T> = std::result::Result<T, ProviderError>;
type Output = Lines<BufReader<ChildStdout>>;

fn thread_params(request: &TextRequest, cwd: &Path) -> Value {
    json!({"ephemeral":true, "approvalPolicy":"never", "sandbox":"read-only",
        "model":request.model, "developerInstructions":request.instructions,
        "allowProviderModelFallback":false, "cwd":cwd})
}
fn turn_params(request: &TextRequest, thread: &str) -> Value {
    let mut params = json!({"threadId":thread, "input":[{"type":"text", "text":request.prompt}]});
    if let Some(effort) = &request.reasoning_effort { params["effort"] = json!(effort); }
    params
}

/// Completed messages contain the full text; deltas must not duplicate it.
#[derive(Default)]
struct TextOutput { last: String, final_answer: Option<String> }
impl TextOutput {
    fn notification(&mut self, message: &Value) -> Result<Option<String>> {
        match message["method"].as_str() {
            Some("item/completed") if message["params"]["item"]["type"] == "agentMessage" => {
                let item = &message["params"]["item"];
                if let Some(text) = item["text"].as_str() {
                    self.last = text.into();
                    if item["phase"] == "final_answer" { self.final_answer = Some(text.into()); }
                }
            }
            Some("turn/completed") => {
                if message["params"]["turn"]["status"] != "completed" {
                    return Err(protocol("Codex could not generate text. Try again."));
                }
                return Ok(Some(self.final_answer.clone().unwrap_or_else(|| self.last.clone())));
            }
            Some("error") if message["params"]["willRetry"] != true => {
                return Err(protocol("Codex could not generate text. Try again."));
            }
            _ => {}
        }
        Ok(None)
    }
}

async fn send(stdin: &mut ChildStdin, value: Value) -> Result<()> {
    send_raw(stdin, &value).await.map_err(|_| protocol("Codex closed before returning text."))
}
async fn next(lines: &mut Output) -> Result<Value> {
    let line = lines.next_line().await.map_err(|e| ProviderError::Io(e.to_string()))?
        .ok_or_else(|| protocol("Codex closed before returning text."))?;
    serde_json::from_str(&line).map_err(|_| protocol("Codex returned invalid text output."))
}

fn denial(message: &Value) -> Option<Value> {
    let id = message.get("id")?;
    message.get("method")?;
    for item in Mapper::new().on_message(message) {
        if let Item::Request(request) = item {
            return Some(json!({"jsonrpc":"2.0", "id":id,
                "result":approval_response(request.kind, Decision::Deny, &request.available)}));
        }
    }
    Some(json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"Text generation does not permit tools or approvals."}}))
}

// Unlike the session handshake helper, this declines requests during initialization too.
async fn response(stdin: &mut ChildStdin, lines: &mut Output, id: i64) -> Result<Value> {
    loop {
        let message = next(lines).await?;
        if let Some(reply) = denial(&message) { send(stdin, reply).await?; continue; }
        if message.get("method").is_none() && message["id"] == id {
            if message.get("error").is_some() { return Err(protocol("Codex rejected the text request.")); }
            return message.get("result").cloned().ok_or_else(|| protocol("Codex returned no response."));
        }
    }
}
async fn initialize(stdin: &mut ChildStdin, lines: &mut Output) -> Result<()> {
    send(stdin, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"discard","version":env!("CARGO_PKG_VERSION")}}})).await?;
    response(stdin, lines, 1).await?;
    send(stdin, json!({"method":"initialized"})).await
}
async fn read_config(stdin: &mut ChildStdin, lines: &mut Output, cwd: &Path) -> Result<Value> {
    send(stdin, json!({"id":2,"method":"config/read","params":{"cwd":cwd,"includeLayers":true}})).await?;
    conversation::effective_config(&response(stdin, lines, 2).await?)
}
fn pipes(process: &mut TextProcess) -> (ChildStdin, Output) {
    (process.child.stdin.take().expect("piped stdin"), BufReader::new(process.child.stdout.take().expect("piped stdout")).lines())
}

pub(super) async fn generate(provider: &CodexProvider, request: TextRequest) -> Result<String> {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let running = provider.read_version().await?;
    tokio::time::timeout_at(deadline, generate_inner(provider, request, &running)).await.unwrap_or_else(|_| {
        Err(crate::version::verification_error(timed_out(), "Codex", &running, crate::version::CODEX_TESTED))
    })
}
async fn generate_inner(provider: &CodexProvider, request: TextRequest, running: &str) -> Result<String> {
    let verification_error = |e| crate::version::verification_error(e, "Codex", running, crate::version::CODEX_TESTED);
    let files = ConversationFiles::new()?;
    let (catalog, _) = provider.conversation_catalog(&provider.codex_home()?, &request.model, CatalogTools::None, running).await?;
    let catalog_path = files.root.path().join("models.json");
    std::fs::write(&catalog_path, catalog.to_string()).map_err(|e| ProviderError::Io(e.to_string()))?;
    let mut controls = conversation::codex_controls(&json!({}), CatalogTools::None).map_err(verification_error)?;
    controls["model_catalog_json"] = json!(catalog_path);
    let mut command = provider.command();
    command.args(conversation::config_args(&controls)).args(["app-server", "--listen", "stdio://"]);
    let mut probe = TextProcess::spawn(command, &files.cwd).map_err(verification_error)?;
    let (mut stdin, mut lines) = pipes(&mut probe);
    let effective = async {
        initialize(&mut stdin, &mut lines).await?;
        read_config(&mut stdin, &mut lines, &files.cwd).await
    }.await;
    let effective = probe.finish(effective).await.map_err(verification_error)?;
    // A config probe starts no thread or MCP client. Disable every inherited server
    // before the actual thread, then verify that managed policy kept the controls.
    controls = conversation::codex_controls(&effective, CatalogTools::None).map_err(verification_error)?;
    controls["model_catalog_json"] = json!(catalog_path);
    let mut command = provider.command();
    command.args(conversation::config_args(&controls)).args(["app-server", "--listen", "stdio://"]);
    let mut process = TextProcess::spawn(command, &files.cwd).map_err(verification_error)?;
    let (mut stdin, mut lines) = pipes(&mut process);
    let result = async {
        async {
            initialize(&mut stdin, &mut lines).await?;
            let effective = read_config(&mut stdin, &mut lines, &files.cwd).await?;
            conversation::validate_controls(&effective, &controls)?;
            let servers = effective["mcp_servers"].as_object().ok_or_else(|| protocol("Codex returned invalid MCP controls."))?;
            if servers.values().any(|server| server["enabled"] != false) {
                return Err(protocol("An inherited MCP server remains enabled."));
            }
            Ok(())
        }.await.map_err(verification_error)?;
        let mut params = thread_params(&request, &files.cwd);
        params["config"] = controls;
        send(&mut stdin, json!({"id":3,"method":"thread/start","params":params})).await?;
        let response = response(&mut stdin, &mut lines, 3).await?;
        let thread = response["thread"]["id"].as_str().ok_or_else(|| protocol("Codex returned no thread ID."))?;
        send(&mut stdin, json!({"id":4,"method":"turn/start","params":turn_params(&request, thread)})).await?;
        let mut output = TextOutput::default();
        loop {
            let message = next(&mut lines).await?;
            if let Some(reply) = denial(&message) { send(&mut stdin, reply).await?; continue; }
            if message.get("method").is_none() && message.get("error").is_some() {
                return Err(protocol("Codex rejected the text request."));
            }
            if let Some(text) = output.notification(&message)? { return Ok(text); }
        }
    }.await;
    process.finish(result).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> TextRequest { TextRequest { model: "exact-model".into(), reasoning_effort: Some("high".into()), instructions: "personality instructions".into(), prompt: "description".into() } }
    #[test]
    fn thread_is_ephemeral_and_tool_free_with_the_draft_model_and_instructions() {
        let request = request();
        let params = thread_params(&request, Path::new("/empty"));
        assert_eq!(params, json!({"ephemeral":true, "approvalPolicy":"never", "sandbox":"read-only", "model":"exact-model", "developerInstructions":"personality instructions", "allowProviderModelFallback":false, "cwd":"/empty"}));
        let params = turn_params(&request, "thread");
        assert_eq!(params["effort"], "high");
        assert_eq!(params["input"][0]["text"], "description");
        let mut request = request; request.reasoning_effort = None;
        assert!(turn_params(&request, "thread").get("effort").is_none());
    }
    #[test]
    fn notifications_return_only_the_final_message_without_duplicate_deltas() {
        let mut output = TextOutput::default();
        let results: Vec<_> = include_str!("../../fixtures/codex-text-turn.jsonl").lines().filter_map(|line| {
            output.notification(&serde_json::from_str(line).unwrap()).unwrap()
        }).collect();
        assert_eq!(results, ["You are direct. You check assumptions."]);
        for status in ["failed", "interrupted", "inProgress"] {
            assert!(TextOutput::default().notification(&json!({"method":"turn/completed","params":{"turn":{"status":status}}})).is_err());
        }
        assert!(TextOutput::default().notification(&json!({"method":"error","params":{"willRetry":false}})).is_err());
        assert!(TextOutput::default().notification(&json!({"method":"error","params":{"willRetry":true}})).unwrap().is_none());
    }
    #[test]
    fn approvals_and_unknown_tool_requests_are_declined() {
        for method in ["item/commandExecution/requestApproval", "item/fileChange/requestApproval"] {
            let reply = denial(&json!({"id":9,"method":method,"params":{"availableDecisions":["accept","decline"]}})).unwrap();
            assert_eq!(reply["id"], 9);
            assert_eq!(reply["result"]["decision"], "decline");
        }
        for method in ["item/tool/call", "item/permissions/requestApproval", "item/tool/requestUserInput", "unknown"] {
            assert!(denial(&json!({"id":"request","method":method,"params":{}})).unwrap().get("error").is_some());
        }
    }
}
