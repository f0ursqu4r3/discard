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

The only enabled MCP server is scoped Discard memory. It lists memory tools and the applicable project-file and worktree tools. Project channels can list, read, and search files through the scoped project tools. Codex also exposes three generic MCP resource helpers: `list_mcp_resources`, `list_mcp_resource_templates` and `read_mcp_resource`. These helpers use the same scoped server; they do not grant filesystem access. The empty working directory prevents project instruction discovery. Trusted provider-wide instructions, including global `AGENTS.md`, and administrator policy can still apply. This mode does not isolate the provider from its administrator.

The scoped memory server sets `default_tools_approval_mode="approve"` and uses an explicit `enabled_tools` allowlist: six memory tools, two worktree tools, and three project-file tools. The server further limits the listed tools by conversation scope. Codex's default `auto` mode can require a prompt even for these tools, which `approvalPolicy:"never"` cannot answer. The scoped grant keeps never approval, the read-only sandbox and all native-tool restrictions. Discard's memory service still controls scope access and shared-write approval; a shared save remains a pending proposal. Coding sessions keep their existing approval behavior.

The supported version is pinned because tool registration and catalog fields can change. The opt-in synthetic probe at `crates/discard-providers/tests/probes/codex_conversation.py` checks effective controls and the actual model-facing tool allowlist against a local test endpoint. It also executes a synthetic private save, read and shared proposal, and verifies tool-call receipt, result delivery and a pending shared proposal with no shared write. It uses a temporary provider home and no real credentials. Ordinary process fixtures verify adapter behavior; they do not prove provider isolation by themselves.

Run the local probe with `python3 crates/discard-providers/tests/probes/codex_conversation.py`. The script works from any working directory when you give its full path. Use `--codex /path/to/codex` or `DISCARD_CODEX_BIN` to select the CLI executable. Use `--codex-home /path/to/provider-home` to select the source model cache; the default uses `CODEX_HOME` or `~/.codex`. The probe fails before thread creation if a required effective control differs.

Normal app conversations replay the current conversation transcript with refreshed personality and applicable saved memory before every response. Memory edits apply to existing conversations without restarting. Native resume behavior described above applies to Coding sessions. See [Saved memory](memory.md) for the four domains, shared-write approval, Forget, and capture controls.

## Workspace sessions (spike results)

Tested codex-cli 0.166.0 against a synthetic local Responses endpoint with a temporary home.
Command for every row: `python3 crates/discard-providers/tests/probes/codex_workspace.py`.
The available model cache was written by 0.167.0. The probe uses only its model metadata;
it does not copy credentials or change the source cache. Production still requires a 0.166.0 cache.

| Question | Observed value | Probe command |
| --- | --- | --- |
| Native tools | Cached unified_exec shell exposes `functions.exec_command`, `functions.write_stdin`, and `functions.apply_patch`; `features.shell_tool=true` works with `unified_exec=false`. | Command above |
| In-root apply_patch | Sends `item/fileChange/requestApproval`; params contain itemId and grantRoot, but no changes. Paths come from the preceding fileChange item. | Command above |
| Shell requests | `item/commandExecution/requestApproval`, `kind=command`, command, cwd, commandActions. | Command above |
| Decisions | Commands offer accept, acceptWithExecpolicyAmendment, cancel. File-change requests omit availableDecisions. | Command above |
| Escalation marker | No reliable distinct marker. Classify every Workspace command approval as SandboxEscape. | Command above |
| Writable roots | Effective `sandbox_workspace_write.writable_roots=[]`, network_access=false; temp directories remain writable by default. | Command above |
| Instruction discovery | `project_doc_max_bytes=0`; root AGENTS.md sentinel absent from model requests. | Command above |
| Approved writes | Approving all requests permits outside patch, temporary-HOME touch, and git add. Ordinary approval must not be described as proof that these operations stay sandboxed. | Command above |
| Network | curl exits 6 with network_access=false. | Command above |

The recorded transcript is `fixtures/codex-workspace-turn.jsonl`. Paths are scrubbed.
The probe uses the observed exec_command schema and adds a three-second curl limit.

## Workspace sessions

Workspace starts a fresh ephemeral thread in the granted directory. It uses `untrusted`
approval policy and `workspace-write` sandbox, with network access off and no extra writable
roots. It preserves the exact cached shell and patch capabilities and disables the other
Conversation features. The scoped MCP allowlist adds worktree_commit. Hooks, provider memory,
plugins, other MCP servers and project instruction discovery stay off. `GIT_OPTIONAL_LOCKS=0`
permits read-only Git inspection. System temporary directories remain writable.

The adapter checks effective controls before prompt delivery. Missing controls, mismatches,
invalid directories and non-host approval modes return SandboxUnavailable for read-only
fallback. The model cache and running CLI remain pinned to 0.166.0. Workspace uses the same bounded model-list cache recovery as Conversation mode. A stale cache refresh keeps the selected model’s shell and patch capabilities. Ordinary setup, spawn, and protocol failures retain their error type; only failed Workspace controls trigger read-only fallback.

All Workspace command approvals use the full-access SandboxEscape card because the spike
found no reliable escalation marker. File-change paths come from preceding item notifications;
unknown paths never receive automatic edit approval. Coding approvals keep their existing behavior.

Supplemental synthetic review probes found that a file move supplies its destination in `changes[].kind.move_path`. The source can be inside the root while the destination is outside it, with `grantRoot: null`. Discard checks both paths. Incomplete path records cannot receive automatic approval. Outside or protected targets use full-access cards, and the normalized paths appear in the approval input.

### Slice B spike results

The synthetic main-checkout probe uses a normal temporary repository, a temporary
provider home, and a local model endpoint. Run:
`python3 crates/discard-providers/tests/probes/codex_workspace.py --main-checkout`.
All approval requests are accepted to measure provider protection itself.

Tested codex-cli 0.166.0. `codex_in_sandbox_marker = none`.
Three in-sandbox requests and three explicit escalation requests differ in command
text, item IDs, timestamps, suggested command rules, and reason. The reason is null
for in-sandbox requests and model-supplied for escalation. None is a reliable,
non-absent sandbox marker. Workspace command approvals keep `SandboxEscape`.

`codex_protects_git_in_root = false`: `touch .git/marker`, `git add -A`,
`git commit -m x`, and native `apply_patch` to `.git/config` all succeeded after
approval. Main-checkout Workspace setup must fail closed.


Slice B adds `worktree_request_creation` to scoped Conversation tools. The MCP
server checks the agent and channel before listing or running it. App approval
starts a fresh Continue response. Under decision 20e, Codex does not support
main-checkout writes. Its MCP identity hides `main_checkout_request_write` and
refuses direct calls. Its prompt explains the limit instead of offering a request
that cannot succeed. The engine also refuses main-checkout grants for unsupported
providers. The adapter still rejects Workspace roots containing a `.git`
directory. Linked-worktree writes keep their existing protection checks. Coding
behavior stays unchanged.

The marker result remains `none`. Every Workspace command approval therefore
remains SandboxEscape. Its card says "Always allow is unavailable for Codex write
sessions." Discard's durable rule system does not approve these requests.
