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

## Codex (best effort, not verified)

`crates/discard-providers/fixtures/codex.capabilities.json` is built from `--help` output only.
`codex exec --json` prints JSONL events and supports `exec resume`, but it has no interactive approval
channel. `codex app-server` is the candidate for interactive approvals, and it can generate a JSON schema
of its protocol (`codex app-server generate-json-schema`). The matrix marks approvals, per-session MCP,
and pre-execution blocking as `false` so that Discard fails closed. The M4 spike must verify these.
