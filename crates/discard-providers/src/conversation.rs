//! Conversation boundaries shared by CLI adapters. No credentials are copied.

use discard_core::provider::{ProviderError, SessionSpec};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) const WORKSPACE_TOOLS: &[&str] = &["worktree_commit"];

pub(crate) const MEMORY_TOOLS: &[&str] = &["memory_search", "memory_list", "memory_read", "memory_save", "memory_edit", "memory_forget", "worktree_inspect", "worktree_request_removal", "worktree_request_creation", "main_checkout_request_write", "project_list_files", "project_read_file", "project_search"];

pub(crate) fn setup(message: impl Into<String>) -> ProviderError {
    ProviderError::Setup(format!("conversation mode: {}", message.into()))
}

pub(crate) struct ConversationFiles {
    pub root: tempfile::TempDir,
    pub cwd: PathBuf,
}

impl ConversationFiles {
    pub fn new() -> Result<Self, ProviderError> {
        let root = tempfile::Builder::new().prefix("discard-conversation-").tempdir().map_err(|e| setup(e.to_string()))?;
        let cwd = root.path().join("cwd");
        std::fs::create_dir(&cwd).map_err(|e| setup(e.to_string()))?;
        Ok(Self { root, cwd })
    }
}

pub(crate) fn memory_config(doc: &Value) -> Result<Value, ProviderError> {
    let servers = doc.get("mcpServers").and_then(Value::as_object).ok_or_else(|| setup("missing memory MCP configuration"))?;
    if servers.len() != 1 || !servers.contains_key("discard-memory") {
        return Err(setup("only the Discard memory MCP server is permitted"));
    }
    let server = &servers["discard-memory"];
    let url = server.get("url").and_then(Value::as_str).ok_or_else(|| setup("memory server URL is missing"))?;
    let parsed = url::Url::parse(url).map_err(|_| setup("memory server must use a local HTTP endpoint"))?;
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host == "localhost",
        None => false,
    };
    let has_user_info = !parsed.username().is_empty()
        || parsed.password().is_some()
        || url
            .split_once("://")
            .is_some_and(|(_, rest)| rest.split(['/', '?', '#']).next().is_some_and(|authority| authority.contains('@')));
    if parsed.scheme() != "http" || !loopback || has_user_info {
        return Err(setup("memory server must use a local HTTP endpoint without user information"));
    }
    if server.get("command").is_some() || server.get("args").is_some() {
        return Err(setup("memory MCP must not execute a process"));
    }
    let headers = server.get("headers").and_then(Value::as_object).ok_or_else(|| setup("memory server headers must be an object"))?;
    if headers.values().any(|v| !v.is_string()) {
        return Err(setup("memory server headers must contain strings"));
    }
    Ok(doc.clone())
}

pub(crate) fn read_memory_config(spec: &SessionSpec) -> Result<Value, ProviderError> {
    let path = spec.mcp_config.as_ref().ok_or_else(|| setup("a scoped memory MCP configuration is required"))?;
    let bytes = std::fs::read(path).map_err(|e| setup(format!("could not read memory configuration: {e}")))?;
    let doc = serde_json::from_slice(&bytes).map_err(|e| setup(format!("invalid memory configuration: {e}")))?;
    memory_config(&doc)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogTools { None, Workspace }

pub(crate) fn sanitize_catalog(cache: Value, selected: &str, tools: CatalogTools) -> Result<(Value, Option<u64>), ProviderError> {
    if cache.get("client_version").and_then(Value::as_str) != Some("0.166.0") {
        let actual = cache.get("client_version").and_then(Value::as_str).unwrap_or("unknown");
        return Err(setup(format!("The Codex model cache was written by version {actual}, but Discard requires 0.166.0. Refresh the model list with Codex 0.166.0, then retry.")));
    }
    let mut models = cache
        .get("models")
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty())
        .cloned()
        .ok_or_else(|| setup("Codex model cache is empty or incompatible"))?;
    let selected_model = models
        .iter()
        .find(|m| m.get("slug").and_then(Value::as_str) == Some(selected))
        .ok_or_else(|| setup(format!("the exact selected model `{selected}` is absent from the Codex model cache")))?;
    let window = selected_model.get("context_window").and_then(Value::as_u64);
    for model in &mut models {
        let record = model.as_object_mut().ok_or_else(|| setup("invalid Codex model record"))?;
        if record.get("slug").and_then(Value::as_str).filter(|s| !s.is_empty()).is_none() {
            return Err(setup("Codex model record has no slug"));
        }
        // Preserve every unrelated model field. These are supported ModelInfo capability fields.
        if tools == CatalogTools::None {
            record.insert("apply_patch_tool_type".into(), Value::Null);
            record.insert("shell_type".into(), json!("disabled"));
        }
        record.insert("experimental_supported_tools".into(), json!([]));
        record.insert("multi_agent_version".into(), json!("disabled"));
        record.insert("node_repl_disabled".into(), json!(true));
        record.insert("tool_mode".into(), json!("direct"));
    }
    Ok((json!({"models":models}), window))
}

pub(crate) fn load_catalog(home: &Path, selected: &str, tools: CatalogTools) -> Result<(Value, Option<u64>), ProviderError> {
    let bytes = std::fs::read(home.join("models_cache.json")).map_err(|e| {
        setup(format!("could not read Codex model cache: {e}. Open Codex once and refresh its model list, then retry with the same model"))
    })?;
    let doc = serde_json::from_slice(&bytes).map_err(|e| setup(format!("invalid Codex model cache: {e}")))?;
    sanitize_catalog(doc, selected, tools)
}

pub(crate) fn codex_controls(effective: &Value, tools: CatalogTools) -> Result<Value, ProviderError> {
    let mut controls = json!({
        "web_search":"disabled", "agents":{"enabled":false},
        "memories":{"use_memories":false,"dedicated_tools":false,"generate_memories":false},
        "cloud":{"skills":{"enabled":false}},
        "skills":{"include_instructions":false,"bundled":{"enabled":false}},
        "project_doc_max_bytes":0,
        "tools":{"experimental_request_user_input":{"enabled":false},"update_plan":{"enabled":false}},
        "features":{}, "mcp_servers":{}
    });
    for name in [
        "shell_tool",
        "unified_exec",
        "view_image",
        "multi_agent",
        "multi_agent_v2",
        "plugins",
        "apps",
        "browser_use",
        "computer_use",
        "image_generation",
        "multi_env_copy_file",
        "token_budget",
        "executor_capability_discovery_v2",
        "executor_capability_discovery",
        "agent_message_board",
        "hooks",
        "tool_suggest",
        "remote_plugin",
        "skill_search",
        "code_mode",
        "code_mode_only",
        "code_mode_host",
        "goals",
        "deferred_executor",
    ] {
        controls["features"][name] = json!(false);
    }
    controls["features"]["skip_host_skill_discovery"] = json!(true);
    if tools == CatalogTools::Workspace {
        controls["features"]["shell_tool"] = json!(true);
        controls["sandbox_workspace_write"] = json!({"network_access":false,"writable_roots":[]});
    }
    if let Some(servers) = effective.get("mcp_servers") {
        let servers = servers.as_object().ok_or_else(|| setup("effective Codex mcp_servers is malformed"))?;
        if servers.contains_key("discard-memory") {
            return Err(setup("an inherited discard-memory server conflicts with the scoped session server"));
        }
        for name in servers.keys() {
            controls["mcp_servers"][name] = json!({"enabled":false});
        }
    }
    Ok(controls)
}

/// Public Config.tools drops utility-tool fields. Recover those fields from the
/// ordered raw layers. Managed requirements do not define these two settings.
pub(crate) fn effective_config(response: &Value) -> Result<Value, ProviderError> {
    fn merge(target: &mut Value, source: &Value) {
        if let Some(fields) = source.as_object() {
            if !target.is_object() {
                *target = json!({});
            }
            for (key, value) in fields {
                merge(&mut target[key], value);
            }
        } else {
            *target = source.clone();
        }
    }
    let mut config =
        response.get("config").filter(|v| v.is_object()).cloned().ok_or_else(|| setup("Codex returned no effective configuration"))?;
    let layers = response.get("layers").and_then(Value::as_array).ok_or_else(|| setup("Codex returned no raw configuration layers"))?;
    let mut tools = json!({});
    for layer in layers.iter().rev().filter(|l| l.get("disabledReason").is_none_or(Value::is_null)) {
        if let Some(value) = layer["config"].get("tools") {
            merge(&mut tools, value);
        }
    }
    config["tools"] = tools;
    Ok(config)
}

/// Every required startup override must survive effective configuration and managed policy.
pub(crate) fn validate_controls(effective: &Value, expected: &Value) -> Result<(), ProviderError> {
    fn walk(actual: &Value, expected: &Value, path: String) -> Result<(), ProviderError> {
        if let Some(fields) = expected.as_object() {
            for (key, value) in fields {
                walk(&actual[key], value, format!("{path}.{key}"))?;
            }
        } else if actual != expected {
            return Err(setup(format!("required Codex control `{}` is unavailable or overridden", path.trim_start_matches('.'))));
        }
        Ok(())
    }
    walk(effective, expected, String::new())
}

/// The CLI splits dotted paths literally. Use whole inline tables so server names
/// containing dots cannot change the path or escape their disabled entry.
pub(crate) fn config_args(config: &Value) -> Vec<String> {
    fn toml_value(value: &Value) -> String {
        if let Some(fields) = value.as_object() {
            let fields = fields.iter().map(|(key, value)| format!("{} = {}", json!(key), toml_value(value))).collect::<Vec<_>>();
            format!("{{{}}}", fields.join(", "))
        } else {
            value.to_string()
        }
    }
    let mut args = Vec::new();
    for (key, value) in config.as_object().expect("conversation controls are an object") {
        args.extend(["-c".into(), format!("{key}={}", toml_value(value))]);
    }
    args
}

pub(crate) fn check_claude_managed_files(workspace: bool) -> Result<(), ProviderError> {
    let root = if cfg!(target_os = "macos") { Path::new("/Library/Application Support/ClaudeCode") } else { Path::new("/etc/claude-code") };
    let mut paths = vec![root.join("managed-settings.json")];
    match std::fs::read_dir(root.join("managed-settings.d")) {
        Ok(entries) => {
            for entry in entries {
                let path = entry.map_err(|e| setup(e.to_string()))?.path();
                if path.extension().is_some_and(|s| s == "json") {
                    paths.push(path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(setup("cannot inspect local managed Claude settings")),
    }
    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(setup("cannot inspect local managed Claude settings")),
        };
        let settings: Value = serde_json::from_slice(&bytes).map_err(|_| setup("local managed Claude settings are malformed"))?;
        if workspace && (settings["sandbox"]["enabled"] == false || settings["sandbox"]["allowUnsandboxedCommands"] == true
            || settings["sandbox"]["autoAllowBashIfSandboxed"] == true || settings["sandbox"]["filesystem"].get("allowWrite").is_some()) {
            return Err(ProviderError::SandboxUnavailable("managed Claude settings override workspace sandbox controls".into()));
        }
        for key in ["CLAUDE_CODE_DISABLE_AUTO_MEMORY", "CLAUDE_CODE_DISABLE_CLAUDE_MDS"] {
            if settings["env"].get(key).is_some_and(|v| v.as_str() != Some("1")) {
                return Err(setup(format!("managed Claude setting overrides required {key}")));
            }
        }
        if settings.get("disableAllHooks") == Some(&json!(false))
            || settings["enabledPlugins"].as_object().is_some_and(|m| m.values().any(|v| v == &json!(true)))
        {
            return Err(setup("managed Claude settings enable hooks or plugins"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_catalog_keeps_shell_and_patch() {
        let (out, _) = sanitize_catalog(catalog(), "chosen", CatalogTools::Workspace).unwrap();
        assert_eq!(out["models"][0]["shell_type"], "local");
        assert_eq!(out["models"][0]["apply_patch_tool_type"], "freeform");
        assert_eq!(out["models"][0]["experimental_supported_tools"], json!([]));
        assert_eq!(out["models"][0]["multi_agent_version"], "disabled");
        assert_eq!(out["models"][0]["node_repl_disabled"], true);
    }
    #[test]
    fn workspace_controls_keep_sandbox_and_disable_network() {
        let c = codex_controls(&json!({}), CatalogTools::Workspace).unwrap();
        assert_eq!(c["features"]["shell_tool"], true);
        assert_eq!(c["sandbox_workspace_write"]["network_access"], false);
        assert_eq!(c["sandbox_workspace_write"]["writable_roots"], json!([]));
        assert_eq!(c["project_doc_max_bytes"], 0); assert_eq!(c["web_search"], "disabled");
    }

    fn catalog() -> serde_json::Value {
        json!({"client_version":"0.166.0", "identity":{"secret":"must-not-copy"}, "models":[{
            "slug":"chosen", "display_name":"Chosen", "context_window":123456,
            "apply_patch_tool_type":"freeform", "shell_type":"local", "unrelated":{"preserve":true}
        }]})
    }

    #[test]
    fn catalog_preserves_complete_records_and_exact_model() {
        let (out, window) = sanitize_catalog(catalog(), "chosen", CatalogTools::None).unwrap();
        assert_eq!(window, Some(123456));
        assert_eq!(out["models"][0]["slug"], "chosen");
        assert_eq!(out["models"][0]["unrelated"], json!({"preserve":true}));
        assert_eq!(out["models"][0]["apply_patch_tool_type"], serde_json::Value::Null);
        assert_eq!(out["models"][0]["tool_mode"], "direct");
        assert_eq!(out["models"][0]["node_repl_disabled"], true);
        assert!(out.get("identity").is_none());
    }

    #[test]
    fn missing_exact_model_and_incompatible_catalog_fail() {
        assert!(sanitize_catalog(catalog(), "other", CatalogTools::None).is_err());
        assert!(sanitize_catalog(json!({"models":[]}), "chosen", CatalogTools::None).is_err());
        assert!(sanitize_catalog(json!({"models":[{}]}), "chosen", CatalogTools::None).is_err());
        let mut old = catalog();
        old["client_version"] = json!("0.1.0");
        let error = sanitize_catalog(old, "chosen", CatalogTools::None).unwrap_err().to_string();
        assert!(error.contains("written by version 0.1.0"));
        assert!(error.contains("Refresh the model list with Codex 0.166.0, then retry."));
    }

    #[test]
    fn memory_config_rejects_other_servers_and_bad_headers() {
        assert!(memory_config(&json!({"mcpServers":{"discard-plugins":{"url":"http://127.0.0.1"}}})).is_err());
        assert!(memory_config(&json!({"mcpServers":{"discard-memory":{"url":"http://127.0.0.1", "headers":[]}}})).is_err());
        assert!(
            memory_config(
                &json!({"mcpServers":{"discard-memory":{"url":"http://127.0.0.1/mcp", "headers":{"Authorization":"Bearer synthetic"}}}})
            )
            .is_ok()
        );
    }

    #[test]
    fn memory_config_uses_the_parsed_loopback_host_and_rejects_user_info() {
        let document = |url: &str| json!({"mcpServers":{"discard-memory":{"url":url,"headers":{"Authorization":"Bearer synthetic"}}}});
        for url in [
            "http://127.0.0.1:80@example.com/mcp",
            "http://localhost:80@example.com/mcp",
            "http://user@127.0.0.1/mcp",
            "http://@127.0.0.1/mcp",
            "http://127.0.0.1.evil.example/mcp",
            "https://127.0.0.1/mcp",
            "not a URL",
        ] {
            assert!(memory_config(&document(url)).is_err(), "accepted {url}");
        }
        for url in ["http://127.0.0.1:1234/mcp", "http://localhost:1234/mcp", "http://[::1]:1234/mcp"] {
            assert!(memory_config(&document(url)).is_ok(), "rejected {url}");
        }
    }

    #[test]
    fn inherited_servers_are_disabled_and_overridden_controls_fail() {
        let source = json!({"mcp_servers":{"unrelated":{"command":"do-not-run"}}});
        let config = codex_controls(&source, CatalogTools::None).unwrap();
        assert_eq!(config["mcp_servers"]["unrelated"]["enabled"], false);
        assert!(codex_controls(&json!({"mcp_servers":{"discard-memory":{"command":"inherited"}}}), CatalogTools::None).is_err());
        validate_controls(&config, &config).unwrap();
        let mut bad = config.clone();
        bad["features"]["shell_tool"] = json!(true);
        assert!(validate_controls(&bad, &config).is_err());
        assert!(codex_controls(&json!({"mcp_servers":[]}), CatalogTools::None).is_err());
    }

    #[test]
    fn effective_config_restores_utility_fields_from_ordered_enabled_layers() {
        let response = json!({
            "config":{"tools":{}},
            "layers":[
                {"config":{"tools":{"update_plan":{"enabled":true}}},"disabledReason":"untrusted"},
                {"config":{"tools":{"update_plan":{"enabled":false}}}},
                {"config":{"tools":{"update_plan":{"enabled":true}}}}
            ]
        });
        assert_eq!(effective_config(&response).unwrap()["tools"]["update_plan"]["enabled"], false);
        assert!(effective_config(&json!({"config":{}})).is_err());
    }

    #[test]
    fn cli_overrides_use_literal_top_level_keys_and_safe_inline_server_names() {
        let args = config_args(&json!({"features":{"shell_tool":false},"mcp_servers":{"a.b":{"enabled":false}}}));
        assert_eq!(args, vec!["-c", "features={\"shell_tool\" = false}", "-c", "mcp_servers={\"a.b\" = {\"enabled\" = false}}"]);
    }

    #[test]
    fn app_owned_directory_is_empty_and_removed_on_drop() {
        let dir = ConversationFiles::new().unwrap();
        let root = dir.root.path().to_path_buf();
        assert!(std::fs::read_dir(&dir.cwd).unwrap().next().is_none());
        drop(dir);
        assert!(!root.exists());
    }
}
