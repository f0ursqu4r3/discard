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

Conversation sessions pass `--allowedTools` with the exact scoped Discard tool names: six memory tools, two worktree tools, and three project-file tools. The server further limits the listed tools by conversation scope. This lets private saves, reads, shared proposals, and scoped project inspection run even when the session has no permission prompt. It also avoids duplicate host prompts in HostPrompts mode. Built-in tools remain absent. Discard's memory service still controls scope access and shared-write approval; a shared save remains a pending proposal. Coding sessions receive no new tool grant.

The adapter checks the CLI version and required help controls before sending instructions. It rejects detected local managed settings that conflict with memory, hooks or plugin restrictions. Managed policy remains authoritative. Claude can receive remote or MDM policy that the CLI does not expose through an effective-settings API. Discard cannot independently verify that opaque policy, and does not claim administrator or operating-system isolation.

The opt-in synthetic probe at `crates/discard-providers/tests/probes/claude_conversation.py` checks the actual model-facing tools and executes a synthetic private save, read and shared proposal against local test endpoints. It verifies tool-call receipt, result delivery and a pending shared proposal with no shared write. The tested version exposed only the six Discard memory tools. The probe uses a temporary configuration directory and a synthetic API key. Process-fixture tests cover flags, environment overrides, fresh instructions and cleanup.

Run the local probe with `python3 crates/discard-providers/tests/probes/claude_conversation.py`. The script works from any working directory when you give its full path. Use `--claude /path/to/claude` or `DISCARD_CLAUDE_BIN` to select the CLI executable.

Normal app conversations replay the current conversation transcript with refreshed personality and applicable saved memory before every response. Memory edits apply to existing conversations without restarting. Native resume behavior described above applies to Coding sessions. See [Saved memory](memory.md) for the four domains, shared-write approval, Forget, and capture controls.

## Workspace sessions (spike results)

Tested Claude Code 2.1.318 with synthetic local model requests and temporary homes.
Command for every row: `python3 crates/discard-providers/tests/probes/claude_workspace.py`.

| Question | Observed value | Probe command |
| --- | --- | --- |
| Native tools | Exactly Read, Glob, Grep, Edit, Write, Bash. | Command above |
| In-root Edit | `can_use_tool`, with `input.file_path`; allow edits the file. | Command above |
| Outside Write | `--restricted` rejects it before a card; without that flag an allow writes outside. Keep `--restricted`. | Command above |
| Bash sandbox | In-root touch succeeds; touch in the temporary HOME fails with Operation not permitted. | Command above |
| Shell approvals | touch, curl, and git add ask; git status runs without asking. | Command above |
| Network approval | Separate `can_use_tool` named `SandboxNetworkAccess`, with `input.host=example.com`. An allow permits that domain. | Command above |
| Instruction discovery | The root AGENTS.md/CLAUDE.md sentinel never reaches the model with `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`. | Command above |
| Git writes | The three specified sandbox settings alone permit git add. A third run adds `sandbox.filesystem.denyWrite=[<absolute .git path>]`; git add then fails with Operation not permitted. Workspace must protect Git metadata explicitly. | Command above |
| Escape marker | No escape was offered with `allowUnsandboxedCommands=false`; explicit unsandboxed inputs must never become ordinary approvals. | Command above |

The recorded transcript is `fixtures/claude-workspace-turn.jsonl`. Paths are scrubbed.
The probe adds a three-second curl limit and a third Git-protection run.

## Workspace sessions

Workspace uses the Conversation isolation controls and a fresh replay, with the bound directory
as cwd. It enables only Read, Glob, Grep, Edit, Write and Bash, plus the scoped Discard tools
and worktree_commit. Native resume is refused. Hooks, plugin directories, automatic memory,
and instruction discovery remain off. `GIT_OPTIONAL_LOCKS=0` allows read-only Git commands.

The profile requires macOS, sandbox-exec, HostPrompts, the pinned version, and the required
help controls. It rejects conflicting local managed sandbox settings. It keeps `--restricted`,
sets sandbox.enabled, disables unsandboxed commands and automatic Bash approval, and denies
writes to the worktree's .git path, Git directory and common Git directory. Missing controls
return SandboxUnavailable so the engine can use the read-only profile. Remote managed policy
has the same visibility limit as Conversation mode.

Edit paths are explicit. SandboxNetworkAccess requests use NetworkDomain cards. Explicit
unsandboxed requests use SandboxEscape cards. Neither plugins nor automatic edit rules can
approve these cards.

Supplemental synthetic review probes found that native `Write` can change a linked worktree's `.git` pointer after approval even with `--restricted` and `sandbox.filesystem.denyWrite`. Those sandbox restrictions protect shell writes. Discard therefore treats native edits to Git metadata, its aliases, outside paths, and unknown targets as user-only full-access requests before storing approvals.

### Slice B spike results

The synthetic main-checkout probe uses a normal temporary repository, a temporary
provider home, and a local model endpoint. Run:
`python3 crates/discard-providers/tests/probes/claude_workspace.py --main-checkout`.
All approval requests are accepted to measure provider protection itself.

Tested Claude Code 2.1.318 with `--restricted` and
`sandbox.filesystem.denyWrite=[<root>/.git]`. Select this pinned binary with
`DISCARD_CLAUDE_BIN` if the installed default has changed.

`claude_protects_git_in_root = false`: `touch .git/marker`, `git add -A`,
and `git commit -m x` failed with `Operation not permitted`. Native `Write` to
`.git/config` succeeded after approval. Shell protection does not protect native
file tools. This raw probe result is unchanged.

Decision 20e enables Claude main-checkout grants in Discard. The sandbox's
`denyWrite` blocks shell writes to `.git`. The driver classifies native file-tool
edits under `.git` as `SandboxEscape`, so they need the full-access card. These
controls also protect linked worktrees.


Slice B adds scoped `worktree_request_creation` and `main_checkout_request_write`
tools to Conversation-family allowlists. The MCP server checks the agent and
channel before listing or running them. App approval starts a fresh Continue
response. Claude advertises main-checkout write support. The engine passes that
capability into the scoped MCP identity before the server lists the request tool
or adds its prompt line. Approval allows one write response in the main checkout.
Missing sandbox controls still cause a read-only fallback. Linked-worktree writes
keep their existing protection checks. Coding behavior stays unchanged.

Discard saves simple command-prefix rules and exact network-host rules per agent
and project. The user chooses the prefix cut. Rules approve one provider request
at a time and record their rule ID in the audit. Project settings support revocation.
Compound shell syntax and sandbox escapes cannot use these rules.
