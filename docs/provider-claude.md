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

`SessionMode::Conversation` and Workspace use Claude Code 2.1.318 as the tested version. A newer CLI may run only after all required isolation controls pass verification. Older versions stay blocked with an update message. An unreadable version also stays blocked. It uses a fresh, empty Discard-owned working directory and rejects native resume handles, unless Discard passes `resumable`. A resumable session uses a stable Discard-owned directory and persists its transcript. It sets `DISABLE_AUTO_COMPACT=1`. Discard supplies the selected model and refreshed system instructions for a new transcript replay.

The adapter passes `--tools ""`, `--strict-mcp-config`, `--disable-slash-commands`, `--restricted` and `--system-prompt-snapshot off`. It accepts only the scoped Discard memory MCP configuration. It forces `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` and `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` in both process environment and explicit session settings. It disables hooks and removes inherited explicit plugin directories. Authentication uses the existing CLI login; Discard does not use `--bare`, copy credentials or change user settings.

Conversation sessions pass `--allowedTools` with the exact scoped Discard tool names: six memory tools, two worktree tools, and three project-file tools. The server further limits the listed tools by conversation scope. This lets private saves, reads, shared proposals, and scoped project inspection run even when the session has no permission prompt. It also avoids duplicate host prompts in HostPrompts mode. Built-in tools remain absent. Discard's memory service still controls scope access and shared-write approval; a shared save remains a pending proposal. Coding sessions receive no new tool grant.

The adapter checks the CLI version and every `--flag` emitted for the session against `claude --help`, with the fixed isolation-control list as a minimum, before sending instructions. A missing flag stops setup and the error names the flag, running version, and tested version. It rejects detected local managed settings that conflict with memory, hooks or plugin restrictions. Managed policy remains authoritative. Claude can receive remote or MDM policy that the CLI does not expose through an effective-settings API. Discard cannot independently verify that opaque policy, and does not claim administrator or operating-system isolation.

After a newer CLI starts, Discard adds a conversation marker with the running and tested versions. Each distinct notice appears at most once per engine instance. Notice storage does not block the turn. A newer CLI can change the behavior of a control without removing it; these checks cannot detect every such change.

The opt-in synthetic probe at `crates/discard-providers/tests/probes/claude_conversation.py` checks the actual model-facing tools and executes a synthetic private save, read and shared proposal against local test endpoints. It verifies tool-call receipt, result delivery and a pending shared proposal with no shared write. The tested version exposed only the six Discard memory tools. The probe uses a temporary configuration directory and a synthetic API key. Process-fixture tests cover flags, environment overrides, fresh instructions and cleanup.

Run the local probe with `python3 crates/discard-providers/tests/probes/claude_conversation.py`. The script works from any working directory when you give its full path. Use `--claude /path/to/claude` or `DISCARD_CLAUDE_BIN` to select the CLI executable.

Normal app conversations replay the current conversation transcript with refreshed personality and applicable saved memory before every response. Memory edits apply to existing conversations without restarting. Agents with **Keep provider session** on resume their Claude session between responses (see [Conversation runtime](runtime/conversation-context.md#native-provider-resume)); other agents replay. See [Saved memory](memory.md) for the four domains, shared-write approval, Forget, and capture controls.

## Native resume spike results

Tested Claude Code 2.1.325 (the installed version; the pinned tested version stays 2.1.318). The probes used the real CLI, the Haiku model, a stable empty working directory, and the Conversation isolation flags (`--tools`, `--strict-mcp-config`, `--disable-slash-commands`, `--restricted`, `--system-prompt-snapshot off`, `--settings`). A stdio MCP server returned a fixed token, `TOKA` or `TOKB`. Resume used `--resume <id>`.

| Question | Result |
| --- | --- |
| Resume from a stable empty directory | Yes. `--resume` found the session and the `system/init` event kept the same `session_id`. A resume from a different directory also worked; `init.cwd` showed the new directory. Use the same directory anyway. |
| `--tools` on resume | Re-applies. A session started with `--tools ""` showed `Bash` in `init.tools` after a resume with `--tools Bash`. A resume with `--tools ""` removed it again. |
| `--permission-mode` on resume | Re-applies. `init.permissionMode` was `plan` after a resume with `--permission-mode plan`. |
| `--mcp-config` on resume | Re-applies. A session started with the `TOKA` server returned `TOKB` after a resume with the `TOKB` config. |
| `--allowedTools` on resume | Re-applies. The scoped tool ran with no permission prompt. |
| `--append-system-prompt` on resume | Not stored in the session. A resume without the flag lost the earlier prompt (the model could not find the codeword). A resume with a new prompt used the new one. Pass the prompt on every resume. |
| Sandbox settings in `--settings` on resume | Re-applies. A resume with `sandbox.filesystem.denyWrite=[<dir>]` blocked `touch <dir>/m1` (`Operation not permitted`) and allowed a write in the working directory. |
| `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` on resume | Still applies. A `CLAUDE.md` sentinel in the working directory was invisible in the first turn and in the resumed turn. A control run without the variable saw it. |

No control failed. Claude does not need "adapter refuses resume". The adapter must pass every isolation flag, the MCP config, `--allowedTools`, `--settings` and the system prompt again on each resume. This is the same argument list as a fresh start plus `--resume`.

### Handle rejected

| Case | Behavior |
| --- | --- |
| Unknown or expired ID (a deleted session file gives the same result) | Exit code 1. No `system/init` event. stderr holds `No conversation found with session ID: <id>`. stdout holds one `result` event: `subtype: "error_during_execution"`, `is_error: true`, `num_turns: 0`, `errors: ["No conversation found with session ID: <id>"]`. |
| Value that is not a UUID | Exit code 1. `errors: ["Error: --resume requires a valid session ID or session title when used with --print. ..."]`. |

Marker for the engine: a `result` event with `subtype: "error_during_execution"` and an `errors[]` entry that starts with `No conversation found with session ID`, with no `init` event before it and exit code 1. Treat a non-UUID error the same way, because Discard stores only IDs that Claude gave it.

### Session store

| Setting | Session file |
| --- | --- |
| Default | `~/.claude/projects/<working directory with / and . replaced by ->/<session_id>.jsonl` |
| `HOME` changed | `<HOME>/.claude/projects/...`. The login is lost (`Not logged in`). |
| `CLAUDE_CONFIG_DIR` changed | `<CLAUDE_CONFIG_DIR>/projects/...`. The login is lost in the test. |

Resume reads the store of the current `HOME` and `CLAUDE_CONFIG_DIR`. Discard must keep both unchanged between turns of one session. Claude removes old sessions by its own cleanup; a removed file is an expired handle.

### Compaction

Claude compacts a long session on its own, and the compaction also runs on a resumed session.

- Flags: `--autocompact <auto|tokens>` sets the window (100k to 1M tokens). The environment variable `DISABLE_AUTO_COMPACT=1` turns it off. The settings key `autoCompactEnabled` and `DISABLE_COMPACT` exist in the binary and were not probed.
- Test: a Haiku session with `--autocompact 100000` grew past 160k tokens over three resumed turns with `DISABLE_AUTO_COMPACT=1` and showed no compaction. The next resumed turn without the variable compacted: `pre_tokens` 198262, `post_tokens` 37519.
- Stream signal: `system` event with `subtype: "status"` and `status: "compacting"`, then `system` with `subtype: "compact_boundary"` and `compact_metadata` (`trigger: "auto"`, `pre_tokens`, `post_tokens`). A compaction that cannot run shows `status: null`, `compact_result: "failed"`, `compact_error`.
- With `DISABLE_AUTO_COMPACT=1` Discard owns the size guard. The behavior near the full window with the variable set was not probed.

### Live probe result

`cargo test -p discard-providers --test claude_live real_claude_resumes -- --ignored --nocapture` passed on Claude Code 2.1.325 with model `haiku` (2026-11-18). It ran three Conversation turns for one agent with **Keep provider session** on. A new fact, not an instruction or preference, was saved between turns 2 and 3. Turn 1 started a session. Turn 2 and turn 3 resumed it with `--resume`. The answer in turn 3 used the new fact, which reached the model only through the context update. The saved `agent_sessions` row kept the same handle after all three turns. No "Fresh replay" notice appeared.

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
and worktree_commit. Native resume is available for agents that keep their provider session. Hooks, plugin directories, automatic memory,
and instruction discovery remain off. `GIT_OPTIONAL_LOCKS=0` allows read-only Git commands.
Every Workspace session passes `--setting-sources ""`. The agent can write
`<root>/.claude/settings.json` and `settings.local.json`, so Claude must not load them. Only the
`--settings` value configures the sandbox. The adapter's help check covers this flag.

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

### Sandbox autorun

With `SessionSpec.sandbox_autorun`, a Workspace session also sets
`sandbox.autoAllowBashIfSandboxed: true`, `sandbox.filesystem.denyRead`, and
`permissions.deny` `Read(//absolute/path/**)` rules. The paths come from
`crates/discard-core/src/sandbox_reads.rs`: credential stores in the home directory,
the Claude and Codex config directories, and every Discard file outside the session
root. The sandbox covers Bash only, so the `Read` rules protect the native tools.
The rules escape `*`, `?`, `[`, `]`, `{` and `}` with a backslash. The sandbox paths stay literal.
The home directory and relocated directories are canonicalized first. Dotfiles setups often
symlink credential paths, such as `~/.ssh -> ~/dotfiles/ssh`. Discard denies the link and its real
target. It also walks each credential directory (4 levels, 2000 entries, no directory symlinks
followed) and denies the real target of every symlink that points outside that directory.
Dangling symlinks are skipped. Discard never denies a target that equals or contains the session root. A relative
`CLAUDE_CONFIG_DIR` or `CODEX_HOME` is ignored, so the default directory stays denied.

Claude saves large tool outputs under `<Claude config>/projects/<encoded cwd>/` and reads them
back with `Read`. The Claude config directory is `CLAUDE_CONFIG_DIR` or `~/.claude`. The adapter
denies every entry of that directory except `projects/<encoded session root>`. Claude encodes
the cwd by replacing each character that is not an ASCII letter or digit with `-`. Claude
shortens names longer than 200 characters with a hash. Discard does not match those, so the
whole `projects` directory stays denied for such a root.

Autorun sessions also set `GIT_TERMINAL_PROMPT=0`, `GIT_CONFIG_COUNT=1`,
`GIT_CONFIG_KEY_0=credential.helper` and `GIT_CONFIG_VALUE_0=` (empty). Git then reads no
credential helper, such as the macOS Keychain helper, and never prompts.
Autorun sessions also add `Bash(security:*)` to `permissions.deny` and remove the GitHub
token variables from the child environment. Both are partial Keychain mitigations. The
settings `env` never sets those variables.
The session still needs the Discard data directory. Without it, `start_session`
returns `SandboxUnavailable`. Local managed settings that set `autoAllowBashIfSandboxed`
still fail the managed-settings check.

Nested `.git` paths. The adapter's `sandbox.filesystem.denyWrite` list holds the root `.git` pointer,
the worktree's Git directory, and the common Git directory. It does not cover a `.git` below the root.
A sandboxed Claude shell command can therefore write under a nested `.git` with no card. Native `Write`
and `Edit` still get a card, because the path classifier protects any `.git` component. This is inferred
from the adapter settings. The live probe does not cover it. Claude autorun does not refuse a worktree
that already has nested `.git` paths. Instead, the engine walks the root before and after each autorun
response. It shows a notice for existing paths, once for each set per session. It saves a warning as a
system message when the response created new ones. See the Codex page for the shared check.

Live probe results. Run:
`cargo test -p discard-providers --test claude_live -- --ignored autorun --nocapture`.
The probe uses the real subscription, model `haiku`, one turn, and a linked worktree in
a temporary directory. The fake credential file is `<temporary home>/.ssh/probe_credential`. The temporary home
is named `home[1]` to check glob escaping. The CLI child keeps the real `HOME` so that login
works. The adapter builds its denial list from the temporary home. The temporary home's
`.claude` is a link to the CLI's real config directory, so the walk runs on the real directory.
The probe reads only entry names there. Before the turn, the probe writes
`<root>/.claude/settings.local.json` with `sandbox.network.allowedDomains: ["example.com"]`.
Set `DISCARD_CLAUDE_BIN` to choose another executable.

Tested Claude Code 2.1.321. The adapter's tested version is 2.1.318. The newer version
passes the version gate with a notice.

| Question | Observed value |
| --- | --- |
| Writing command | `touch inside.txt` ran and created the file. No `can_use_tool` request. |
| Git directory write | `touch <common git dir>/probe-marker` failed with Operation not permitted. No marker. A linked worktree's `.git` is a file, so the probe uses the common directory. |
| Shell read of a denied file | `cat '<temporary home>/.ssh/probe_credential'` failed with Operation not permitted. The path contains `[1]`. |
| Native read of a denied file | `Read` returned "File is in a directory that is denied by your permission settings." No approval request. The escaped `home\[1\]` rule matched. |
| Native Grep | `Grep` with path `<temporary home>` returned "... is outside <root>; --restricted confines the file tools to the working directory." `--restricted` refuses it before the deny rules. |
| Native Glob | `Glob` with path `<temporary home>/.ssh` returned "Permission to read <temporary home>/.ssh has been denied." No file names. |
| Network | `curl -m 3 https://example.com` sent `SandboxNetworkAccess` with `input.host=example.com`, although `<root>/.claude/settings.local.json` allowed the domain. The probe denied it. |
| Keychain (before the deny rule) | `security find-generic-password -s DISCARD_PROBE_NONEXISTENT` printed "The specified item could not be found in the keychain." and `exit=44`. The sandbox does not block Keychain queries. |
| Keychain (with `Bash(security:*)` denied) | `security find-generic-password -s DISCARD_PROBE_NONEXISTENT; echo exit=$?` returned an error: "Permission to use Bash with command ... has been denied." The command did not run. The deny rule is enforced although sandboxed Bash is auto-allowed. No approval request. |
| Large output | `seq 1 200000` saved its output to `~/.claude/projects/-private-var-folders-...-discard-worktrees-probe/<session>/tool-results/<id>.txt`. `Read` with offset 199995 returned lines 199995 to 199997. |
| Nonexistent denial paths | Accepted. The temporary home had no `.aws`, `.gnupg`, and so on. The session started. |
| Secret leakage | The secret text appeared in no event. |

Not probed live: reads of files outside the denied list. The sandbox does not limit those reads.
The Keychain probe queried only a nonexistent item. Claude's sandbox always allows the
Keychain service. Discard denies `Bash(security:*)` and removes `GH_TOKEN`, `GITHUB_TOKEN`,
`GH_ENTERPRISE_TOKEN`, and `GITHUB_ENTERPRISE_TOKEN` from the Claude process. A script can
still call the Keychain directly, for example through another tool or language runtime.
macOS may show its own access prompt for a protected item.

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

## Text generation

Personality generation uses a separate non-interactive request in an empty temporary
working directory. It sends the draft model and optional effort, replaces the system
prompt with `--system-prompt`, writes the description on stdin, and reads the JSON
result. It uses `--tools ""`, strict empty MCP configuration, no session persistence,
no slash commands, no user/project/local settings sources, and `--restricted`.
Hooks and automatic memory are off. The memory and instruction-discovery environment
controls are set to `1`; inherited plugin directories are removed. Detected conflicting
local managed settings stop the request. The process group is killed when the request
completes, its future is dropped, or the 120-second timeout expires. The editor’s
Cancel button discards the result. The backend request can continue until completion
or timeout. Generation does not log prompts or results.


## Custom API address reachability

Before any Claude session or text-generation CLI starts, Discard checks an explicit
`ANTHROPIC_BASE_URL`. A provider environment override takes priority over the process
environment. An explicit removal counts as unset. For a URL with a host and a port,
Discard opens a TCP connection with a two-second timeout. The default port is 443
for HTTPS and 80 for HTTP. It sends no data. An unset or unparseable address skips
the check.

If the connection fails, setup stops and names the address. The error includes only
the scheme, host, and port; it excludes user information, paths, and queries. Start
that service or remove the variable, then retry. Discard does not change the address
or environment. This check does not cover proxy variables or settings-file values.
A reachable address can still fail later; the text-generation timeout message remains
as a fallback. Claude text generation has no CLI version gate.
