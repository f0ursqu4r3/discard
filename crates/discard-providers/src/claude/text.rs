//! One non-interactive Claude request with no session or tool access.
use discard_core::provider::{ProviderError, TextRequest};
use serde_json::Value;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, process::Command};
use super::api::{api_address, check_api_address};
use crate::text::{TextProcess, TIMEOUT, protocol, timed_out};

pub(super) fn build_args(request: &TextRequest) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p", "--output-format", "json", "--no-session-persistence", "--tools", "",
        "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}", "--disable-slash-commands",
        "--setting-sources", "", "--restricted", "--settings",
        "{\"disableAllHooks\":true,\"autoMemoryEnabled\":false,\"env\":{\"CLAUDE_CODE_DISABLE_AUTO_MEMORY\":\"1\",\"CLAUDE_CODE_DISABLE_CLAUDE_MDS\":\"1\",\"CLAUDE_CODE_PLUGIN_DIRS\":\"\"}}",
    ].map(String::from).into();
    args.extend(["--model".into(), request.model.clone(), "--system-prompt".into(), request.instructions.clone()]);
    if let Some(effort) = &request.reasoning_effort { args.extend(["--effort".into(), effort.clone()]); }
    args
}

fn result_text(output: &[u8]) -> Result<String, ProviderError> {
    let value: Value = serde_json::from_slice(output).map_err(|_| protocol("Claude returned invalid text output."))?;
    if value["type"] != "result" || value["is_error"] == true || value["subtype"] != "success" {
        return Err(protocol("Claude could not generate text. Try again."));
    }
    value["result"].as_str().map(str::to_owned).ok_or_else(|| protocol("Claude returned no text result."))
}

/// The CLI retries an unreachable API address without output, so name the address.
fn explain_timeout(error: ProviderError, address: Option<&str>) -> ProviderError {
    match (error, address) {
        (ProviderError::Protocol(message), Some(address)) if message == "The model did not answer in time." => {
            protocol(&format!("{message} Claude uses the API address {address} (ANTHROPIC_BASE_URL). Make sure that address is reachable."))
        }
        (error, _) => error,
    }
}

pub(super) async fn generate(mut command: Command, request: TextRequest) -> Result<String, ProviderError> {
    check_api_address(&command).await?;
    crate::conversation::check_claude_managed_files(false)?;
    let files = crate::conversation::ConversationFiles::new()?;
    command.args(build_args(&request)).env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1").env_remove("CLAUDE_CODE_PLUGIN_DIRS");
    let address = api_address(&command);
    let mut process = TextProcess::spawn(command, &files.cwd)?;
    let result = tokio::time::timeout(TIMEOUT, async {
        let mut stdin = process.child.stdin.take().expect("piped stdin");
        let mut stdout = process.child.stdout.take().expect("piped stdout");
        // Drain output while writing stdin, so a large current personality cannot fill both pipes.
        let write = async move {
            stdin.write_all(request.prompt.as_bytes()).await?;
            stdin.shutdown().await
        };
        let read = async {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        };
        let (_, output) = tokio::try_join!(write, read).map_err(|_| protocol("Claude closed before returning text."))?;
        let status = process.child.wait().await.map_err(|e| ProviderError::Io(e.to_string()))?;
        if !status.success() { return Err(protocol("Claude could not generate text. Try again.")); }
        result_text(&output)
    }).await.unwrap_or_else(|_| Err(timed_out()));
    process.finish(result).await.map_err(|error| explain_timeout(error, address.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> TextRequest { TextRequest { model: "exact-model".into(), reasoning_effort: None, instructions: "Replace the default instructions".into(), prompt: "private description".into() } }
    #[test]
    fn arguments_isolate_text_and_pass_exact_model_instructions_and_optional_effort() {
        let mut request = request();
        let args = build_args(&request);
        let value = |flag: &str| args.windows(2).find(|pair| pair[0] == flag).unwrap()[1].clone();
        assert_eq!(value("--model"), "exact-model");
        assert_eq!(value("--system-prompt"), request.instructions);
        assert_eq!(value("--tools"), "");
        assert_eq!(value("--mcp-config"), "{\"mcpServers\":{}}");
        assert_eq!(value("--setting-sources"), "");
        for flag in ["-p", "--strict-mcp-config", "--no-session-persistence", "--disable-slash-commands"] { assert!(args.iter().any(|a| a == flag)); }
        assert!(!args.contains(&"--effort".into()));
        assert!(!args.contains(&request.prompt));
        assert!(!args.contains(&"--append-system-prompt".into()));
        let settings: Value = serde_json::from_str(&value("--settings")).unwrap();
        assert_eq!(settings["disableAllHooks"], true); assert_eq!(settings["autoMemoryEnabled"], false);
        request.reasoning_effort = Some("high".into());
        assert!(build_args(&request).windows(2).any(|p| p == ["--effort", "high"]));
    }
    #[test]
    fn a_timeout_names_the_custom_api_address_without_path_or_user_information() {
        let mut command = Command::new("claude");
        command.env("ANTHROPIC_BASE_URL", "http://user:secret@127.0.0.1:8787/private/path");
        assert_eq!(api_address(&command).as_deref(), Some("http://127.0.0.1:8787"));
        let error = explain_timeout(crate::text::timed_out(), api_address(&command).as_deref()).to_string();
        assert!(error.contains("The model did not answer in time."));
        assert!(error.contains("http://127.0.0.1:8787 (ANTHROPIC_BASE_URL)"));
        assert!(!error.contains("secret") && !error.contains("private"));
        command.env_remove("ANTHROPIC_BASE_URL");
        assert_eq!(api_address(&command), None);
        assert!(matches!(explain_timeout(crate::text::timed_out(), None), ProviderError::Protocol(m) if m == "The model did not answer in time."));
        assert!(matches!(explain_timeout(protocol("other"), Some("http://x")), ProviderError::Protocol(m) if m == "other"));
        command.env("ANTHROPIC_BASE_URL", "not a url");
        assert_eq!(api_address(&command), None);
    }
    #[test]
    fn final_result_rejects_errors_and_malformed_output() {
        assert_eq!(result_text(br#"{"type":"result","subtype":"success","is_error":false,"result":"You are concise."}"#).unwrap(), "You are concise.");
        for output in [r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#, r#"{"type":"result","subtype":"success"}"#, "invalid"] { assert!(result_text(output.as_bytes()).is_err()); }
    }
    #[tokio::test]
    async fn prompt_uses_stdin_in_an_empty_directory_without_memory_environment() {
        let mut command = Command::new("sh");
        command.args(["-c", r#"
            test "$CLAUDE_CODE_DISABLE_AUTO_MEMORY" = 1 || exit 1
            test "$CLAUDE_CODE_DISABLE_CLAUDE_MDS" = 1 || exit 1
            test -z "$CLAUDE_CODE_PLUGIN_DIRS" || exit 1
            test -z "$(ls -A)" || exit 1
            input=$(cat)
            test "$input" = 'private description' || exit 1
            echo '{"type":"result","subtype":"success","result":"You are concise."}'
        "#]);
        assert_eq!(generate(command, request()).await.unwrap(), "You are concise.");
    }
    #[tokio::test]
    async fn nonzero_exit_reports_a_redacted_stderr_tail() {
        let mut command = Command::new("sh");
        command.args(["-c", "cat >/dev/null; echo 'generation failed' >&2; exit 2"]);
        let error = generate(command, request()).await.unwrap_err();
        assert!(matches!(&error, ProviderError::Protocol(_)));
        assert!(error.to_string().contains("generation failed"));
    }
}
