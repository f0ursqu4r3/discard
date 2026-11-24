# Terminal tabs design

Date: 2026-11-23. Size: project. Status: approved.

## Intent

A project channel can run several shells at once. The user runs a dev server in one tab and a test run in another. Every tab starts in the worktree of the channel.

## Goals

- A channel has up to 8 terminal tabs. Each tab is one shell session.
- Tabs keep running when the user switches channel, switches tab, hides the panel, or reloads the window. This is the existing "Sessions stay" rule, applied per tab.
- Every path that ends a channel session ends all tabs of that channel: archive, worktree removal, project removal, a missing folder, and app exit.
- The planned "New Terminal Tab" item becomes real and its `TODO` goes away.

## Non-goals

- Split panes, tab rename, drag to reorder, a folder picker, and tabs that follow the running program.
- New keyboard shortcuts. The terminal still passes every key except Toggle Terminal and the Cmd keys to the shell.
- Tabs that survive an app restart.

## Anti-goals

- A tab list that the webview and the backend can disagree on. A running dev server with no tab is a failure.
- A new way for agents, MCP tools, or plugins to reach a terminal. The module stays user-only. Discard still never logs terminal input or output.

## Approaches

- **A. The backend owns the tab list. (Chosen.)** Each session has its own ID. The registry knows which channel and which position each session has. Commands take a session ID. The panel asks the backend for the tabs of a channel.
- **B. The webview owns the tab list.** The backend stays keyed by channel plus a tab number, and the webview saves the list in local storage. Rejected: a reload or a second window can lose a running shell.

## Design

### Backend (`crates/discard-app/src/desktop/terminal.rs`)

- `Terminals` keys sessions by session ID. Each entry holds the channel ID, the project ID, an order number, and a tab number. The tab number is the highest number among the open tabs of the channel, plus one (1 for the first tab). The name is "Terminal N".
- `Terminals::tabs(channel)` returns the sessions of a channel in order. `close_channel` ends every session of the channel. The other close paths keep their filters.
- A channel with 8 sessions refuses a new one with the error "A channel can have 8 terminals."
- Commands:
  - `terminal_tabs(channel_id) -> Vec<TerminalTab>`. A tab has `sessionId`, `number`, `cwd`, `running`, `exitCode`, `signal`. This replaces `terminal_status`.
  - `terminal_create(channel_id, cols, rows, on_event) -> OpenedTerminal`. It starts a shell in the worktree of the channel with `cwd::for_channel`, adds a tab, and attaches the window.
  - `terminal_open(session_id, cols, rows, on_event) -> OpenedTerminal`. It re-attaches an existing tab. The owner rules stay: the new window replaces the old listener, and the old window hears `moved`.
  - `terminal_write`, `terminal_resize`, `terminal_detach`, `terminal_close` take `session_id` in place of `channel_id`.
  - `terminal_kill(session_id)`. It tells the listener "The session was killed.", ends the shell, and keeps the tab.
  - `terminal_restart(session_id, cols, rows, on_event) -> OpenedTerminal`. It ends the shell and starts a new one in the same position with the same number.
- `OpenedTerminal` and `TerminalEvent` keep their shape.

### Webview

- `web/src/bridge/terminal.ts` mirrors the commands above.
- `terminal-state.ts` adds tab list helpers: pick the active tab, the tab after a close, and the rule that a channel with 8 tabs disables `+`.
- `TerminalPanel.vue` gets a tab strip above the header. Each tab shows its name and a close button. A `+` button replaces the planned item and is disabled at 8 tabs, with the reason in its title.
- One xterm instance shows the active tab. A tab switch detaches the old session, then opens the new one and replays its kept output (about 1 MiB). The scroll position of a tab is not kept.
- The panel opens the first tab itself when the channel has none, as it does today. A new tab becomes active.
- The active tab of each channel is kept in memory for the window. After a reload the first tab shows.
- Closing a tab ends its shell. Closing the last tab hides the panel. The next toggle starts a fresh tab.
- Restart and Kill act on the active tab. Kill ends the shell but keeps the tab in the strip as a closed tab (new `terminal_kill(session_id)`). The user closes it or restarts it.

### Docs and tests

- `docs/project-channel-status.md` changes: "Each project channel can have one shell session" becomes tabs, and the "Not built" line goes.
- Rust tests: the registry with several sessions per channel, the cap, `close_channel` ending all tabs, numbering after a close, and restart keeping position.
- Web tests: tab strip actions, the cap, close-last-tab hides, tab switch detaches and re-opens, and the `+` item no longer says Soon. The old test "New Terminal Tab is visible, disabled, labelled Soon" changes to match.

## Calls the builder settles

- Whether a tab with an ended shell shows a marker in the strip. Default: a muted name.
- The exact error text for a refused session when the folder is gone. Keep the current text.
