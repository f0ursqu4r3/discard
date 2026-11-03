# Codex provider notes

Findings from the M4 spike. CLI version `0.166.0`. The capability matrix is
`crates/discard-providers/fixtures/codex.capabilities.json` (matrix version 2, verified live).

## How Discard talks to Codex

Discard starts `codex app-server --listen stdio://` as a child process in its own process group. The protocol
is JSON-RPC over stdio (one JSON message per line). Discard uses the user's existing ChatGPT login and never
reads or stores credentials. `codex login status` exit code 0 means logged in.

Handshake, in order:

1. `initialize` with `clientInfo` (`name: "discard"`).
2. `initialized` notification.
3. `thread/start`, or `thread/resume` with the saved thread ID. The response holds `thread.id`.
4. `turn/start` with `threadId` and `input: [{"type": "text", "text": ...}]`.

`codex app-server generate-json-schema --out DIR` writes the full protocol schema. Discard does not depend on
the generated types. It reads the few fields it needs.

## Capability findings

| Capability | Result | How |
| --- | --- | --- |
| Interactive approvals | Yes | With `approvalPolicy: "untrusted"`, Codex sends the server request `item/commandExecution/requestApproval` (or `item/fileChange/requestApproval`) and waits. Discard answers with `{"decision": ...}`. |
| Block a tool call before it runs | Yes | The request arrives before the command runs. A `cancel` answer stopped it. Verified live: a denied `touch` did not run. |
| Tool-call visibility | Yes | `item/started` and `item/completed` notifications for `commandExecution`, `fileChange`, and `mcpToolCall`. |
| Per-session MCP | Yes | The `config` object of `thread/start` takes `mcp_servers.<name>.url` and `http_headers`. Verified live with Discard's memory server. |
| Resume | Yes | `thread/resume` with `threadId`. Threads must be started with `ephemeral: false` to be resumable later. |
| Cancel | Yes | Discard sends SIGTERM to the process group, waits 2 seconds, then SIGKILL. |

## Things that surprised us

1. **The server lists the decisions it accepts** (`availableDecisions`). A command approval offered only
   `accept`, `acceptWithExecpolicyAmendment`, and `cancel`. There was no `decline`. Discard maps a denial to
   `decline` if offered, otherwise to `cancel`. `cancel` also interrupts the turn. "Always allow" maps to
   `acceptForSession` only if offered, otherwise to a plain `accept`.
2. **Denial interrupts the turn on some requests.** After a `cancel`, Codex ends the turn instead of letting the
   agent try something else.
3. **Server requests Discard cannot answer.** `item/permissions/requestApproval`, `item/tool/requestUserInput`,
   `item/tool/call`, and the legacy approval methods get a JSON-RPC error reply and a visible error event, so
   Codex never waits for an answer that will not come. This is fail-closed. Permission escalation is not
   supported yet.
4. **MCP tool approvals arrive as elicitations** (`mcpServer/elicitation/request`). Discard shows them as
   approvals named `mcp__<server>__<tool>`, so the memory tools are approved by the same policy as in Claude.
5. **The `config` override needs the dotted-key structure as nested JSON** (`{"mcp_servers": {...}}`).
6. **Startup is noisy.** The server reports plugin and MCP server startup status and warnings about the user's
   own config. Discard ignores those notifications.
7. **Sandbox choice.** `HostPrompts` uses `approvalPolicy: "untrusted"` with `sandbox: "workspace-write"`.
   `ProviderDenies` uses `approvalPolicy: "never"` with `sandbox: "read-only"`.

## Event mapping

| Codex message | Normalized event |
| --- | --- |
| `turn/started` | `StatusChange(Working)` |
| `item/agentMessage/delta` | `TextDelta` |
| `item/started` (command, file change, MCP call) | `ToolCallStart` |
| `*/requestApproval`, `mcpServer/elicitation/request` | `ApprovalRequest` |
| `item/completed` (same item types) | `ToolCallResult` |
| `thread/tokenUsage/updated` | kept, reported at the end of the turn |
| `turn/completed` | `Usage`, then `Error` if the turn failed, then `StatusChange(Idle)` |
| `error` with `willRetry: false` | `Error` |
| everything else | dropped |

## Tests

- `src/codex/parse.rs`: mapper tests, including the scrubbed transcript `fixtures/codex-approval-turn.jsonl`.
- `tests/codex_process.rs`: real subprocess tests against `fixtures/fake-codex.py`.
- `tests/codex_live.rs`: `#[ignore]` tests against the real CLI. Run: `cargo test -p discard-providers --test codex_live -- --ignored`.
- `tests/memory_live.rs`: `#[ignore]` tests that save and recall memory through the real MCP server with Claude and Codex.

## Choosing a provider

Run `discard --provider codex` (or set `DISCARD_PROVIDER`). Each project gets one channel per provider, with
its own agent and session.
