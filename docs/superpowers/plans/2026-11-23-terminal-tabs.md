# Terminal Tabs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A project channel runs up to 8 terminal tabs, each its own shell session that starts in the channel worktree.

**Architecture:** The backend registry keys sessions by session ID and keeps channel, order, and tab number per entry. Commands take session IDs. The panel lists tabs from the backend and shows the active tab in one xterm instance, which it re-attaches on a tab switch.

**Tech Stack:** Rust (Tauri 2 commands, `portable-pty`), Vue 3 + TypeScript, xterm.js, Vite+ tests.

**Spec:** `docs/superpowers/specs/2026-11-23-terminal-tabs-design.md`

## Global Constraints

- The terminal is for the user only. No new path for agents, MCP tools, plugins, or the plugin event stream. Discard never logs terminal input or output.
- Cap: 8 tabs per channel. Refusal text: `A channel can have 8 terminals.`
- Tab name: `Terminal N`. N is the highest number among the open tabs of the channel plus one (1 for the first).
- A new tab always starts in the worktree of the channel (`cwd::for_channel`).
- Kill keeps the tab. Restart keeps its position and number. Closing the last tab hides the panel.
- Web tests live in a `__tests__` directory beside the code. Use short, direct sentences in docs and UI text.
- Remove the `TODO` for "New Terminal Tab". No other planned item changes.
- Run Rust tests with `CLAUDE*` and `ANTHROPIC*` variables unset. Run web tests with `pnpm --dir web test -- <path>`.

## Review Focus

- Close a tab that is not active: the active tab and its stream stay.
- Restart when the worktree is gone: the panel shows the error and the tab stays.
- Window B opens a tab that window A showed: A hears `moved`, and A still shows its tab list.
- Channel switch with several tabs: only the active tab detaches. Other tabs keep running. Coming back shows the remembered tab.
- The webview shows 7 tabs but the backend has 8 (a race): the backend refuses the 9th and the panel shows the error.

## File Structure

- `crates/discard-app/src/desktop/terminal.rs`: registry and commands.
- `crates/discard-app/src/desktop/mod.rs`: command registration (line ~343).
- `crates/discard-app/tests/terminal.rs`: registry tests on real shells.
- `web/src/bridge/terminal.ts`: command wrappers and types.
- `web/src/components/terminal/terminal-state.ts`: tab helpers.
- `web/src/components/terminal/TerminalPanel.vue`: tab strip and flow.
- `docs/project-channel-status.md`: Terminal section.

### Task 1: Backend registry and commands

**Files:**
- Modify: `crates/discard-app/src/desktop/terminal.rs`, `crates/discard-app/src/desktop/mod.rs`
- Test: `crates/discard-app/tests/terminal.rs`, unit tests in `terminal.rs`

**Interfaces:**
- Produces (Rust):
  - `pub const MAX_TABS: usize = 8;` and `pub const TAB_LIMIT: &str = "A channel can have 8 terminals.";`
  - `pub fn next_number(open: &[u32]) -> u32` (max + 1, or 1).
  - `Terminals::create(&self, channel: &str, project: &str, spec: SpawnSpec) -> Result<(Arc<Session>, u32), String>` (session and number; `Err(TAB_LIMIT)` at 8).
  - `Terminals::get(&self, session_id: &str) -> Option<Arc<Session>>`.
  - `Terminals::channel_of(&self, session_id: &str) -> Option<(String, String)>` (channel, project).
  - `Terminals::tabs(&self, channel: &str) -> Vec<(u32, Arc<Session>)>` in creation order.
  - `Terminals::restart(&self, session_id: &str, spec: SpawnSpec) -> Result<(Arc<Session>, u32), String>`: spawns first, then swaps the entry (same order and number), then ends the old session. A failed spawn keeps the old tab.
  - `Terminals::close_session(&self, session_id: &str, reason: Option<&'static str>)`.
  - `Terminals::kill(&self, session_id: &str, reason: &'static str)`: announces and terminates, keeps the entry.
  - `close_channel`, `close_project`, `close_inside`, `close_missing`, `close_all` keep their names. `close_channel` ends every tab of the channel.
  - Commands (all `#[tauri::command] async`, app windows only): `terminal_tabs(channel_id) -> Vec<TerminalTab>`, `terminal_create(channel_id, cols, rows, on_event) -> OpenedTerminal`, `terminal_open(session_id, cols, rows, on_event) -> OpenedTerminal`, `terminal_restart(session_id, cols, rows, on_event) -> OpenedTerminal`, `terminal_kill(session_id)`, and `terminal_write`, `terminal_resize`, `terminal_detach`, `terminal_close` taking `session_id`.
  - `TerminalTab { session_id, number, cwd, running, exit_code, signal }` (camelCase). `OpenedTerminal` gains `number: u32`. `terminal_status` and `TerminalStatus` are removed.

- [ ] **Step 1: Write failing tests.** In `tests/terminal.rs`, replace `open`/`get(channel)` usage with `create`/`get(session_id)`. Add: `a_channel_runs_several_sessions_in_order` (two creates give numbers 1 and 2 and `tabs` order), `the_ninth_session_of_a_channel_is_refused` (error equals `TAB_LIMIT`; another channel still creates), `closing_a_channel_ends_all_its_tabs`, `a_new_tab_after_a_close_takes_the_highest_number_plus_one` (open 1,2,3, close 3, create gives 3; close 1, create gives 4), `restart_keeps_position_and_number` (new session id, same slot, old one gone), `a_failed_restart_keeps_the_old_tab` (spec with missing shell), `kill_keeps_the_tab_and_ends_the_shell`. In `terminal.rs` unit tests: `next_number_is_one_more_than_the_highest` (`[]`→1, `[1,2]`→3, `[2,5]`→6).
- [ ] **Step 2: Run to confirm failure.** `cargo test -p discard-app --test terminal` with Claude vars unset. Expected: compile errors for the missing API.
- [ ] **Step 3: Implement the registry.** `Entry { session, project, channel, order: u64, number: u32 }`. Use a `next_order: u64` counter inside the same mutex as the map. `create` checks the count and spawns under the lock as `open` did.
- [ ] **Step 4: Implement the commands.** Factor one helper that attaches a window and builds `OpenedTerminal`. `terminal_create` and `terminal_restart` get the folder with `cwd::for_channel` and the shell with `shell_path`/`login_spec`; `terminal_restart` finds the channel through `channel_of`. Ownership rules (`owned_session`, `check_owner`) stay as they are. Replace the registrations in `mod.rs` and add the new ones.
- [ ] **Step 5: Run tests.** `cargo test -p discard-app` and `cargo build -p discard-app`. Expected: all pass, no warnings from the changed files.
- [ ] **Step 6: Commit.** `git commit -m "feat: key terminal sessions by ID and add tab commands"`

### Task 2: Bridge and tab helpers

**Files:**
- Modify: `web/src/bridge/terminal.ts`, `web/src/components/terminal/terminal-state.ts`
- Test: `web/src/bridge/__tests__/terminal.test.ts`, `web/src/components/terminal/__tests__/terminal-state.test.ts`

**Interfaces:**
- Consumes: Task 1 command names and argument names (`channelId`, `sessionId`, `cols`, `rows`, `onEvent`).
- Produces (TypeScript):
  - `interface TerminalTab { sessionId: string; number: number; cwd: string; running: boolean; exitCode: number | null; signal: string | null }`; `TerminalOpened` gains `number: number`; `TerminalStatus` and `terminalStatus` are removed.
  - `terminalTabs(channelId): Promise<TerminalTab[]>`, `terminalCreate(channelId, cols, rows, onEvent)`, `terminalOpen(sessionId, cols, rows, onEvent)`, `terminalRestart(sessionId, cols, rows, onEvent)`, `terminalKill(sessionId)`, and `terminalWrite/Resize/Detach/Close` taking `sessionId`.
  - In `terminal-state.ts`: `MAX_TABS = 8`, `TAB_LIMIT_REASON = 'A channel can have 8 terminals.'`, `tabName(n: number): string`, `canAddTab(count: number): boolean`, `pickActive(tabs: {sessionId: string}[], remembered?: string): string | null` (remembered if present, else the first, else null), `nextActive(tabs: {sessionId: string}[], closed: string): string | null` (right neighbour, else left, else null), `rememberActive(channelId: string, sessionId: string): void`, `recallActive(channelId: string): string | undefined`.

- [ ] **Step 1: Write failing tests.** Bridge: each wrapper calls its command with the exact argument object (`terminal_tabs` with `{ channelId }`, `terminal_create` with `{ channelId, cols, rows, onEvent }`, `terminal_write` with `{ sessionId, data }`, and so on). State: `tabName(2)` is `Terminal 2`; `canAddTab(7)` true and `canAddTab(8)` false; `pickActive` with remembered present, missing, and an empty list; `nextActive` for middle, last, only tab; `recallActive` returns what `rememberActive` stored for that channel only.
- [ ] **Step 2: Run to confirm failure.** `pnpm --dir web test -- src/bridge/__tests__/terminal.test.ts src/components/terminal/__tests__/terminal-state.test.ts`. Expected: FAIL, missing exports.
- [ ] **Step 3: Implement** the wrappers and helpers above. `rememberActive` uses a module-level `Map`.
- [ ] **Step 4: Run the same tests.** Expected: PASS. `TerminalPanel.vue` does not compile against the new bridge until Task 3, so do not run the full suite here.
- [ ] **Step 5: Commit.** `git commit -m "feat: bridge and helpers for terminal tabs"`

### Task 3: Panel, tests, and docs

**Files:**
- Modify: `web/src/components/terminal/TerminalPanel.vue`, `docs/project-channel-status.md`
- Test: `web/src/components/terminal/__tests__/TerminalPanel.test.ts`, check `web/src/__tests__/App.terminal.test.ts`

**Interfaces:**
- Consumes: Task 2 bridge and helpers. Props and emits of `TerminalPanel` stay `channelId`, `toggleShortcut`, `hide`.
- Produces: tab strip with `data-testid="terminal-tabs"`; each tab `data-testid="terminal-tab"` (active has `aria-selected="true"`) with a close button `data-testid="terminal-tab-close"`; add button `data-testid="terminal-new-tab"` (replaces the planned item; disabled with `TAB_LIMIT_REASON` as title at 8).

- [ ] **Step 1: Write failing tests** in `TerminalPanel.test.ts` (extend the `invoke` mock so `terminal_tabs` returns the tab list and `terminal_create`/`terminal_open`/`terminal_restart` return `opened()` with a `number`): `it lists the tabs of the channel and opens the remembered one`, `a channel without tabs starts one with terminal_create`, `+ creates a tab and makes it active`, `+ is disabled with the limit reason at 8 tabs`, `a backend refusal of the ninth tab shows its message`, `switching tabs detaches the old session and opens the new one`, `closing an inactive tab keeps the active stream`, `closing the active tab opens its neighbour`, `closing the last tab emits hide`, `Restart calls terminal_restart and keeps the tab in place`, `a failed restart shows the error and keeps the tab`, `Kill calls terminal_kill and keeps the tab as closed`, `a channel switch detaches only the active tab and recalls the remembered tab on return`. Replace the old `New Terminal Tab is visible, disabled, labelled Soon` test with the `+` tests, and update the existing open/close/restart tests to the session-ID commands.
- [ ] **Step 2: Run to confirm failure.** `pnpm --dir web test -- src/components/terminal/__tests__/TerminalPanel.test.ts`. Expected: FAIL.
- [ ] **Step 3: Implement the panel.** Keep one xterm. `start(channelId)` loads `terminalTabs`; with none it calls `terminalCreate`; otherwise it opens `pickActive(tabs, recallActive(channelId))`. State: `tabs` (list of `TerminalTab`), `activeId`. Switching detaches the current `attached` session and re-opens the target through the existing queue and `generation` guard. `+` calls `terminalCreate`, appends the tab, and activates it. Close calls `terminalClose`, removes the tab, then opens `nextActive` or emits `hide`. Restart and Kill use `terminalRestart`/`terminalKill` on the active tab. Call `rememberActive` on every activation. Header cwd follows the active tab. Remove the planned `TODO` and `· Soon` text.
- [ ] **Step 4: Update docs.** In `docs/project-channel-status.md` Terminal section: tabs replace "one shell session"; add the tab rules (cap 8, naming, close-last hides, Kill keeps the tab, Restart keeps position); delete the "Not built" line.
- [ ] **Step 5: Run the checks.** `pnpm --dir web test`, `pnpm --dir web check`, and `cargo test -p discard-app` with Claude vars unset. Expected: pass. Known unrelated: `MoveRequestDetails.vue` format failure in `check` on main; the `ChannelList.navigation-actions` timeout under load passes alone.
- [ ] **Step 6: Commit.** `git commit -m "feat: terminal tabs in the panel"`
