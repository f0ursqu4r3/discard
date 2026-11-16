# Project window

Date: 2026-11-16

## Goal

A project can open in its own window. The project window shows that project only. It has no project rail.

The main window and each project window hold their own selection. A change of project or channel in one window does not change another window.

## Owner decisions (2026-11-16)

- A project opens in its own window, with no project rail.
- Window state is per-window. A window that only hides the rail and shares one selection is rejected.
- The build uses per-window views in one core (approach A below).
- The main window closes alone when another app window is open. Agents continue. Discord works this way.
- An action in a project window that leads outside its project opens the target in the main window and focuses the main window.

The other decisions are the designer's calls. They are marked **(call)** where the owner is likely to have an opinion.

## Terms

- **Main window**: the window with the label `main`. It has the project rail.
- **Project window**: a window that shows one project. Its label is `project-<project ID>`.
- **App window**: the main window or a project window. The settings window is not an app window.
- **View**: the selection of one app window: the project, the selected channel, the open thread, the branch tree, and the layout of that window.
- **Channel pool**: the loaded channels of all projects, with their agent sessions. All views share it.
- **Focused view**: the view of the app window that had focus last.

## How it works today

- `AppCore` holds one `State` (`crates/discard-app/src/desktop/core.rs`). `State.active` is the selected channel. `State.parked` holds the other loaded channels, of all projects. A project switch keeps their agent sessions.
- The selection is single: `project`, `project_id`, `project_path`, `active`, `open_thread`, `branches`, `repositories`, `collapsed`, `current_branch`, `project_busy`. `AppCore.branch_source` and `AppCore.branch_generation` are single too.
- The pump (`pump.rs`) builds one snapshot. `WebEmit` (`mod.rs`) sends `discard://snapshot`, `discard://history`, and `discard://thread-history` to all windows.
- Commands read and change the single selection. Most do not know which window called them.
- Menu actions, close handling, saved bounds, and the terminal test for the label `main` (`actions.rs`, `mod.rs`, `terminal.rs`).
- `main.ts` mounts `SettingsApp` for `#/settings/…` and `App` for all else. The capability file lists the windows `main` and `settings`.

## Approaches

| | Approach | Trade-off |
|---|---|---|
| A | Per-window views in one core. One channel pool. Each command acts on the view of the calling window. The pump sends each window its own snapshot. | One agent session per channel. Two windows can show one channel and stay in sync. Large refactor of `core.rs` and the commands. |
| B | One core per window, with a shared store. | Small change to `core.rs`. A channel loaded in two cores gets two agent sessions, so a project must be open in one window only, and running turns must move between cores. Quit, cancel-all, plugins, and settings need a registry of cores. |
| C | One app process per project. | Two processes write one store and one state file with no coordination. Dropped. |

The owner chose A. B moves the hard part to the hand-off of running agents. A has one source of truth.

## Goals

- Open a project in its own window from the project menu.
- Each app window selects its project and channel independently.
- One agent session per channel, in all cases.
- The main window behaves as it does today.

## Non-goals

- Project windows that open again at the next start.
- Saved bounds for project windows.
- A window for direct messages or for one channel.
- Two windows for one project. A second main window.
- A drag of a project out of the rail to make a window.
- A single-instance check.
- Tray features other than the menu of decision T4: no unread badge, no list of project windows, no notifications.

## Anti-goals

- Two agent sessions for one channel.
- A window that follows the selection of another window.
- A closed project window that stops an agent.
- A snapshot, history, or thread transcript that arrives in the wrong window.

## Decisions

### Open and close

1. The project menu gets **Open in New Window**, directly above **Reveal in Finder**. It is an implemented action with the ID `open-window`. It is enabled at all times. The project menu mounts on rail tiles only (`RailProjectTile.vue`), so the action shows in the main window only.
2. One project has at most one project window. If the window exists, the action shows and focuses it.
3. The project window opens with the channel that the store saves for the project (`project_channel_or_fallback`), the same channel a rail click opens.
4. The window title is the project name. A rename of the project updates the title.
5. The window uses the options of the main window: overlay title bar, hidden title, traffic light position, `dragDropEnabled: false`. The start size is 1100 × 750 logical points. The position is the position of the main window plus an offset of 32 points on each axis.
6. A close of an app window closes that window only. It asks no confirmation. It stops no agent. It unloads no channel. This applies to the main window and to project windows.
6a. A close of a project window drops its view. A close of the main window keeps the `main` view and its selection.
7. A close of the last app window does not end the app when the app runs in the background (see Tray icon). Agents continue. If the app does not run in the background, that close follows the Quit rule, as the close of the main window does today: it asks the quit confirmation if an agent runs, then the app exits.
7a. Today the close handler tests for the label `main`, calls `cancel_all()`, and relies on `main` being the last window (`mod.rs`, `plan.rs` `close_plan`). The `ExitRequested` handler in `mod.rs` applies the Quit rule when the last window closes. Both must follow decision 7.
7b. A new action **Main Window** in the native Window menu shows and focuses the main window. It opens the main window if it is closed, with the saved bounds and the selection of the `main` view. On macOS a click on the Dock icon does the same when the main window is closed.
7c. The builder chooses whether a closed main window is hidden or destroyed. A hidden main window is not an app window for decisions 6 and 7.
8. **(call)** Project windows do not open again at the next start.
9. **Remove Project…** closes the project window of that project.

### Tray icon

T1. The app runs in the background when no app window is open. Agents continue. The owner asked for this (2026-11-16); Discord works this way.
T2. Windows and Linux get a tray icon. It exists while the app runs. This needs the `tray-icon` feature of the `tauri` crate.
T3. **(call)** macOS gets no menu bar icon. The Dock icon is the way back: a click on it shows the main window (decision 7b).
T4. The tray menu has **Main Window** (the action of decision 7b), a separator, and **Quit Discard**. On Windows a left click on the icon also shows the main window. On Linux the tray gives no click event, so the menu is the only path.
T5. The tray icon uses the selected app icon (`app_icon.rs`). It changes when the setting changes.
T6. **Quit Discard**, the Quit menu item, and the quit shortcut follow the Quit rule. If no app window is open and the rule asks for confirmation, the main window opens to show it.
T7. **(call)** General settings get **Keep running in the background when all windows are closed**. The default is on. When it is off, decision 7 applies the Quit rule to the last app window. Reason: on some Linux desktops (GNOME without an extension) the tray icon is created but not shown, and the app cannot detect this. Without the setting, that user has no way back to a running app.
T8. If the tray icon cannot be created (for example a missing appindicator library on Linux), the app does not run in the background in that session, and the setting shows a line that says why.
T9. The app has no single-instance check today. A second start of the app while it runs in the background starts a second process. A single-instance check is a non-goal of this spec.

### Content of a project window

10. A project window shows the same layout as the main window without the project rail: channel list, conversation, member list, thread panel, terminal, and context panel. The compact layout applies in the same way.
11. A project window has no project switch, no direct messages entry, and no **Add Project** control. These controls are absent, not disabled. They are not planned for this window.
12. The toolbar shows the project name, as the main window does.
13. All menus and dialogs of the project and its channels work in a project window as they do in the main window.

### Views

14. `State` keeps one channel pool and a map of views, keyed by window label. `active` and `parked` become one map of channels. A view names its selected channel by ID.
15. A view holds: project (ID, name, path), selected channel ID, open thread, branch tree and its repositories, collapsed rows, current branch, branch source, branch generation, and the busy guard for navigation.
16. The `main` view always exists. A project view exists while its window exists.
17. A view of a project window is fixed to its project. It never selects a channel of another project or a direct message.
17a. An action in a project window that leads outside its project selects the target in the `main` view, then shows and focuses the main window. It opens the main window if it is closed. The view of the project window does not change. The known cases are: **Agents** in the project dropdown of the channel list (`show_friends`), **Message** on an agent profile (`message_agent:`), `discard://agent-profile` with no agent ID, and a message source link to a channel of another project.
17b. The core enforces decision 17: a select command for a project view with a target outside its project routes to the `main` view. The web does not need a separate check for each control.
18. **(call)** The main window can select a project that also has a project window. The two views are independent. The rail click does not focus the project window.
19. Two views can select the same channel. Messages, the running turn, approvals, and prompts are channel state, so both windows show the same content.
20. A view change never unloads a channel that another view selects. `replace_project` and `clear_project` must follow this rule; today `replace_project` clears `parked`.
21. A guard that protects shared data stays app-wide (for example archive, write approval, `changing_channels`, `changing_agents`). A guard that protects the selection of a view becomes per-view. `project_busy` does both today: it guards navigation, and it guards write approval, project removal, folder change, and worktree removal. It splits into a per-view navigation guard and an app-wide guard.
21a. The snapshot of a project view lists only the channels and thread approvals of its project (`channels`, `threadApprovals`). The snapshot of the main view lists all loaded channels, as today.
21b. `choose_project_folder` reads `store.selected_project()` after a folder change. That is the project of the main view. It must use the project ID of the calling view.
22. When a channel becomes unavailable (archive, delete, agent removal), each view that selects it opens `#general` of the same project. This is the current rule, applied to every view.

### Saved state

23. Read markers stay per channel and shared. A read in one window marks the channel read in all windows.
24. The main view saves `selected_conversation`, bounds, panel widths, and member list visibility to `state.toml`, as today.
25. A project view starts with the saved panel widths and member list visibility. Its changes apply to that window only. They are not saved.
26. A channel selection in a project view saves the selected channel of the project in the store (`select_project_channel`). It does not change `selected_conversation` or the selected project of the store. Those belong to the main view.

### Commands and events

27. A command that reads or changes the selection takes the calling `tauri::WebviewWindow` and uses the view with that label. A call from a window that has no view (the settings window) uses the focused view, with one exception: a Settings action that opens a conversation (for example `memory_fresh_conversation`) selects it in the `main` view and shows the main window, as `show_conversation` does today.
28. A command that names its channel or project explicitly does not change.
29. `discard://snapshot`, `discard://history`, and `discard://thread-history` go to one window each, with `emit_to`. Each window receives the snapshot of its own view. The pump tracks the history and thread revisions per view.
30. `discard://conversation-opened` goes to the window whose view changed. `discard://toggle-terminal` and `discard://agent-profile` go to the focused app window.
31. `discard://conversations`, `discard://projects`, `discard://settings`, `discard://memory`, and `discard://shortcuts` stay broadcast.
32. The capability file allows the project windows (`project-*`) the same permissions as `main`.

### Menu, shortcuts, and focus

33. The app tracks the focused view from window focus events.
34. The native menu reads its context (`turn_running`, `archived`) from the focused view. It rebuilds when the focused view changes.
35. Menu actions that act on a window (Close Window, Minimize, Zoom, Toggle Terminal, Toggle Member List, Cancel Turn, agent profile) act on the focused app window and its view.
36. The quit and relaunch confirmation stays app-wide. It shows in the focused app window only, because the main window can be closed.
37. Settings pages that read the active channel (`workspaces.rs`, `agents.rs`) read the focused view.

### Terminal

38. The terminal works in project windows. The `main_window_only` guard is removed. Each command checks that the caller is an app window.
39. A terminal session belongs to a channel, as today, and has one listener. **(call)** When a second window attaches to the terminal of the same channel, the session moves to that window. The first window shows the notice `This terminal is open in another window.` with a **Show Here** button that attaches again.

### Plugins

40. Plugin slots render per view, from the slot state of that view. The slot cache holds one entry (`slots.rs`), so each view gets its own cache. One shared cache would render the Lua slots again on each pump tick.
41. `set_ui_state` and the project source of plugin memory (`main.rs`) read the focused view.
42. Plugin-drawn UI needs no separate work. The project window mounts the same components.

### Web

43. The project window loads `index.html#/project/<project ID>`. `main.ts` mounts `App` for it. `App` reads the window mode once at start.
44. In project mode `App` and `NavigationPanel` do not mount `ServerRail` and do not load the rail layout. In the compact layout the navigation strip holds the user bar only.
45. The web store needs no change to its event names. Each window receives its own events.

## Constraints

- AGENTS.md: the new menu item is implemented, so it has no `Soon` label and no `TODO`. No no-op handler.
- Web tests go in `__tests__` beside the code.
- The existing Rust and web tests must pass after each slice.

## Build order

1. Refactor `State` to a channel pool and a `main` view. No behavior change. All tests pass.
2. Make commands window-scoped. Send snapshot, history, and thread history per window.
3. Add the project window: open, focus, close, the web project mode, and the menu item. Add the **Main Window** action.
3a. Add the background mode: the tray icon, the Dock click, the General setting, and the close rule of decision 7.
4. Add focus tracking: native menu context, menu actions, plugins, and settings pages.
5. Enable the terminal in project windows, with the move notice.
6. Handle project removal, project rename, unavailable channels in all views, and actions that lead outside the project.

## Tests

- Core: two views select different channels; a switch in one view does not change the other; an update to a shared channel shows in both snapshots; a dropped view cancels no turn and unloads no channel; a project view rejects a channel of another project.
- Pump: each window receives the snapshot and history of its own view only.
- Plan: a close of an app window that is not the last is allowed with a running turn and cancels nothing. A close of the last app window is allowed and cancels nothing when the app runs in the background. It follows the Quit rule when the setting is off or the tray icon is unavailable, for the main window and for a project window.
- Manual, tray: close all windows with a turn running; the turn continues; **Main Window** in the tray (Dock click on macOS) shows the main window; **Quit Discard** asks for confirmation.
- Core: a select in a project view with a target outside its project changes the `main` view and not the project view.
- Web: the project menu has **Open in New Window**; `App` in project mode has no rail, no direct messages entry, and no **Add Project**.
- Manual: `cargo tauri dev`, two projects in two windows, one turn running in each.

## Left to the builder

- Names of the types and fields.
- The revision scheme (one revision per view, or one shared revision with a per-view compare).
- How the window label encodes a project ID, if the ID has characters that a Tauri label does not allow.
- The exact wiring of focus events on each platform.
