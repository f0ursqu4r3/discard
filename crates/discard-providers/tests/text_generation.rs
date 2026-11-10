//! One-shot text requests against local processes; no live providers or network.
use discard_core::provider::{Provider, ProviderError, TextRequest};
use discard_providers::codex::CodexProvider;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fake-codex.py");
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/codex-text-turn.jsonl");
fn request() -> TextRequest {
    TextRequest { model: "exact-model".into(), reasoning_effort: Some("high".into()), instructions: "Personality only".into(), prompt: "A terse reviewer".into() }
}
struct Rig { dir: tempfile::TempDir }
impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("models_cache.json"), json!({"client_version":"0.166.0","models":[{
            "slug":"exact-model", "shell_type":"local", "apply_patch_tool_type":"freeform"
        }]}).to_string()).unwrap();
        Self { dir }
    }
    fn log(&self) -> PathBuf { self.dir.path().join("requests.log") }
    fn pid(&self) -> PathBuf { self.dir.path().join("child.pid") }
    fn provider(&self, mode: &str) -> CodexProvider {
        CodexProvider::new(FAKE).with_env("CODEX_HOME", self.dir.path()).with_env("FAKE_CODEX_MODE", mode)
            .with_env("FAKE_CODEX_LOG", self.log()).with_env("FAKE_CODEX_FIXTURE", FIXTURE)
            .with_env("FAKE_CODEX_CHILD_PID", self.pid())
            .with_env("FAKE_CODEX_CONFIG", r#"{"mcp_servers":{"inherited":{"command":"must-not-run"}},"features":{"hooks":true}}"#)
    }
    fn messages(&self) -> Vec<Value> {
        std::fs::read_to_string(self.log()).unwrap().lines().filter_map(|line| line.strip_prefix("STDIN "))
            .map(|line| serde_json::from_str(line).unwrap()).collect()
    }
    fn assert_directories_removed(&self) {
        for cwd in std::fs::read_to_string(self.log()).unwrap().lines().filter_map(|line| line.strip_prefix("CWD ")) {
            assert!(!std::path::Path::new(cwd).exists(), "temporary directory remains: {cwd}");
        }
    }
}

#[tokio::test]
async fn codex_text_uses_isolated_draft_parameters_and_declines_tool_requests() {
    let rig = Rig::new();
    assert_eq!(rig.provider("unsupported").generate_text(request()).await.unwrap(), "You are direct. You check assumptions.");
    let messages = rig.messages();
    let threads: Vec<_> = messages.iter().filter(|m| m["method"] == "thread/start").collect();
    assert_eq!(threads.len(), 1);
    let params = &threads[0]["params"];
    assert_eq!(params["ephemeral"], true);
    assert_eq!(params["approvalPolicy"], "never");
    assert_eq!(params["sandbox"], "read-only");
    assert_eq!(params["model"], "exact-model");
    assert_eq!(params["developerInstructions"], "Personality only");
    assert_eq!(params["allowProviderModelFallback"], false);
    let config = &params["config"];
    assert_eq!(config["mcp_servers"], json!({"inherited":{"enabled":false}}));
    assert_eq!(config["features"]["shell_tool"], false);
    assert_eq!(config["features"]["hooks"], false);
    assert_eq!(config["memories"]["use_memories"], false);
    assert_eq!(config["project_doc_max_bytes"], 0);
    let turn = messages.iter().find(|m| m["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["effort"], "high");
    assert_eq!(turn["params"]["input"][0]["text"], "A terse reviewer");
    assert!(messages.iter().any(|m| m["id"] == 77 && m.get("error").is_some()));
    assert!(!messages.iter().any(|m| m["method"] == "thread/resume"));
    rig.assert_directories_removed();
}

#[tokio::test]
async fn codex_text_fails_before_prompt_delivery_if_controls_are_overridden() {
    let rig = Rig::new();
    assert!(rig.provider("control_override").generate_text(request()).await.is_err());
    assert!(!rig.messages().iter().any(|m| m["method"] == "thread/start" || m["method"] == "turn/start"));
    rig.assert_directories_removed();
}

#[tokio::test]
async fn failed_codex_turn_is_a_protocol_error() {
    let rig = Rig::new();
    let fixture = rig.dir.path().join("failed.jsonl");
    std::fs::write(&fixture, "{\"method\":\"turn/completed\",\"params\":{\"turn\":{\"status\":\"failed\"}}}\n").unwrap();
    assert!(matches!(rig.provider("approval").with_env("FAKE_CODEX_FIXTURE", fixture).generate_text(request()).await, Err(ProviderError::Protocol(_))));
    rig.assert_directories_removed();
}

#[tokio::test]
async fn dropping_codex_text_kills_descendants_and_removes_temporary_files() {
    let rig = Rig::new();
    let provider = rig.provider("hang");
    let task = tokio::spawn(async move { provider.generate_text(request()).await });
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(rig.pid()) && let Ok(pid) = text.trim().parse::<i32>() { break pid; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("child must start");
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("descendant must exit");
    rig.assert_directories_removed();
}

#[tokio::test]
async fn codex_text_accepts_newer_versions_and_refreshes_the_catalog() {
    let rig = Rig::new();
    let mut cache: Value = serde_json::from_slice(&std::fs::read(rig.dir.path().join("models_cache.json")).unwrap()).unwrap();
    cache["client_version"] = json!("0.167.0");
    let refreshed = rig.dir.path().join("refreshed.json");
    std::fs::write(&refreshed, cache.to_string()).unwrap();
    let text = rig.provider("approval").with_env("FAKE_CODEX_VERSION", "codex-cli 0.167.0")
        .with_env("FAKE_CODEX_REFRESH_CACHE", &refreshed).generate_text(request()).await.unwrap();
    assert_eq!(text, "You are direct. You check assumptions.");
    assert_eq!(rig.messages().iter().filter(|v| v["method"] == "model/list").count(), 1);
}

#[tokio::test]
async fn codex_text_rejects_old_or_unreadable_versions() {
    for (version, expected) in [("codex-cli 0.165.0", "Codex 0.165.0 is older than the tested version 0.166.0. Update Codex, then retry."), ("unreadable", "version could not be read")] {
        let rig = Rig::new();
        let error = rig.provider("approval").with_env("FAKE_CODEX_VERSION", version).generate_text(request()).await.unwrap_err();
        assert!(matches!(error, ProviderError::Setup(_)));
        assert!(error.to_string().contains(expected), "{error}");
        assert!(!rig.log().exists());
    }
}

#[tokio::test]
async fn newer_codex_text_still_rejects_overridden_controls() {
    let rig = Rig::new();
    let path = rig.dir.path().join("models_cache.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    cache["client_version"] = json!("0.167.0");
    std::fs::write(path, cache.to_string()).unwrap();
    let error = rig.provider("control_override").with_env("FAKE_CODEX_VERSION", "codex-cli 0.167.0").generate_text(request()).await.unwrap_err();
    assert!(error.to_string().contains("0.167.0") && error.to_string().contains("0.166.0"), "{error}");
    assert!(!rig.messages().iter().any(|v| v["method"] == "thread/start" || v["method"] == "turn/start"));
}

#[tokio::test]
async fn unreachable_claude_api_blocks_text_before_cli_start() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let provider = discard_providers::claude::ClaudeProvider::new("/nonexistent/claude")
        .with_env("ANTHROPIC_BASE_URL", format!("http://user:secret@127.0.0.1:{port}/private"));
    let start = std::time::Instant::now();
    let error = provider.generate_text(request()).await.unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(matches!(error, ProviderError::Setup(_)));
    assert!(error.to_string().contains(&format!("http://127.0.0.1:{port} (ANTHROPIC_BASE_URL)")));
    assert!(!error.to_string().contains("secret") && !error.to_string().contains("private"));
}
