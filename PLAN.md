# Discard — Design Plan

Discard is an agent harness for managing projects and multiple agents. Its UI follows the Discord desktop client. Lua plugins control both visuals and behavior. Agents can write plugins that modify the app live.

## Stack

- Rust, Tauri 2, and a Vue 3 + TypeScript front end with shadcn-vue and Tailwind CSS 4.
- Luau through `mlua`.
- SQLite (history, FTS5 memory index).
- Platform order: macOS, then Linux, then Windows.

## Workspace layout

| Crate | Role |
| --- | --- |
| `discard-core` | Sessions, agents, memory, storage, plugin-host traits. No GUI dependency. |
| `discard-providers` | Claude and Codex providers. |
| `discard-lua` | Luau host and plugin API. |
| `discard-app` | The Tauri app and the engine-to-webview bridge. |
| `web/` | The Vue 3 front end. |

Testing is TDD-first. `core`, `providers`, and `lua` have unit tests. Providers are tested with a fake provider and recorded CLI transcripts. Lua API tests run headless. UI tests stay thin.

End-to-end failure tests (headless, with a fake provider process) cover: provider process crash mid-turn, invalid resume handle, stale approvals after restart, duplicate approval decisions, worktree creation failure and races, MCP calls with a bad token, memory path escapes and concurrent edits, index corruption, plugin CPU, memory, and queue limit violations, plugin auto-revert, and safe mode.

Discard writes structured logs with secret redaction for provider, plugin, approval, storage, MCP, and workspace operations. Logs carry session, turn, and plugin IDs. A log viewer is a Rust-rendered surface.

## Domain model

- Server = project (repo or workspace folder).
- Text channel = session in a project. One primary agent per channel.
- Member list = agents, with online, working, and idle status.
- Thread = sub-agent run or session branch.
- DMs = project-less agent chats.
- Voice channels: out of scope. Multi-agent channels: later.
- Agent = persistent persona: name, avatar, provider and model, system prompt, permission policy, memory. Stored as TOML. Creatable from the UI and from Lua. Many channels can use one agent. Creating an agent, and any change to an agent's permission policy, needs Rust-rendered user approval. This holds when a plugin or another agent starts the change. A plugin cannot create an agent with a policy looser than the user has approved.

## Providers

- A `Provider` trait wraps the official `claude` and `codex` CLIs as child processes. It uses the user's existing subscription login. It uses ACP adapters where stable.
- Discard never reads or stores credentials. It detects install and login state at startup and in onboarding. If a CLI is missing or logged out, it shows the exact command to run.
- The trait emits one normalized event stream: text delta, tool call start and result, approval request, status change, usage, error. Each event has an optional `raw` JSON field.
- API-key providers: post-v1. Lua-written providers: not in v1.
- **Integration contract (before any shared UI is built).** M1 starts with a spike per provider. Each spike produces a versioned capability matrix, checked in as a test fixture, that records whether the integration supports: interactive approvals, tool-call visibility, per-session MCP server configuration, session resume, and cancellation. Candidate protocols: Claude stream-json or SDK mode, Codex `app-server`, ACP. The `Provider` trait is shaped by the matrix, not assumed. A missing capability is shown in the UI, not hidden.
- **Fail closed.** If a provider cannot enforce an agent's configured approval policy (for example, it cannot pause for approvals), Discard does not run that agent in a looser mode on its own. The agent runs only in a mode the provider can enforce, or it is blocked until the user makes an explicit, approved policy change.
- **Hook enforcement matrix.** The capability matrix also records which hooks each provider can enforce **before** execution (for example, blocking a tool call). A hook that needs pre-execution enforcement is active only on providers that support it. On other providers it is disabled and shown as "observe only" in the plugin UI. Discard never presents an after-the-fact event as a block.
- **Resume and recovery.** The CLI session holds the model context. Discard's SQLite holds the UI history. Before resuming, Discard checks that the resume handle is valid. If the handle is missing or rejected, Discard tells the user and offers to start a new provider session seeded with a summary of the stored transcript. Discard builds the summary, shows it to the user, and starts the new session only after the user confirms or edits it. It never resumes silently into a different context.
- **Turn and approval state machine.** Every event has a durable ID and a per-session sequence number. Turn states: `queued`, `running`, `awaiting_approval`, `completed`, `failed`, `cancelled`. Approval decisions are idempotent (a duplicate decision is ignored and logged). If a provider process exits, its pending approvals become `stale` and cannot be approved. At startup, Discard reconciles non-terminal turns and approvals against live processes. Every approval decision is written to an audit table: who or what decided, policy applied, timestamp.

## Process, storage, workspaces

- v1 is a single process. `discard-core` has no GUI dependency, so a daemon can be split out later.
- Discard's SQLite store is the source of truth for messages, tool calls, and events. The CLI session id is a resume handle only.
- Each channel in a git project gets its own git worktree by default. The user can turn it off per channel or per project.
- Discard selects and creates the workspace **before** it starts the provider session, on the channel's first turn. It never moves a running session to a new directory. Worktree creation is serialized per repository with a lock. The worktree branches from the project's current `HEAD`. Uncommitted files in the main checkout are not copied, and the UI says so. If creation fails, the turn does not start and the error shows in the channel. Discard always keeps the worktree's branch. When a channel is archived, Discard removes the worktree only if all of these are true: no uncommitted changes, no untracked files, no ignored files with content, and no commits that are not on another branch. Otherwise it lists what would be lost and asks the user. Discard never deletes user data without that confirmation.
- Each channel records its workspace mode (`worktree` or `main-checkout`) when it is created. Channels created before worktree support (M1 to M5) stay in their original directory. Moving one to a worktree is an explicit action that starts a new session, with the same confirmed-summary flow as resume recovery.

## Approvals

- CLI permission requests appear in the channel as messages: Approve, Deny, Always allow.
- A global inbox badge counts pending approvals.
- Policy lives in the agent config. Rust enforces policy and renders the approval prompt.
- A Lua hook can auto-decide an approval only if the plugin holds the explicit `approvals.decide` capability. Every auto-decision is audited. A hook that errors or times out fails closed: the approval stays pending for the user.
- Plugins can theme approval prompts, but Rust enforces minimum contrast and readability on them.

## Memory

- Discard hosts a local MCP server. Both CLIs connect to it.
- Tools: `memory_read`, `memory_edit`, `memory_search`, `memory_append`.
- **Caller identity.** Discard issues each provider session a unique, unguessable token and binds it to one agent, project, and policy. The MCP server listens on a local-only transport (Unix socket, mode 0600, or loopback with the token). It authorizes every tool call against the bound identity and the requested scope. A call with a missing or unknown token is rejected and logged. The same rule applies to the `plugin_*` tools.
- **Token handling.** The agent process legitimately holds its own session token. The boundary this protects is between sessions: one agent must not use another agent's identity. So each session gets its own token. Discard passes it through a per-session MCP config file (mode 0600, in Discard's runtime directory, outside the project and worktree), not through command-line arguments. Tokens expire when the session ends or after a maximum lifetime, and Discard can revoke them at any time. Where the platform supports it, the server also checks the peer process on the socket. Memory and `plugin_*` tools use separate connections with separate scopes, so a memory-only session cannot call plugin tools.
- **Stated limit.** Session tokens route calls and scope them for honest sessions. They also give a reliable audit trail. They are **not** a defense against an agent that has shell or filesystem access as the same OS user: such an agent can read another session's token. Discard does not claim otherwise, and the UI does not present MCP scoping as a security boundary against such agents. The real defense for that case is the provider's own sandbox and the user's approval policy for shell and file tools. Discard shows each agent's effective sandbox and approval mode in its profile. OS-level isolation of agent processes is post-v1.
- Core memory blocks are appended to the system prompt at session start. Everything else is reached through tools.
- Storage: markdown files under `~/.discard/memory/`. SQLite FTS5 is a rebuildable search index. No embeddings in v1.
- Scopes: agent-private, project-shared, global. Scope IDs are canonical (agent ID, project ID, fixed `global`), never user-supplied paths. Discard resolves every memory path, rejects `..`, absolute paths, and symlinks that leave the scope root, and checks this in tests.
- Writes are atomic (write to a temp file, then rename). Each memory file has a version. An edit that names a stale version is rejected, and the agent must re-read. Writes to one file are serialized.
- At startup, Discard reconciles the FTS5 index against the files (by mtime and hash) and rebuilds what is stale.
- Background (sleep-time) consolidation: post-v1. The memory API must allow it later.

## Lua plugin system

### Runtime

- Luau via `mlua`, sandboxed. Capabilities are granted per plugin.
- Each plugin gets its **own Luau state**. Each state runs on its **own dedicated OS thread**, never on the app's main thread and never on a shared pool. Execution and I/O workers are both owned by the plugin (see Cancellation below). The number of active plugins is capped (default 64) so thread count stays bounded. They talk to the main thread through bounded channels. One plugin cannot block another or the host.
- **Cancellation by operation type.**
  - Async network calls and timers: cancelled by dropping the task.
  - `exec`: Discard runs each process in its own process group and kills the group on timeout or termination.
  - Filesystem operations: not interruptible once started. Each call is bounded (size and time limits, chunked for large reads or writes), and it runs on that plugin's own reserved workers. Side effects of a call that already started are not reversed. The API docs say so.
  - Each plugin owns two things: its Luau thread (execution) and a small fixed set of I/O workers (default 2). There is no shared pool for plugin work. A plugin that stalls its thread or workers stalls only itself. Calls over its concurrency cap fail fast with an error. The shared async runtime handles only non-blocking, cancellable work (network, timers) and is never used for blocking calls.
  - Tests: timeout of each operation type, and a plugin that exhausts its workers while other plugins keep running.
- Async uses coroutines. Host functions are non-blocking: file, network, and process work runs on a Rust async runtime and resumes the coroutine when done.
- Enforced limits per plugin: a CPU budget per call (Luau interrupt callback), a memory cap (`mlua` memory limit), a bounded event queue (when it is full, Discard drops the oldest events and logs the drop), and a timeout on every host call. A plugin that exceeds a limit is terminated, its state is discarded, and the error badge shows. Repeated violations follow the auto-revert rule below.

### UI

- Each slot is a pure `render(state)` function that returns an element tree.
- Discard re-runs `render` when subscribed state changes (channel data, agent status, plugin signals).
- Rust validates the tree and sends it to the webview, which renders it through fixed Vue components. Plugins never mutate the UI imperatively.
- Plugins control theme tokens (colors, radii, spacing) and register into named slots: sidebar, channel header, message renderer, status bar, command palette.
- Messages are typed blocks: markdown, code, diff, tool call, approval, plugin-defined. Rust ships default renderers. Plugins override by block type, except protected types: `approval`, capability and diff review, and plugin error blocks. Those, and the containers that hold them, are not overridable and cannot be hidden. Plugin theme values are clamped. Protected surfaces keep a minimum contrast ratio, a minimum size, and a visible border.
- The default UI is a bundled Lua plugin where practical.

### Functionality

- Slash commands.
- Event subscriptions with pre/post hooks that can modify or block: message sent, tool call, approval requested, agent status change.
- Custom agent tools, served through the MCP server.
- Timers and background tasks.
- Scoped fs, net, and exec access. Scopes are enforced by the host, not by the plugin:
  - `fs`: explicit list of allowed root paths and read or write mode. Paths are canonicalized, and symlink escapes are rejected.
  - `net`: explicit list of allowed hosts and ports.
  - `exec`: explicit list of allowed executables with argument patterns. Spawned processes get a minimal environment. Discard cannot restrict what an allowed executable does after it starts: it can read files, use the network, and launch other programs. So **every `exec` grant is shown in the approval UI as full local access**, and the executable list is only a record of intent. OS-level sandboxing of plugin processes is post-v1.

### Packaging

- A plugin is a directory in `~/.discard/plugins/` with `plugin.toml` and `init.lua`.
- The manifest declares capabilities: `fs`, `net`, `exec`, `memory`, `ui`, `approvals.decide`, each with its scope list, and an `api` semver.
- Capabilities are deny-by-default. The user approves them at install.
- Hot reload is on in dev mode and available on demand.
- A plugin crash is isolated and shown as an error badge.

## Agent-authored plugins

- The MCP server exposes `plugin_create`, `plugin_edit`, `plugin_reload`, `plugin_test`, `plugin_rollback`, `plugin_list`.
- Agents write to a draft copy, never to the live plugin.
- The user approves a diff plus the capability list before the first load **and before every later change to the plugin's code**, not only capability changes.
- Agent-authored plugins start with zero capabilities.
- A per-plugin "trust future edits" toggle exists. It is offered only for plugins that declare `ui` alone. With it on, later code edits load without review, but a capability change always needs review. For a trusted plugin, the load path is: capture a snapshot, verify that the manifest's capabilities are identical to the approved ones and still `ui` only, then atomically record the new hash and commit it to the plugin repo. If the check fails, the normal review flow applies.
- **Review binds to exact bytes.** Discard snapshots the draft into an immutable directory and computes a content hash. The user reviews that snapshot. Discard installs and loads that same hash atomically. It does not follow symlinks inside plugin directories. If the draft changes after the snapshot, the review is void and the user reviews again.
- **Load-time verification.** An agent with filesystem access can write to the live plugin directory directly, so the `plugin_*` draft rule alone is not enough. Discard **loads from a captured snapshot, not from the live directory**. It reads every plugin file into memory once, hashes those exact bytes, compares the hash to the approved hash, and gives those same bytes to the Luau runtime. `require` inside a plugin resolves against the captured snapshot, never against the disk, so a later change on disk cannot affect the running plugin. This runs at startup, on every reload, and when a file-watcher event triggers a reload. Discard stores approved hashes in its app-data database, with an HMAC whose key is in the OS secret store: macOS Keychain, Windows Credential Manager, or Linux Secret Service. If no secret store is available, Discard asks for a passphrase at launch and derives the key from it. If the user declines, Discard fails closed: it loads no user plugins and shows why. If the content does not match, Discard does not load it. It disables the plugin, shows "modified outside review" in the Rust-rendered plugin list, and offers to review the diff against the last approved commit or to roll back. This applies to every plugin, not only agent-authored ones. A same-user process that can also read the Keychain item is subject to the stated limit in the Memory section.
- `plugin.write` is a permission in the agent policy. It is off by default. A bundled Builder agent has it on.
- These surfaces are rendered by Rust and cannot be replaced or hidden by Lua: tool-approval prompts, capability and diff review, the safe-mode screen, the plugin error list. Plugins may theme them.
- `~/.discard/plugins/` is a git repo. Each approved change is one commit. Rollback is one click.
- Discard auto-reverts a plugin after 3 consecutive load failures or errors.
- Safe mode (launch flag and menu item) starts with all user plugins disabled.
- Agent discoverability: generated `.d.luau` types, generated markdown docs, a `plugin_api_docs` tool, a live `ui_inspect` tool that returns the current slot tree, and a plugin-authoring guide injected into the memory of any agent with `plugin.write`. All are generated from the Rust API definitions, so they cannot drift.

## Menus and Settings

### Actions

- One Rust `Action` enum. The menu bar, context menus, keybindings, and the command palette all dispatch it. No surface has its own handler.
- Items whose backend does not exist yet (for example New channel, Switch channel) show greyed with a "Soon" tag and a tooltip. They have no handler. One Rust list tracks them, so an item turns on when its backend lands.

### Menu bar and context menus

- macOS menu bar first: Discard, File, Edit, View, Window, Help, and a Plugins menu. Linux and Windows get an in-window fallback later.
- Rust owns all menu structure and every protected item: Safe Mode, Review Plugins, Plugin Errors, Settings, Quit. Plugins cannot remove, reorder, or replace Rust items.
- A plugin with the `ui` capability may add items to the Plugins menu and to a labeled section at the end of context menus. Each item shows the plugin name. No new capability is needed.
- A plugin menu callback receives only the target type and ID (for example `message`, `<id>`). It reads content through APIs it already holds.
- Context menu targets: server icon, channel row, member or agent row, message, code block, diff block, tool-call block. Message, code block, and channel row come first.
- Archive channel uses the existing worktree data-loss check and confirmation.

### Settings window

- A separate OS window, single instance, opened with Cmd+,. A sidebar lists categories. A search box filters across pages and plugin fields. Menu items and banners deep-link to a page.
- Pages: General, Appearance, Agents, Providers, Plugins, Memory, Workspaces, Keybindings, Logs, Advanced. Build order: General, Plugins, Providers, Agents, Appearance, then the rest.
- **Plugins page.** It replaces the in-window plugin panel: plugin list, review cards, trust toggle, rollback, safe mode, error list. A Rust-rendered banner and badge stay in the main window when a review is pending or a plugin has an error, so a pending review is never hidden behind Settings.
- **Safe mode.** Settings opens normally in safe mode. The Plugins page says "Safe mode: user plugins disabled" and offers "Relaunch normally". The Safe Mode menu item relaunches the app with or without the flag, because plugins load once at startup.
- **Agents page.** Saving a policy change shows a before and after diff. A looser policy shows a warning and needs an explicit confirm. If the provider cannot enforce a policy, Settings disables that choice and says why. A plugin or agent can never save a policy change through Settings.
- **Appearance page.** The user's settings are the base. A plugin theme layers on top, and Rust still clamps protected surfaces. The page shows which plugin overrides each token. An "ignore plugin themes" switch is available.
- **Keybindings page.** The page edits the map from key to action. System keys are fixed: Cmd+Q, Cmd+, , Cmd+W, Cmd+H, Cmd+M, Cmd+C, Cmd+V, Cmd+X, Cmd+A, Cmd+Z. A conflict blocks the save and names the other action. Open Settings, Quit, and Cancel turn can be rebound but not unbound. Plugins cannot register keybindings in v1. They add menu items and slash commands only.
- **Advanced page.** Three destructive actions: Reset settings (backs up to `settings.toml.bak`), Rebuild memory index, Clear logs. Each shows a confirmation that lists what it changes. There is no "delete all data" action.
- **Logs page.** Discard first adds a rotating, redacted log file in `~/.discard/logs/`, next to the stderr output. The viewer is read-only and follows the file live. It filters by level, component (provider, plugin, approval, storage, MCP, workspace), and session, turn, or plugin ID.

Plugin-declared settings are not built and are in no milestone.

### Settings storage

- App settings live in `~/.discard/settings.toml`. Rust owns the schema and validates it. Changes apply live. A hand edit reloads, and errors show in the window.
- The file has `version = 1`. Discard keeps unknown keys when it writes. An invalid value falls back to its default for that key only, and the window shows the error. A bad file never blocks startup.
- Per-agent and per-project settings stay in their existing stores.
- **Plugin settings.** A plugin declares typed fields in `plugin.toml` (bool, string, number, choice). Rust renders them under Plugins > `<plugin name>`. Plugins cannot draw their own settings UI. Values live in a plugin namespace in `settings.toml`. A plugin reads them through `discard.settings`.

### Structure and tests

- `SettingsStore` and the `Action` list live in `discard-core` with no GUI dependency. The Settings window is a second Tauri window with Vue pages in `web/src/settings/`. It shares the app's existing services.
- TDD-first. Core logic has unit tests. Settings validation has failure tests: invalid values, unknown keys, version mismatch, a bad file at startup, a conflicting keybinding, and a policy the provider cannot enforce. UI tests stay thin.

## Milestones

1. **M1** — Desktop shell, Claude provider, streaming chat, SQLite history, approvals.
2. **M2** — Luau host, theme tokens, slots, default UI as a plugin.
3. **M3** — Memory MCP server and memory files.
4. **M4** — Codex provider.
5. **M5** — Agent-authored plugins with the full safety stack.
6. **M6** — Worktrees per channel.
7. **M7** — `Action` layer, `SettingsStore`, native menu bar, Settings window shell with General and Plugins pages. The plugin panel moves into Settings.
8. **M8** — Context menus on message, code block, and channel row. Plugin menu items. Providers, Agents, and Appearance pages.
9. **M9** — Remaining context menu targets. Keybindings, Memory, Workspaces, Logs (with the file sink), and Advanced pages.

M1 and M2 come first so the plugin architecture shapes the UI from the start.

## Out of v1

Voice channels, multi-agent channels, API-key providers, Lua providers, embeddings, background memory consolidation, daemon split, Windows.
