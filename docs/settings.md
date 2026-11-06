# Settings, menus, and key bindings

Discard has a settings file, a native menu bar, seven default key bindings, and a Settings window. Rust owns all of
them. Plugins can add items to the Plugins menu and context menus. They cannot change Rust items, keys, or Settings pages.

## The settings file

The file is `settings.toml` under `$DISCARD_HOME` (default `~/.discard`).

```toml
version = 1

[general]
default_provider = "claude"       # claude or codex
confirm_while_working = true      # ask before Quit or Safe Mode while a turn runs

[appearance]
theme_source = "default"          # default, ghostty, or custom
theme = "light:Catppuccin Latte,dark:Catppuccin Mocha"   # used when theme_source is "custom"
ignore_plugin_themes = false      # true: plugin theme tokens have no effect

[keybindings]
zoom = "cmd-shift-k"
```

| Key | Type | Default |
| --- | --- | --- |
| `version` | integer | none. Discard writes `1` the first time it writes the file. |
| `general.default_provider` | `"claude"` or `"codex"` | `"claude"` |
| `general.confirm_while_working` | `true` or `false` | `true` |
| `appearance.theme_source` | `"default"`, `"ghostty"`, or `"custom"` | `"default"` |
| `appearance.theme` | A Ghostty `theme` value: `Name` or `light:X,dark:Y` | `"light:Catppuccin Latte,dark:Catppuccin Mocha"` |
| `appearance.ignore_plugin_themes` | `true` or `false` | `false` |
| `keybindings.<action>` | Shortcut string, or an empty string to unbind an optional action | Action default |

Discard never creates the file at startup. It creates the file on the first change you make in the Settings window.
The folder already exists after startup.

### A bad file never stops startup

Discard reads each key on its own. A bad key falls back to its default. The rest of the file still counts.

| Problem | Result |
| --- | --- |
| Missing file | All defaults. No problem is reported. |
| Wrong type for a key (for example `confirm_while_working = "yes"`) | That key uses its default. One problem is reported. |
| Invalid keybinding or conflict | That action uses its default. One problem is reported. |
| `keybindings` is not a table | All shortcuts use their defaults. One problem is reported. |
| Unknown provider name | `default_provider` uses `claude`. One problem is reported. |
| Unknown `theme_source`, or a `theme` that names no theme (empty, or only `light:X`) | That key uses its default. One problem is reported. |
| `general` is not a table | All `general` keys use their defaults. One problem is reported. |
| `version` is not `1` | The keys are still read. One problem is reported. This holds for a newer version and for any other number. |
| `version` is not an integer | One problem is reported. The keys are still read. |
| The file is not valid TOML or not valid UTF-8 | All defaults. One problem, for the key `file`, is reported. |
| The file exists but cannot be read (for example, no read permission) | All defaults. One problem, for the key `file`, is reported. |
| Unknown keys and unknown tables | Ignored. No problem is reported. |

Discard writes each problem to the log at startup (`settings issue`). The General page lists the problems in a box
titled "Problems in settings.toml".

### How Discard writes the file

- **Comments and unknown keys stay.** Discard edits the TOML text in place. Comments, spacing, and keys it does not
  know stay as they are. A changed value keeps the comment on its line.
- **One change at a time.** Each click in the Settings window validates one change and writes the file. An unknown
  provider name is refused, and the file stays as it was.
- **Whole-file replace.** Discard writes a temporary file (`settings.toml.tmp`) next to the real one, then renames it
  into place.
- **A newer `version` stays.** Discard writes `version = 1` only if the file has no `version`. It never lowers or
  changes a version that is there.
- **A non-table `general` is replaced.** If `general` is not a table (for example `general = 3`), the first write
  replaces it with a `[general]` table that holds your change.
- **Backup of an unparseable file.** If the file could not be parsed, the first write copies the bad text to
  `settings.toml.invalid`, then writes a new file from defaults plus your change. Each later backup of a bad file
  replaces the old `settings.toml.invalid`.
- **Edits outside Discard.** While the Settings window is open, it checks the file every 0.7 seconds. It reads the
  file again when the bytes changed, and the page updates. Before it writes a change, it reads the file again, so a
  hand edit is not lost. With the Settings window closed, Discard does not watch the file, but it reads the file
  again before it runs each menu item, key, or Rust-drawn button. So a hand edit to `confirm_while_working` applies at
  the next action. Opening the Settings window also reads the file again.

## Provider precedence

Discard picks the provider for the session in this order:

1. The `--provider` flag.
2. The `DISCARD_PROVIDER` environment variable. An empty value counts as not set.
3. `general.default_provider` in `settings.toml`.

An unknown name in the flag or in `DISCARD_PROVIDER` stops startup with an error that lists the valid names.

The setting applies at the next launch. A running Discard never switches provider. A project has one channel for each
provider, so a new provider starts in that provider's channel. Discard creates the channel if it does not exist.

## The menu bar

Rust builds the menu bar. Discard builds it again when a turn starts or ends, or when the number of waiting reviews
changes. The Safe Mode check mark reflects the mode the app started in.

| Menu | Item | What it does |
| --- | --- | --- |
| Discard | Settings… | Opens the Settings window on the General page. |
| Discard | Services | The standard macOS Services submenu. |
| Discard | Hide Discard | Hides Discard. |
| Discard | Hide Others | Hides every other app. |
| Discard | Show All | Shows every hidden app. |
| Discard | Quit Discard | Quits. May ask first. See "Safe mode, relaunch, and confirm". |
| File | New Channel… | Greyed, "Soon". |
| File | Archive Channel… | Archives the current channel when no turn runs. |
| File | Close Window | Closes the focused window. Closing the main window quits, so it may ask first. Closing Settings never asks and does not quit. |
| File | Cancel Turn | Cancels the running turn. Greyed when no turn runs. |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All | Standard items. The OS and the text input handle them. |
| View | Switch Channel… | Greyed, "Soon". |
| Plugins | Review Plugins… | Opens the Settings window on the Plugins page. With waiting reviews, the label shows the count: "Review Plugins (2)…". |
| Plugins | Plugin Errors… | Opens the Settings window on the Plugins page. |
| Plugins | Safe Mode | Relaunches Discard with Safe Mode on or off. Has a check mark when Safe Mode is on. |
| Window | Minimize | Minimizes the focused window. |
| Window | Zoom | Zooms the focused window. |
| Help | Discard Documentation | Greyed, "Soon". |

### Why some items are greyed

An item is greyed "Soon" when its feature does not exist yet. These items have no handler. Three items are in this
state: New Channel…, Switch Channel…, and Discard Documentation. The list is one Rust constant
(`FUTURE_ITEMS` in `crates/discard-core/src/action.rs`). An item turns on when its feature is built.

Native menus show no tooltip. Each greyed item carries " (Soon)" in its label instead. Cancel Turn is greyed when no
turn runs, but it has no "Soon" tag.

## Default keys

Discard binds seven keys. The list is `default_keymap()` in `crates/discard-core/src/action.rs`.

| Key | Action |
| --- | --- |
| `Cmd+,` | Open Settings |
| `Cmd+Q` | Quit |
| `Cmd+.` | Cancel Turn (does nothing when no turn runs) |
| `Cmd+W` | Close Window |
| `Cmd+H` | Hide Discard |
| `Alt+Cmd+H` | Hide Others |
| `Cmd+M` | Minimize |

The webview adds no window or app keys of its own, so Discard binds Cmd+W, Cmd+H, and Cmd+M itself. Discard binds these keys
before it builds the menu bar, so the menu items for them show their keys: Settings…, Hide Discard, Hide Others, Quit
Discard, Close Window, and Minimize. Cancel Turn shows its key only while a turn runs. When no turn runs, its item
carries a placeholder action with no key. Show All and Zoom have no key. An invalid binding is logged and skipped.

The edit keys Cmd+C, Cmd+V, Cmd+X, Cmd+A, and Cmd+Z stay with the OS and text inputs. They are reserved.
Cmd+Q and Cmd+, remain reserved for Quit and Settings even if you move those actions. Cmd+W, Cmd+H, and Cmd+M are locked.
Quit, Settings, and Cancel Turn need a shortcut. You can change other actions or unbind them. A conflict blocks the
change and names the action. Key changes apply at the next launch. Plugins cannot register shortcuts.

## The Settings window

- **Open it** with Discard > Settings…, `Cmd+,`, or the **Settings** button in the channel list of the main window.
  Plugins > Review Plugins…, Plugins > Plugin Errors…, the **Plugins** button, the banner **Review** button, and the
  **Details** button on the problem line open it on the Plugins page.
- **One window.** If the window is open, Discard brings it to the front and switches its page. It never opens a
  second window. Closing Settings does not quit the app.
- **Sidebar.** The sidebar lists ten pages: General, Appearance, Agents, Providers, Plugins, Memory, Workspaces,
  Keybindings, Logs, Advanced.
- **Search.** The box above the list filters pages. A page stays if its title or one of its field labels contains the
  text. Case and spaces do not matter, so "rollback" finds "Roll back". "No matching settings" shows when nothing
  matches.
- **Pages:** All ten pages are available.

### General page

- A "Problems in settings.toml" box, when the file has problems.
- **Default provider.** One button for each provider. A note says "Applies at next launch".
- **Confirm while working.** A switch. A note says it confirms Quit and the Safe Mode relaunch while a turn runs.
- **Reopen last channel.** Greyed, "Soon". It has no setting yet.
- A write error shows inline, and stays until the next change succeeds.

### Appearance page

From top to bottom:

- **Theme source.** Default, Ghostty, or Custom. A line under the buttons says what each one does.
- **Themes.** Shown only for Custom. Pick a light theme and a dark theme from searchable lists. Discard switches
  between them with the system appearance. Turn on "Use one theme for both" to pick one theme. The lists group
  themes as Yours, Ghostty, and Bundled. A name in an earlier group hides the same name in a later one.
- **The active theme.** Its name and origin, with an "Edit a copy" button. The button copies the theme to
  `$DISCARD_HOME/themes` and opens the copy in the default editor. It never overwrites an existing copy. A theme
  given as an absolute path opens as it is. Under the name are the theme file path and the Ghostty config path.
- **Swatches.** The 16 palette colors, then background, foreground, and selection.
- **Ignore plugin themes.** Keeps the theme colors above, whatever plugins ask for.
- **Plugin overrides.** Each theme token a plugin sets, with the plugin that wins it. Later plugins win.
- Theme problems and write errors show at the top of the page.

### Providers page

Each provider card shows setup status, the install or login command when needed, all capabilities, and hook enforcement.
Missing capabilities show as "No". **Re-check** runs the setup check again. Discard does not read or store credentials.

### Agents page

Create agents with a generated editable name, optional avatar, connected provider, exact model ID, and optional personality. Multiple agents can use the same model. Personality stays separate from learned memory. Review the changes before save. A looser permission policy or enabling `plugin.write` requires explicit confirmation. Unsupported choices show the reason.

Saved edits apply without restarting. A running response keeps its original configuration; changes apply to the next response. Archive preserves conversations and memory and supports restoration. Permanent deletion offers separate choices for conversations and private memory. Global and project memory stay saved. Retained deleted-agent conversations remain readable.

### Plugins page

Rust draws this page. A plugin cannot hide or replace it. From top to bottom:

- **Safe mode row.** Shows "Safe mode: user plugins disabled" and a **Relaunch normally** button. Only in Safe Mode.
- **Plugin problems.** A list of plugin errors, plugins disabled because their files changed outside review, failed
  plugins, a signing-key problem, and, in Safe Mode, the Safe Mode line.
- **A notice**, with a **Dismiss** button. It shows the last error from an approve or roll back.
- **Waiting for your review.** One card for each review. The card shows "New plugin" or "Update", the description,
  the capabilities the plugin asks for, changes since the approved version, a full-local-access warning when the
  plugin can run programs, the code diff, a "Trust future edits" toggle when the plugin qualifies, and **Reject** and
  **Approve and run** buttons. The review rules are in [plugins.md](plugins.md).
- **Installed plugins.** One row for each user plugin with its state (running, failed, disabled because files changed
  outside review, disabled in Safe Mode), a "trusts future edits" tag, and a **Roll back** button. The bundled
  interface is a plugin too, but it is not listed.

### Keybindings page

Record, reset, or unbind shortcuts. Locked actions cannot be edited. Changes apply at the next launch. Use **Relaunch** to apply them.

### Memory page

Browse all global, agent, project, and agent-project memory, including orphan domains. Inspect each item's kind, timestamps, source conversation, version, and history. Edit text, move the item to another domain, or use **Forget** to remove it from retrieval. Moving to a shared domain explicitly shares the item. Pending proposals have **Approve** and **Reject** controls.

**Reveal item file** selects the actual item document. **Import file edits and reload** imports external body edits as a new version and resolves the file conflict before another edit. Reload replaces the inspector draft; copy that draft first if needed.

The conversation Memory panel shows applicable items. Automatic saves and approved updates appear beside their saved response with versioned Undo. Forget and Undo of a new save pause automatic capture. The conversation provides explicit Resume and fresh-conversation actions. Forget does not erase transcript messages. See [Saved memory](memory.md) for domain boundaries and pause behavior.

### Workspaces page

Browse projects, channels, workspace modes, and worktree paths. Missing folders and archived channels are marked. Use **Reveal** to open a folder. This page is read-only.

### Logs page

Discard writes redacted JSON lines to stderr and `$DISCARD_HOME/logs/discard.log`. It rotates at 5 MiB and keeps four older files. Filter by level, component, or session, turn, or plugin ID. Follow scrolls to new lines. The log is read-only.

### Advanced page

Reset settings saves a backup at `settings.toml.bak`. Rebuild memory index reads the memory files again. Clear logs removes old log files and empties the active file. Each action asks for confirmation.

## Context menus and archive

Right-click a message to copy its text, a code block to copy its code, or a channel row to copy its name or archive it.
Right-click a diff to copy it, a tool call to copy its input or output, or an agent to copy its name or open Agent Settings. Every project icon has a context menu. It can open Project Settings, create a channel, reveal the folder in Finder, remove the project after confirmation, or copy its name, path, or ID. Opening a menu does not select the project. Settings and channel creation select the target project through the existing idle-agent guard. Copy, Reveal, and Remove use the clicked project's identity.

Planned project actions remain visible and disabled with a Soon label. Mute, Notifications, Move to, and Copy submenus remain openable. Missing behavior has TODO comments in the menu model. This rule applies to all new and changed UI actions; see AGENTS.md. The live streaming row has no context menu. Plugin items follow the built-in items and show the plugin name. Project menus also open with Shift+F10 or the Menu key while a project icon has focus.

Archive is blocked while a turn runs. If removing a worktree can lose work, Discard lists the items and asks for
confirmation. Cancel keeps the channel and worktree. After archive, the composer is locked. **Relaunch** starts a new
channel. The worktree branch is kept.

## Safe mode, relaunch, and confirm

Plugins load once at startup. So Safe Mode needs a new process. Plugins > Safe Mode and **Relaunch normally** do this:

1. Discard starts a detached `/bin/sh` child with no input or output. Its error output is the same as Discard's, so
   the relaunched process keeps writing its logs there. The child polls until the old process id is gone.
2. If a turn runs, Discard sends Cancel to the engine. Then Discard quits.
3. The child starts the same executable again, with the original arguments. It removes every `--safe-mode`, and adds
   one when the target is Safe Mode on. Other arguments, such as `--provider` and `--builder`, stay.

The child waits for the old process to exit, so the two processes do not share the database or the memory runtime's
TCP port. The child does not wait for provider child processes. See "Limits you should know".

If Discard cannot start the child, it does not quit. The main window shows "Could not relaunch Discard:" and the
reason.

### The confirm notice

When a turn runs and `confirm_while_working` is `true`, Quit and the Safe Mode relaunch do not act at once. The main
window comes to the front and shows a notice with **Cancel** and **Confirm** buttons:

- "The agent is working. Quit Discard and stop the turn?"
- "The agent is working. Relaunch in safe mode and stop the turn?"
- "The agent is working. Relaunch normally and stop the turn?"

Closing the main window (the red close button, `Cmd+W`, or File > Close Window) follows the Quit rule and shows the
Quit notice. Closing the Settings window never asks.

Confirm sends Cancel to the engine and ends the process. Cancel clears the notice. With
`confirm_while_working = false`, or with no turn running, the action runs at once. Cancel Turn, opening Settings,
Hide, Minimize, and Zoom never ask.

## What stays in the main window

Rust draws these in the main window. Plugins cannot hide or replace them.

- **Review banner.** "Plugin code is waiting for your review", with the plugin names and a **Review** button. It
  shows while any review waits.
- **Problem line.** "N plugin problems", with a **Details** button. It counts each problem line from the Plugins page,
  plus one for a notice. In Safe Mode, the Safe Mode line counts as a problem.
- **Confirm notice.** See above.
- **Plugins and Settings buttons** in the channel list. The Plugins button shows "Plugins (N to review)" when reviews
  wait.

The Details and Review buttons open the Plugins page in Settings. The plugin list itself is no longer in the main
window.

## Closing the main window

Closing the main window quits the app on every platform, and Settings closes with it. Discard has one main window,
and the engine runs for it. A Mac app usually stays open with no window. Discard does not. While a turn runs and
`confirm_while_working` is `true`, the window stays open and shows the Quit notice first.

## Limits you should know

- The default provider applies at the next launch.
- Relaunch works on unix only. On other systems the code starts the new process at once, without waiting for the old
  one to exit. Windows is out of scope for v1.
- The relaunched process has no input or standard output. `relaunch.rs` sets them to null. Its stderr is inherited,
  so the relaunched Discard writes its logs where the first one did.
- The confirm notice comes from the Quit action (`Cmd+Q` and Quit Discard), closing the main window, and the Safe
  Mode relaunch. Discard has no hook that can stop a Quit from the Dock or the system, so those do not ask. They end
  the process. Discard sends Cancel to the engine when the app quits, but the process gets only a short time to run quit handlers.
- Provider processes can outlive Discard. On macOS a quit ends the process with `exit()`, so no cleanup code runs.
  Discard sends Cancel to the engine before it quits, on a best-effort basis only. A quit or relaunch during a running
  tool call can leave provider child processes running, and a relaunched Discard can start while they still run.
- The confirm notice does not clear when the turn ends on its own. It keeps its text until you press Cancel or
  Confirm. A second request replaces the first.
- After the first write to a file that could not be parsed, the "file" problem disappears. Discard does not tell you
  that the old text is now in `settings.toml.invalid`.
- Key changes need a relaunch. The Workspaces page is read-only. Agent and memory changes apply without a relaunch.
- Plugin-declared settings are not built. `settings.toml` holds `version`, `general`, `appearance`, and `keybindings`. Unknown keys stay in the
  file, but nothing reads them.
- Greyed menu items have no tooltip.
- The menu bar, the keys, the confirm notice, the relaunch, the single Settings window, and the Plugins page were
  built and logic-tested. The manual GUI checks have not been run.

## Themes

Discard colors the windows with a Ghostty theme. A plugin theme goes on top.

- **Source.** `appearance.theme_source` picks the theme. `default` uses the bundled pair. `ghostty` reads your Ghostty
  config (`$XDG_CONFIG_HOME/ghostty/config`, `~/Library/Application Support/com.mitchellh.ghostty/config`, and the
  `config.ghostty` names). `custom` uses `appearance.theme`. Discard does not follow `config-file =` includes.
- **Light and dark.** A `light:X,dark:Y` value picks the theme for the system appearance and switches when it
  changes. The window sets the `dark` class on the page from the brightness of the theme's background.
- **Theme files.** Discard reads the Ghostty format (`palette = N=#hex`, `background`, `foreground`,
  `cursor-color`, `selection-background`, `split-divider-color`). Colors can be `#rrggbb`, `rrggbb`, `#rgb`, or an
  X11 name. Inline keys in the Ghostty config replace the same keys in the theme file.
- **Lookup order for a theme name.** `$DISCARD_HOME/themes/`, then Ghostty's user folders, then the Ghostty app, then
  the themes bundled in Discard (see `THIRD-PARTY-NOTICES`). An absolute path also works.
- **Live reload.** While the main window is open, Discard checks `settings.toml`, the Ghostty config, the theme file,
  and the system appearance every 0.7 seconds. A change applies at once.
- **Plugin themes.** Plugin color tokens replace the matching Ghostty colors. `appearance.ignore_plugin_themes = true`
  turns them off. Plugins cannot change selection, caret, or diff colors. Approval cards and review screens read
  their palette from the theme you see, and keep their contrast rule.
- **Diff colors.** Added and removed lines in a plugin review use palette 2 and palette 1, through the contrast rule.
- **A theme that fails.** A missing theme, a missing Ghostty config, or a bad value falls back to the default pair.
  A bad line in a theme file is skipped. The General page lists each problem with the file and line.

## Window state

`$DISCARD_HOME/state.toml` stores the main window position and size and the channel and member list widths.
Discard restores bounds that are large enough and visible on a current display. Otherwise it opens the window centered.
Drag the dividers to resize the lists from 180 to 400 pixels. Each list defaults to 240 pixels.
Discard saves window changes and panel widths in this file.

The composer sends with Enter. Shift+Enter inserts a newline.
Plugin theme tokens `font_scale` and `spacing_scale` adjust text and spacing. Ignore plugin themes disables both tokens.
