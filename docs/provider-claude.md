# Claude provider notes

Findings from the M1 spike. CLI version `2.1.317`. The recorded capability matrix is
`crates/discard-providers/fixtures/claude.capabilities.json`.

## How Discard talks to Claude

Discard starts the official `claude` CLI as a child process, in its own process group:

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --permission-mode manual \
  --permission-prompts host --permission-prompt-tool stdio
```

Discard uses the user's existing login. It never reads or stores credentials.

## Capability findings

| Capability | Result | How |
| --- | --- | --- |
| Interactive approvals | Yes | Needs `--permission-prompt-tool stdio`. The CLI then sends a `control_request` with `subtype: can_use_tool`. Discard answers with a `control_response`. |
| Tool-call visibility | Yes | `assistant` messages carry `tool_use` blocks. `user` messages carry `tool_result` blocks. |
| Block a tool call before it runs | Yes | A `deny` response stops the call. Verified against the real CLI: a denied `touch` did not run. |
| Per-session MCP config | Yes (not used before M3) | `--mcp-config <file>`. |
| Resume | Yes | `--resume <session_id>`. The ID comes from the `system/init` event. |
| Cancel | Yes | Discard sends SIGTERM to the process group, waits 2 seconds, then sends SIGKILL. |

## Things that surprised us

1. **Without `--permission-prompt-tool stdio`, approvals do not reach the host.** `--permission-prompts host`
   alone makes Claude deny the tool call with a tool result that says it needs approval. No
   `control_request` is sent. This flag is missing from `claude --help`.
2. **Read-only shell commands skip approval.** `echo` runs without a prompt. A command that writes
   (`touch`) triggers one. Claude's own permission rules, including the user's allow rules, still apply
   on top of Discard's policy. Discard cannot ask about calls that Claude allows by itself.
3. **"Always allow" is scoped to the session.** Claude suggests rules with a `localSettings`
   destination, which writes to the project's settings file. Discard rewrites the destination to
   `session`.
4. **The CLI exits without a session when `--resume` is rejected.** It prints a message on stderr and sends
   no `init` event. Discard turns the redacted stderr tail into an `Error` event.
5. **A `result` event with `is_error: true`** becomes an `Error` event followed by `Idle`. The turn
   driver marks such a turn as failed.

## Event mapping

| Claude line | Normalized event |
| --- | --- |
| `system/init` | session ID (resume handle) + `StatusChange(Working)` |
| `stream_event` / `text_delta` | `TextDelta` |
| `assistant` with `tool_use` (once per tool ID) | `ToolCallStart` |
| `control_request` / `can_use_tool` | `ApprovalRequest` |
| `user` with `tool_result` | `ToolCallResult` |
| `result` | `Usage`, then `Error` if `is_error`, then `StatusChange(Idle)` |
| everything else (thinking, hooks, rate limits) | dropped |

Every emitted event keeps the original JSON line in `raw`.

## Tests

- `src/claude/parse.rs`: parser tests, including the recorded transcript `fixtures/claude-approval-turn.jsonl`.
- `tests/claude_process.rs`: real subprocess tests against `fixtures/fake-claude.sh`.
- `tests/claude_live.rs`: `#[ignore]` tests against the real CLI. They use one small Haiku turn each.
  Run: `cargo test -p discard-providers --test claude_live -- --ignored`.

## Codex

See `docs/provider-codex.md`. The Codex matrix was verified against a live `codex app-server` in M4.

## Conversation sessions

`SessionMode::Conversation` requires Claude Code 2.1.318 and the documented conversation controls. It uses a fresh, empty Discard-owned working directory and rejects native resume handles. Discard supplies the selected model and refreshed system instructions for a new transcript replay.

The adapter passes `--tools ""`, `--strict-mcp-config`, `--disable-slash-commands`, `--restricted` and `--system-prompt-snapshot off`. It accepts only the scoped Discard memory MCP configuration. It forces `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` and `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` in both process environment and explicit session settings. It disables hooks and removes inherited explicit plugin directories. Authentication uses the existing CLI login; Discard does not use `--bare`, copy credentials or change user settings.

The adapter checks the CLI version and required help controls before sending instructions. It rejects detected local managed settings that conflict with memory, hooks or plugin restrictions. Managed policy remains authoritative. Claude can receive remote or MDM policy that the CLI does not expose through an effective-settings API. Discard cannot independently verify that opaque policy, and does not claim administrator or operating-system isolation.

The opt-in synthetic probe at `crates/discard-providers/tests/probes/claude_conversation.py` checks the actual model-facing tools against local test endpoints. The tested version exposed only the six Discard memory tools. The probe uses a temporary configuration directory and a synthetic API key. Process-fixture tests cover flags, environment overrides, fresh instructions and cleanup.

Run the local probe with `python3 crates/discard-providers/tests/probes/claude_conversation.py`. The script works from any working directory when you give its full path. Use `--claude /path/to/claude` or `DISCARD_CLAUDE_BIN` to select the CLI executable.
