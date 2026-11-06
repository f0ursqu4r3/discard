//! Read provider model metadata without sending a user message or starting a thread.
use discard_core::provider::{ModelInfo, ProviderError, ReasoningOption};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{collections::HashSet, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

type Result<T> = std::result::Result<T, ProviderError>;
fn protocol(message: &str) -> ProviderError {
    ProviderError::Protocol(message.into())
}

// Also kill descendants when the request is cancelled, not only on timeout or success.
struct Probe(Child);
impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(pid) = self.0.id().and_then(|id| i32::try_from(id).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
    }
}
async fn send(stdin: &mut ChildStdin, value: Value) -> Result<()> {
    stdin
        .write_all(format!("{value}\n").as_bytes())
        .await
        .map_err(|e| ProviderError::Io(e.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|e| ProviderError::Io(e.to_string()))
}
async fn next(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<Value> {
    let line = lines
        .next_line()
        .await
        .map_err(|e| ProviderError::Io(e.to_string()))?
        .ok_or_else(|| protocol("The provider closed before returning its model list."))?;
    serde_json::from_str(&line)
        .map_err(|_| protocol("The provider returned invalid model metadata."))
}
async fn rpc(lines: &mut Lines<BufReader<ChildStdout>>, id: u64) -> Result<Value> {
    loop {
        let value = next(lines).await?;
        if value["id"].as_u64() == Some(id) {
            if value.get("error").is_some() {
                return Err(protocol(
                    "The provider could not list models. Recheck its connection and retry.",
                ));
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| protocol("Missing model response."));
        }
    }
}
fn reasoning_options(row: &Value, claude: bool) -> Vec<ReasoningOption> {
    let key = if claude { "supportedEffortLevels" } else { "supportedReasoningEfforts" };
    if claude && row["supportsEffort"] == false { return vec![]; }
    let mut seen = HashSet::new();
    row[key].as_array().into_iter().flatten().filter_map(|entry| {
        let value = if claude { entry.as_str() } else { entry["reasoningEffort"].as_str() }?;
        if !matches!(value, "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra") || !seen.insert(value) { return None; }
        Some(ReasoningOption { value: value.into(), description: entry["description"].as_str().unwrap_or_default().into() })
    }).collect()
}

fn parse_models(rows: &Value, claude: bool) -> Result<Vec<ModelInfo>> {
    let mut rows: Vec<_> = rows
        .as_array()
        .ok_or_else(|| protocol("Missing provider model list."))?
        .iter()
        .collect();
    // Prefer a named model over the equivalent default alias.
    rows.sort_by_key(|row| row["value"] == "default");
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        if row["hidden"] == true {
            continue;
        }
        let value = if claude {
            row["value"].as_str()
        } else {
            row["model"].as_str()
        };
        let Some(value) = value.filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let id = if claude && matches!(value, "default" | "opus" | "sonnet" | "haiku") {
            row["resolvedModel"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(value)
        } else {
            value
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        out.push(ModelInfo {
            reasoning_efforts: reasoning_options(row, claude),
            id: id.into(),
            name: row["displayName"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(id)
                .into(),
            description: row["description"].as_str().unwrap_or_default().into(),
        });
    }
    Ok(out)
}

pub(crate) async fn claude(mut command: Command) -> Result<Vec<ModelInfo>> {
    command.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--disable-slash-commands",
        "--settings",
        "{\"disableAllHooks\":true,\"autoMemoryEnabled\":false}",
    ]);
    probe(command, true, Duration::from_secs(20)).await
}
pub(crate) async fn codex(mut command: Command) -> Result<Vec<ModelInfo>> {
    command.args(["app-server", "--listen", "stdio://"]);
    probe(command, false, Duration::from_secs(20)).await
}
async fn probe(mut command: Command, claude: bool, timeout: Duration) -> Result<Vec<ModelInfo>> {
    let cwd = tempfile::tempdir().map_err(|e| ProviderError::Io(e.to_string()))?;
    let mut child = Probe(
        command
            .current_dir(cwd.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .map_err(|e| ProviderError::Spawn(e.to_string()))?,
    );
    let mut stdin = child
        .0
        .stdin
        .take()
        .ok_or_else(|| protocol("Missing model probe input."))?;
    let mut lines = BufReader::new(
        child
            .0
            .stdout
            .take()
            .ok_or_else(|| protocol("Missing model probe output."))?,
    )
    .lines();
    let result = tokio::time::timeout(timeout, async {
        if claude {
            send(&mut stdin, json!({"type":"control_request","request_id":"models","request":{"subtype":"initialize"}})).await?;
            loop {
                let value = next(&mut lines).await?;
                if value["type"] == "control_response" && value["response"]["request_id"] == "models" {
                    if value["response"]["subtype"] == "error" { return Err(protocol("Claude could not list models. Recheck its connection and retry.")); }
                    return parse_models(&value["response"]["response"]["models"], true);
                }
            }
        }
        send(&mut stdin, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"discard","version":env!("CARGO_PKG_VERSION")}}})).await?;
        rpc(&mut lines, 1).await?;
        send(&mut stdin, json!({"method":"initialized"})).await?;
        let mut cursor = Value::Null;
        let mut cursors = HashSet::new();
        let mut models = Vec::new();
        for id in 2..102 {
            send(&mut stdin, json!({"id":id,"method":"model/list","params":{"limit":100,"includeHidden":false,"cursor":cursor}})).await?;
            let page = rpc(&mut lines, id).await?;
            models.extend(parse_models(&page["data"], false)?);
            let next = page.get("nextCursor").and_then(Value::as_str).filter(|s| !s.is_empty());
            let Some(next) = next else { return Ok(models); };
            if !cursors.insert(next.to_owned()) { return Err(protocol("The provider repeated a model-list page.")); }
            cursor = json!(next);
        }
        Err(protocol("The provider returned too many model-list pages."))
    }).await.unwrap_or_else(|_| Err(ProviderError::Setup("Loading models timed out. Recheck the provider connection and retry.".into())));
    if let Some(pid) = child.0.id().and_then(|id| i32::try_from(id).ok()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
    let _ = child.0.wait().await;
    result.map(|models| {
        let mut seen = HashSet::new();
        models
            .into_iter()
            .filter(|m| seen.insert(m.id.clone()))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_catalogs_expose_supported_reasoning_levels_only() {
        let claude = parse_models(&json!([
            {"value":"thinking","supportsEffort":true,"supportedEffortLevels":["low","high","max"]},
            {"value":"no-thinking","supportsEffort":false,"supportedEffortLevels":["low"]},
            {"value":"legacy"}
        ]), true).unwrap();
        assert_eq!(claude[0].reasoning_efforts.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(), ["low", "high", "max"]);
        assert!(claude[1].reasoning_efforts.is_empty());
        assert!(claude[2].reasoning_efforts.is_empty());
        let codex = parse_models(&json!([{"model":"m","supportedReasoningEfforts":[
            {"reasoningEffort":"high","description":"Think longer"},
            {"reasoningEffort":"high"}, {"reasoningEffort":"ultra"}, {"reasoningEffort":"invalid"}
        ]}]), false).unwrap();
        assert_eq!(codex[0].reasoning_efforts.len(), 2);
        assert_eq!(codex[0].reasoning_efforts[0].description, "Think longer");
        assert_eq!(serde_json::to_value(&codex[0]).unwrap()["reasoningEfforts"][1]["value"], "ultra");
    }

    #[test]
    fn claude_resolves_aliases_deduplicates_default_and_keeps_explicit_context_variants() {
        let rows = json!([
            {"value":"default","resolvedModel":"claude-exact","displayName":"Default"},
            {"value":"opus","resolvedModel":"claude-exact","displayName":"Opus"},
            {"value":"claude-special[1m]","resolvedModel":"claude-special","displayName":"Special"}
        ]);
        let models = parse_models(&rows, true).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-exact");
        assert_eq!(models[0].name, "Opus");
        assert_eq!(models[1].id, "claude-special[1m]");
    }
    #[test]
    fn codex_uses_model_value_and_omits_hidden_and_invalid_rows() {
        let models = parse_models(
            &json!([
                {"id":"picker-id","model":"exact","displayName":"Model"},
                {"model":"hidden","hidden":true}, {"model":""}, {"model":"exact"}
            ]),
            false,
        )
        .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "exact");
        assert!(parse_models(&Value::Null, false).is_err());
        assert!(parse_models(&json!([]), false).unwrap().is_empty());
    }

    #[tokio::test]
    async fn codex_probe_reads_all_pages_without_starting_a_thread() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            r#"
            IFS= read -r request
            case "$request" in *initialize*) ;; *) exit 1 ;; esac
            echo '{"id":1,"result":{}}'
            IFS= read -r request
            case "$request" in *initialized*) ;; *) exit 1 ;; esac
            IFS= read -r request
            case "$request" in *model/list*) ;; *) exit 1 ;; esac
            echo '{"id":2,"result":{"data":[{"model":"first"}],"nextCursor":"page2"}}'
            IFS= read -r request
            case "$request" in *page2*) ;; *) exit 1 ;; esac
            echo '{"id":3,"result":{"data":[{"model":"second"}],"nextCursor":null}}'
            sleep 30
        "#,
        ]);
        let models = probe(command, false, Duration::from_secs(2)).await.unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
    }
    #[tokio::test]
    async fn claude_probe_uses_initialization_metadata_only() {
        let mut command = Command::new("sh");
        command.args(["-c", r#"
            IFS= read -r request
            case "$request" in *control_request*) ;; *) exit 1 ;; esac
            case "$request" in *initialize*) ;; *) exit 1 ;; esac
            echo '{"type":"control_response","response":{"request_id":"models","subtype":"success","response":{"models":[{"value":"exact","displayName":"Exact"}]}}}'
            sleep 30
        "#]);
        let models = probe(command, true, Duration::from_secs(2)).await.unwrap();
        assert_eq!(models[0].id, "exact");
    }
    #[tokio::test]
    async fn stalled_and_failed_probes_return_errors() {
        let mut stalled = Command::new("sh");
        stalled.args(["-c", "sleep 30"]);
        assert!(
            probe(stalled, true, Duration::from_millis(30))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        let mut failed = Command::new("sh");
        failed.args(["-c", "exit 1"]);
        assert!(probe(failed, false, Duration::from_secs(1)).await.is_err());
    }

    #[tokio::test]
    #[ignore = "requires installed and connected CLIs; reads model metadata only"]
    async fn installed_cli_model_catalogs() {
        use discard_core::provider::Provider;
        for provider in [
            Box::new(crate::claude::ClaudeProvider::new("claude")) as Box<dyn Provider>,
            Box::new(crate::codex::CodexProvider::new("codex")) as Box<dyn Provider>,
        ] {
            let models = provider.models().await.unwrap();
            assert!(!models.is_empty());
            println!("{}: {} models", provider.id(), models.len());
        }
    }
}
