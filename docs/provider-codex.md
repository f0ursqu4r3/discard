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

## Conversation sessions

`SessionMode::Conversation` uses a fresh, empty Discard-owned working directory. It starts a new ephemeral thread for each replay. It rejects native resume handles. The selected model must have an exact slug in the installed Codex 0.166.0 model cache. Discard resolves that cache from the provider's `CODEX_HOME`, or `HOME/.codex`. If the cache is missing, open Codex and refresh its model list before retrying. Discard does not change the user's cache, configuration, or authentication.

Discard copies complete model records into a temporary startup catalog. It changes only tool capability fields: patch, shell, experimental tools, delegation, node REPL and tool mode. The model slug and context-window metadata stay intact. `allowProviderModelFallback` is false. `Provider::model_context_window` returns the exact model's published limit when available.

Before sending memory instructions, Discard reads effective configuration without starting a thread. It then disables every inherited named MCP server at startup and checks the final effective controls. It disables plugins, apps, native web, image generation, shell, code mode, delegation, skills, automatic memory, history-note tools, hooks and discovery. An inherited server named `discard-memory` produces an error because its settings could merge into the scoped server. A missing or overridden required control produces an error before prompt delivery.

The only enabled MCP server is scoped Discard memory. The six memory tools appear directly. Codex also exposes three generic MCP resource helpers: `list_mcp_resources`, `list_mcp_resource_templates` and `read_mcp_resource`. These helpers use the same scoped server; they do not grant filesystem access. The empty working directory prevents project instruction discovery. Trusted provider-wide instructions, including global `AGENTS.md`, and administrator policy can still apply. This mode does not isolate the provider from its administrator.

The scoped memory server sets `default_tools_approval_mode="approve"` and retains its six-tool `enabled_tools` allowlist. Codex's default `auto` mode can require a prompt even for these tools, which `approvalPolicy:"never"` cannot answer. The scoped grant allows memory calls while keeping never approval, the read-only sandbox and all native-tool restrictions. Discard's memory service still controls scope access and shared-write approval; a shared save remains a pending proposal. Coding sessions keep their existing approval behavior.

The supported version is pinned because tool registration and catalog fields can change. The opt-in synthetic probe at `crates/discard-providers/tests/probes/codex_conversation.py` checks effective controls and the actual model-facing tool allowlist against a local test endpoint. It also executes a synthetic private save, read and shared proposal, and verifies tool-call receipt, result delivery and a pending shared proposal with no shared write. It uses a temporary provider home and no real credentials. Ordinary process fixtures verify adapter behavior; they do not prove provider isolation by themselves.

Run the local probe with `python3 crates/discard-providers/tests/probes/codex_conversation.py`. The script works from any working directory when you give its full path. Use `--codex /path/to/codex` or `DISCARD_CODEX_BIN` to select the CLI executable. Use `--codex-home /path/to/provider-home` to select the source model cache; the default uses `CODEX_HOME` or `~/.codex`. The probe fails before thread creation if a required effective control differs.

Normal app conversations replay the current conversation transcript with refreshed personality and applicable saved memory before every response. Memory edits apply to existing conversations without restarting. Native resume behavior described above applies to Coding sessions. See [Saved memory](memory.md) for the four domains, shared-write approval, Forget, and capture controls.
