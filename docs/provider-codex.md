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

`SessionMode::Conversation` uses a fresh, empty Discard-owned working directory. A session that Discard does not resume starts a new ephemeral thread for each replay and rejects native resume handles. When Discard passes `resumable`, the thread is persistent (`ephemeral: false`), the working directory is the stable directory Discard chose, and a later start may pass the handle to `thread/resume` with the same controls. The process still gets the same `-c` controls and `model_auto_compact_token_limit=1000000000`. A handle Codex rejects, or a resumed thread whose response reports a model other than the selected one, gives `ResumeRejected` so Discard replays instead. The selected model must have an exact slug in a model cache whose `client_version` equals the running CLI version. Discard resolves that cache from the provider's `CODEX_HOME`, or `HOME/.codex`. If the cache is missing, incompatible, or written by another CLI version, Discard asks Codex to refresh its model list once, then validates the cache again. This bounded recovery starts no thread and sends no user prompt. Discard does not edit configuration or authentication; Codex may refresh its own cache.

Discard copies complete model records into a temporary startup catalog. It changes only tool capability fields: patch, shell, experimental tools, delegation, node REPL and tool mode. The model slug and context-window metadata stay intact. `allowProviderModelFallback` is false. `Provider::model_context_window` returns the exact model's published limit when available.

Codex 0.166.0 is the tested version for Conversation, Workspace, and text generation. A newer CLI may run only after all required isolation controls pass verification. Older versions stay blocked with an update message. An unreadable version also stays blocked. Verification failures on a newer CLI name both versions.

After a newer CLI starts a session, Discard adds a conversation marker with the running and tested versions. Each distinct notice appears at most once per engine instance. Notice storage does not block the turn. A newer CLI can change the behavior of a control without removing it; these checks cannot detect every such change.

Before sending memory instructions, Discard reads effective configuration without starting a thread. It then disables every inherited named MCP server at startup and checks the final effective controls. It disables plugins, apps, native web, image generation, shell, code mode, delegation, skills, automatic memory, history-note tools, hooks and discovery. An inherited server named `discard-memory` produces an error because its settings could merge into the scoped server. A missing or overridden required control produces an error before prompt delivery.

The only enabled MCP server is scoped Discard memory. It lists memory tools and the applicable project-file and worktree tools. Project channels can list, read, and search files through the scoped project tools. Codex also exposes three generic MCP resource helpers: `list_mcp_resources`, `list_mcp_resource_templates` and `read_mcp_resource`. These helpers use the same scoped server; they do not grant filesystem access. The empty working directory prevents project instruction discovery. Trusted provider-wide instructions, including global `AGENTS.md`, and administrator policy can still apply. This mode does not isolate the provider from its administrator.

The scoped memory server sets `default_tools_approval_mode="approve"` and uses an explicit `enabled_tools` allowlist: six memory tools, two worktree tools, and three project-file tools. The server further limits the listed tools by conversation scope. Codex's default `auto` mode can require a prompt even for these tools, which `approvalPolicy:"never"` cannot answer. The scoped grant keeps never approval, the read-only sandbox and all native-tool restrictions. Discard's memory service still controls scope access and shared-write approval; a shared save remains a pending proposal. Coding sessions keep their existing approval behavior.

The supported version is pinned because tool registration and catalog fields can change. The opt-in synthetic probe at `crates/discard-providers/tests/probes/codex_conversation.py` checks effective controls and the actual model-facing tool allowlist against a local test endpoint. It also executes a synthetic private save, read and shared proposal, and verifies tool-call receipt, result delivery and a pending shared proposal with no shared write. It uses a temporary provider home and no real credentials. Ordinary process fixtures verify adapter behavior; they do not prove provider isolation by themselves.

Run the local probe with `python3 crates/discard-providers/tests/probes/codex_conversation.py`. The script works from any working directory when you give its full path. Use `--codex /path/to/codex` or `DISCARD_CODEX_BIN` to select the CLI executable. Use `--codex-home /path/to/provider-home` to select the source model cache; the default uses `CODEX_HOME` or `~/.codex`. The probe fails before thread creation if a required effective control differs.

Normal app conversations replay the current conversation transcript with refreshed personality and applicable saved memory before every response. Memory edits apply to existing conversations without restarting. Agents with **Keep provider session** on resume their Codex thread between responses (see [Conversation runtime](runtime/conversation-context.md#native-provider-resume)); other agents replay. See [Saved memory](memory.md) for the four domains, shared-write approval, Forget, and capture controls.

## Native resume spike results

Tested Codex 0.166.0 against a live `codex app-server`, model `gpt-5.6-luna` (and `gpt-5.6-terra` for the model change), with the Conversation controls from `codex_controls` (tools off, every inherited MCP server disabled) passed as `-c` arguments, `ephemeral: false` on `thread/start`, and a stdio MCP server that returned `TOKA` or `TOKB`.

| Question | Result |
| --- | --- |
| Per-session MCP config on `thread/resume` | Re-applies. `config.mcp_servers.tok` with `TOKB` replaced `TOKA`. The config is not stored in the thread: a resume without it had no `tok` tool. Pass it on every resume. |
| `model` on `thread/resume` | Re-applies. The resume response and the model's own answer showed the new model. A resume without `model` kept the last model. |
| `developerInstructions` on `thread/resume` | Does not re-apply. A resume with `BRAVO` kept `ALPHA` from the thread start. The thread keeps the instructions it started with. The adapter has no check for this. The engine's fingerprint holds the hash of the fixed system prompt, so a changed prompt drops the handle and the response replays in a new thread. |
| `approvalPolicy` and `sandbox` on `thread/resume` | Re-apply. The response showed `untrusted` and `workspaceWrite` after a resume that asked for them. |
| Conversation controls (`codex_controls`, tools off) | Process-level `-c` arguments, not thread state. A resume in a process without them could run `echo hi`. A resume in a process with them answered `NO SHELL`. Discard must pass the same `-c` arguments on each spawn, as it does for a fresh start. |
| Global `~/.codex/AGENTS.md` | Still listed in `instructionSources` on resume, as on a fresh start. |

### Model on resume

`thread/resume` params have no `allowProviderModelFallback`. The `ThreadResumeResponse` has a top-level `model`. For a resumable Conversation or Workspace session, the adapter compares it with the selected model and returns `ResumeRejected` on a difference. If the response has no `model`, the adapter cannot check it and resumes. This is deliberate: the startup catalog holds only the selected model's record, and the spike showed the resume `model` param re-applies. Whether Codex can fall back on resume was not probed.

### Handle rejected

`thread/resume` answers with a JSON-RPC error. The process stays alive.

| Case | Error |
| --- | --- |
| Unknown ID (valid UUID) | code `-32600`, message `no rollout found for thread id <id>` |
| Expired ID (rollout file removed) | code `-32600`, message `no rollout found for thread id <id>` |
| Malformed ID | code `-32600`, message `invalid session id: ...` |

Marker for the engine: any JSON-RPC error answer to `thread/resume`. The adapter already maps this case to `ProviderError::ResumeRejected`. The stable text is `no rollout found for thread id`.

### Session store

| Setting | Rollout file |
| --- | --- |
| Default | `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread id>.jsonl`, with metadata in `~/.codex/state_5.sqlite` |
| `CODEX_HOME` changed | `<CODEX_HOME>/sessions/...`. The login is lost (`401 Unauthorized`). |
| `HOME` changed | `<HOME>/.codex/sessions/...`. The login is lost. |

`thread/start` returns the rollout path in `thread.path`. A resume in a process with a different `CODEX_HOME` or `HOME` cannot find the thread. Discard must keep both unchanged between turns of one session.

A resumable thread starts with `ephemeral: false`, so Codex keeps its rollout. The rollout may store the per-thread MCP `http_headers`, which hold the Discard MCP bearer token, under `~/.codex/sessions`. Discard revokes that token at the end of each response, so a stored token no longer works. Discard never deletes the stored rollouts.

### Compaction

Codex compacts a long thread on its own, also on a resumed thread. It has no off switch.

- Flag: `-c model_auto_compact_token_limit=<tokens>` sets the trigger. A limit of 12000 compacted a resumed thread on turn 2 (about 18k tokens). A limit of `1000000000` did not compact over four resumed turns (about 30k tokens). Raise the limit to delay compaction. The behavior near the full model window with a high limit was not probed.
- Stream signal: an `item/started` and an `item/completed` notification with `item.type: "contextCompaction"` inside the turn. No `thread/compacted` notification appeared.

### Live probe result

`cargo test -p discard-providers --test codex_live real_codex_resumes -- --ignored --nocapture` passed on codex-cli 0.166.0 with model `gpt-5.6-luna` (2026-11-18). It ran three Conversation turns for one agent with **Keep provider session** on. A new fact, not an instruction or preference, was saved between turns 2 and 3. Turn 1 started a thread. Turn 2 and turn 3 resumed it with `thread/resume`. The answer in turn 3 used the new fact. The saved `agent_sessions` row kept the same handle after all three turns. No "Fresh replay" notice appeared, so the model check on resume did not reject the thread. A separate raw `thread/resume` call showed that the real 0.166.0 response includes a top-level `model` (`gpt-5.6-luna`). The adapter's model check therefore runs on this CLI version.

## Workspace sessions (spike results)

Tested codex-cli 0.166.0 against a synthetic local Responses endpoint with a temporary home.
Command for every row: `python3 crates/discard-providers/tests/probes/codex_workspace.py`.
The available model cache was written by 0.167.0. The probe uses only its model metadata;
it does not copy credentials or change the source cache. Production requires a cache that matches the running CLI version.

| Question | Observed value | Probe command |
| --- | --- | --- |
| Native tools | Cached unified_exec shell exposes `functions.exec_command`, `functions.write_stdin`, and `functions.apply_patch`; `features.shell_tool=true` works with `unified_exec=false`. | Command above |
| In-root apply_patch | Sends `item/fileChange/requestApproval`; params contain itemId and grantRoot, but no changes. Paths come from the preceding fileChange item. | Command above |
| Shell requests | `item/commandExecution/requestApproval`, `kind=command`, command, cwd, commandActions. | Command above |
| Decisions | Commands offer accept, acceptWithExecpolicyAmendment, cancel. File-change requests omit availableDecisions. | Command above |
| Escalation marker | No reliable distinct marker. Classify every Workspace command approval as SandboxEscape. | Command above |
| Writable roots | Effective `sandbox_workspace_write`: network_access=false, `exclude_slash_tmp=true`, `exclude_tmpdir_env_var=true`. The one writable root is a private temp directory of the session, and `TMPDIR` points to it. `/tmp` and the user temp directory are not writable. Checked with `codex sandbox` and `config/read` on Codex 0.166.0 (2026-11-11). | Command above |
| Approvals reviewer | Discard sets `approvals_reviewer="user"` in the session controls and `approvalsReviewer: "user"` in `thread/start`. It refuses the session when Codex reports another reviewer. Without this, a user config with a reviewer agent (`guardian_subagent`) let Codex approve a sandbox escape with no Discard card. `thread/start` reported `auto_review` without the override and `user` with it on Codex 0.166.0 (2026-11-11). | Command above |
| Instruction discovery | `project_doc_max_bytes=0`; root AGENTS.md sentinel absent from model requests. | Command above |
| Approved writes | Approving all requests permits outside patch, temporary-HOME touch, and git add. Ordinary approval must not be described as proof that these operations stay sandboxed. | Command above |
| Network | curl exits 6 with network_access=false. | Command above |

The recorded transcript is `fixtures/codex-workspace-turn.jsonl`. Paths are scrubbed.
The probe uses the observed exec_command schema and adds a three-second curl limit.

## Workspace sessions

Workspace starts a fresh ephemeral thread in the granted directory. It uses `untrusted`
approval policy (`on-request` with autorun, see [Autorun](#autorun)) and `workspace-write` sandbox, with network access off and no extra writable
roots. It preserves the exact cached shell and patch capabilities and disables the other
Conversation features. The scoped MCP allowlist adds worktree_commit. Hooks, provider memory,
plugins, other MCP servers and project instruction discovery stay off. `GIT_OPTIONAL_LOCKS=0`
permits read-only Git inspection. System temporary directories remain writable.

The adapter checks effective controls before prompt delivery. Missing controls, mismatches,
invalid directories and non-host approval modes return SandboxUnavailable for read-only
fallback. The model cache must match the running CLI version. The tested-version rule above applies. Workspace uses the same bounded model-list cache recovery as Conversation mode. A stale cache refresh keeps the selected model’s shell and patch capabilities. Ordinary setup, spawn, and protocol failures retain their error type; only failed Workspace controls trigger read-only fallback.

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

## Autorun

Spec decision 11 and 12 govern Codex autorun. Decision 12 requires a spike before Codex may run
sandboxed commands without a card. The spike passed. Codex autorun is supported.

### Autorun spike results

Date: 2026-11-10. Tested codex-cli 0.166.0 on macOS (arm64). The probe starts Codex with
`approvalPolicy: "on-request"` and `sandbox: "workspace-write"` in a linked Git worktree. It uses a
temporary provider home, a temporary repository, and a synthetic local model endpoint that scripts the
tool calls. It uses no credentials and no model turn. It declines every approval request and records it.
The child `TMPDIR` points inside the temporary home, so the main checkout is not writable through a
system temporary directory.

Command for every row: `python3 crates/discard-providers/tests/probes/codex_workspace.py --on-request`.
The probe ran eight times while it grew. Every run passed. The table shows the last run.

| Check | Observed value | Result |
| --- | --- | --- |
| a. `touch .git` (pointer file) | Exit code 1, `Operation not permitted`. Pointer file unchanged. No approval request. | Pass |
| a. `apply_patch` to `.git` | `item/fileChange/requestApproval`, `reason: null`, `grantRoot: null`. Declined. Pointer file unchanged. The preceding `fileChange` item lists `<worktree>/.git`. | Pass |
| a. `apply_patch` to `.GIT` (other case) | Same request as `.git`. Pointer file unchanged. | Pass |
| b. `touch` in the common Git directory | Exit code 1, `Operation not permitted`. No file created. | Pass |
| b. `touch` in the worktree's own Git directory (`.git/worktrees/<name>`) | Exit code 1, `Operation not permitted`. No file created. | Pass |
| b. Append to the common `.git/config` | `operation not permitted`. Config unchanged. | Pass |
| b. `git add -A` in the worktree | Exit code 128, `Unable to create '.../worktrees/<name>/index.lock': Operation not permitted`. Index unchanged. | Pass |
| b. `apply_patch` to the common Git directory (`../repo/.git/...`) | `item/fileChange/requestApproval`. Declined. No file created. | Pass |
| b. `apply_patch` through an in-root symlink to the common Git directory | `item/fileChange/requestApproval` with `reason: "command failed; retry without sandbox?"`. Declined. No file created. | Pass |
| c. `apply_patch` inside the root (`edit.txt`) | No approval request. The `item/started` `fileChange` item lists `<worktree>/edit.txt` before the change applies: the file still had the old text when the notice arrived. The edit then applied. | Pass (second condition) |
| d. Network from the sandbox | `curl` to a loopback listener: exit code 7, listener saw 0 connections. `curl` to `https://example.com`: exit code 6. | Pass |
| e. In-sandbox command (`touch inside.txt`) | Ran. No approval request. | Pass |
| f. Command with `sandbox_permissions: require_escalated` | One `item/commandExecution/requestApproval` with the model's `reason`. Declined. The command did not run. | Pass |
| h1. Shell `touch sub/.git/hooks/probe-hook`, where `sub` is a nested repository made with `git init sub` | Exit code 0. File created. No approval request. | Gap |
| h2. `apply_patch` adding `sub/.git/hooks/probe-hook-patch` | `Success. Updated the following files`. File created. No approval request. | Gap |
| h3. `apply_patch` adding `newdir/.git/x` (no repository there) | `Success`. File created. No approval request. | Gap |
| h4. Shell `mkdir -p newdir2/.git && touch newdir2/.git/x` | Exit code 0. File created. No approval request. | Gap |
| g. `apply_patch` to `.codex/` in the root (information only) | `item/fileChange/requestApproval`, `reason: null`. Declined. No file created. | Information |

Rows h1 to h4 test nested `.git` paths inside the writable root. The probe adds a second file name for
h2 (`probe-hook-patch`), so h1 cannot make it fail. Discard's path classifier protects any path component
named `.git`, at any depth and in any case, and under `untrusted` each of these patches gets a card.
Under `on-request` Codex protects only the root `.git`. Every h write succeeded with no request. This is a
gap against `untrusted`. It does not fail checks a to d, which name the root pointer and the common Git
directory. Rows h are not part of the probe's pass or fail result. The probe prints a separate
`nested .git gap` line.

Rows a to d are the decision 12 conditions. Rows e and f confirm decision 11: under `on-request`,
only an escalation asks. Row g shows that Codex asks before it writes `.codex/`. The engine
classifies that request by path, and `.codex/` has no `.git` component, so it receives automatic
approval. This is the same classification as in `untrusted` sessions. Autorun does not change it.

Limits of the spike:

- The probe ran on macOS (Seatbelt). Autorun is unsupported on other platforms.
- Check c passes through its second condition. With no request, Discard cannot hold the edit. It sees the paths before the change applies. In-root edits already receive automatic approval.
- The probe tests a linked worktree. Autorun is unsupported without a `.git` pointer file.
- Nested `.git` paths are not protected (rows h1 to h4). See the next section.
- The probe does not test read isolation. A Codex command can read any file that the user's account can read.
- The recorded turn is `fixtures/codex-autorun-turn.jsonl`. Paths are scrubbed. Rust tests replay it.

### Nested `.git` paths

An autorun command or patch can create a nested `.git` directory inside the root, or write inside an
existing one, with no card (rows h3, h4, h1, h2). What this permits: a planted hook or Git config in that
nested directory. It runs only if someone later runs Git inside that directory, for example a user or
another tool that enters it. The sandbox still blocks network access and writes outside the root. The
changes card lists the root's tracked changes. It does not show files under a nested `.git`.

The adapter closes the part that it can check at start. At autorun session start, `start_session` walks
the root. It skips the root `.git` pointer. It refuses autorun with `SandboxUnavailable` when any other
entry is named `.git` (any case, file or directory). Symlinks are not followed. The walk reads at most
5,000,000 entries. If it reaches the cap or cannot read a directory, it refuses. The engine then falls back
to a read-only response and shows the reason. Non-autorun Codex sessions are unchanged, because
`untrusted` asks for each patch and command.

The start check cannot see what the agent creates during the session. A new nested `.git` from h3 or h4
stays possible within one autorun response. The engine now walks the root before and after each autorun
response, for every provider. If the second walk finds nested `.git` paths that the first did not, it saves
a warning as a system message of the channel, after the response. The warning names the paths (at most
five). If a walk fails, it says that Discard could not check. The warning does not block, revert, or remove
the write. The next autorun session refuses to start in that worktree until someone removes the nested
directory. The start check and the engine use the same walk, `find_nested_git` in
`crates/discard-core/src/workspace_session.rs`.

Claude has the same limit for shell commands. The adapter's `denyWrite` list holds the root `.git`, the
worktree Git directory, and the common Git directory only, so a sandboxed Claude shell command can write
under a nested `.git` with no card. Claude's native `Write` and `Edit` tools go through the path
classifier and get a card. This comparison comes from the adapter settings and the existing docs. The
Claude probe does not cover nested `.git` paths.

### Autorun sessions

With `SessionSpec.sandbox_autorun` in a Workspace session, `thread/start` uses
`approvalPolicy: "on-request"` and `sandbox: "workspace-write"`. All other Workspace controls stay
the same: network access off, no extra writable roots, and the checks before prompt delivery.
Non-autorun Workspace sessions keep `untrusted`. Coding and Conversation sessions ignore the flag.

Codex runs sandboxed commands without a request and asks only when the model requests an
escalation. Every command approval in an autorun session is therefore an escalation. It keeps the
`SandboxEscape` card with no Always allow action. A command that ran without a card shows "Ran in
sandbox".

File-change approvals keep their classification: paths come from the preceding `fileChange` item.
A path outside the root or with a `.git` component receives the full-access card. In an autorun
session, a file-change request with a non-null `reason` also receives that card. Codex sends such a
request when a patch fails in the sandbox ("command failed; retry without sandbox?"). The retry
leaves the sandbox, so the path does not decide its class.

`start_session` refuses an autorun Workspace session on a platform other than macOS, in a root
without a `.git` pointer file, and in a root with a nested `.git` path. The engine then falls back to a read-only response. `sandbox_autorun()`
reports `Unsupported` with the reason on other platforms. The settings page shows the toggle disabled
with that reason. It is a state block, not planned work.

## Text generation

Personality generation uses a separate app-server request and an empty temporary
working directory. It starts an ephemeral thread with the draft model, developer
instructions, `approvalPolicy: "never"`, `sandbox: "read-only"`, and
`allowProviderModelFallback: false`. The turn uses the draft reasoning effort.
No memory MCP server is added. The verified Codex controls and a sanitized
model catalog remove native tools, plugins, apps, skills, hooks, project instructions,
and automatic memory. Effective configuration checks disable all inherited MCP
servers before the thread starts. A cache matching the running CLI version is required. The same bounded model-list refresh applies when that cache is unavailable or incompatible.
Approval and tool requests are declined. Only the final answer is returned at turn
completion. The process group is killed when the request completes, its future is
dropped, or the 120-second timeout expires. The editor’s Cancel button discards the
result. The backend request can continue until completion or timeout. Generation
does not log prompts or results.
