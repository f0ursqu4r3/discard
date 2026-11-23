# Settings, menus, and key bindings

Discard has a settings file, a native menu bar, eight default key bindings, and a Settings window. Rust owns all of
them. Plugins can add items to the Plugins menu and context menus. They cannot change Rust items, keys, or Settings pages.

## The settings file

The file is `settings.toml` under `$DISCARD_HOME` (default `~/.discard`).

```toml
version = 1

[general]
default_provider = "claude"       # claude or codex
confirm_while_working = true      # ask before Quit or Safe Mode while a turn runs
keep_running_in_background = true  # true: the app goes on when the last window closes. Default false on Linux
desktop_notifications = true      # notify when an approval waits or a turn ends and no window is focused

[appearance]
theme_source = "default"          # default, ghostty, or custom
theme = "light:Catppuccin Latte,dark:Catppuccin Mocha"   # used when theme_source is "custom"
ignore_plugin_themes = false      # true: plugin theme tokens have no effect
mini_project_rail = false         # true: 48 px rail at the default scale
message_density = "comfortable"  # comfortable or compact
show_message_avatars = true       # false: remove the avatar column
app_icon = "classic"              # classic, landscape, graffiti, manga, cyberpunk, matrix, or synthwave

[keybindings]
zoom = "cmd-shift-k"
```

| Key | Type | Default |
| --- | --- | --- |
| `version` | integer | none. Discard writes `1` the first time it writes the file. |
| `general.default_provider` | `"claude"` or `"codex"` | `"claude"` |
| `general.confirm_while_working` | `true` or `false` | `true` |
| `general.keep_running_in_background` | `true` or `false` | `true`. On Linux: `false` |
| `general.desktop_notifications` | `true` or `false` | `true` |
| `appearance.theme_source` | `"default"`, `"ghostty"`, or `"custom"` | `"default"` |
| `appearance.theme` | A Ghostty `theme` value: `Name` or `light:X,dark:Y` | `"light:Catppuccin Latte,dark:Catppuccin Mocha"` |
| `appearance.ignore_plugin_themes` | `true` or `false` | `false` |
| `appearance.mini_project_rail` | `true` or `false` | `false` |
| `appearance.message_density` | `"comfortable"` or `"compact"` | `"comfortable"` |
| `appearance.show_message_avatars` | `true` or `false` | `true` |
| `appearance.app_icon` | `"classic"`, `"landscape"`, `"graffiti"`, `"manga"`, `"cyberpunk"`, `"matrix"`, or `"synthwave"` | `"classic"` |
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
| File | Close Window | Closes the focused window. The main window hides when another app window is open or the app runs in the background. The last app window follows the Quit rule only when the app does not run in the background, so it may ask first. Closing Settings never asks and does not quit. |
| File | Cancel Turn | Cancels the running turn. Greyed when no turn runs. |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All | Standard items. The OS and the text input handle them. |
| View | Switch Channel… | Greyed, "Soon". |
| View | Toggle Terminal | Shows or hides the terminal panel of the main window. Key: `` Ctrl+` ``. See [The terminal](project-channel-status.md#terminal). |
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

Discard binds eight keys. The list is `default_keymap()` in `crates/discard-core/src/action.rs`.

| Key | Action |
| --- | --- |
| `Cmd+,` | Open Settings |
| `Cmd+Q` | Quit |
| `Cmd+.` | Cancel Turn (does nothing when no turn runs) |
| `Cmd+W` | Close Window |
| `Cmd+H` | Hide Discard |
| `Alt+Cmd+H` | Hide Others |
| `Cmd+M` | Minimize |
| `` Ctrl+` `` | Toggle Terminal |

The webview adds no window or app keys of its own, so Discard binds Cmd+W, Cmd+H, and Cmd+M itself. Discard binds these keys
before it builds the menu bar, so the menu items for them show their keys: Settings…, Hide Discard, Hide Others, Quit
Discard, Close Window, and Minimize. Cancel Turn shows its key only while a turn runs. When no turn runs, its item
carries a placeholder action with no key. Show All and Zoom have no key. An invalid binding is logged and skipped.

The edit keys Cmd+C, Cmd+V, Cmd+X, Cmd+A, and Cmd+Z stay with the OS and text inputs. They are reserved.
Cmd+Q and Cmd+, remain reserved for Quit and Settings even if you move those actions. Cmd+W, Cmd+H, and Cmd+M are locked.

On Linux, `Cmd` is the Control key. Close Window is Ctrl+Shift+W and Minimize has no key, so the shell in the terminal
keeps Ctrl+W, Ctrl+M, and Ctrl+H. The Hide actions exist on macOS only.
Toggle Terminal is an optional action. The terminal does not send its key to the shell. The terminal also
passes the Cmd keys and every shortcut that the webview handles to the app. Other keys, including `Ctrl+C`, go to the shell.
Quit, Settings, and Cancel Turn need a shortcut. You can change other actions or unbind them. A conflict blocks the
change and names the action. Key changes apply at the next launch. Plugins cannot register shortcuts.

## The Settings window

- **Open it** with Discard > Settings…, `Cmd+,`, or the **Settings** button in the channel list of the main window.
  Plugins > Review Plugins…, Plugins > Plugin Errors…, the **Plugins** button, the banner **Review** button, and the
  **Details** button on the problem line open it on the Plugins page.
- **One window.** If the window is open, Discard brings it to the front and switches its page. It never opens a
  second window. Closing Settings does not quit the app.
- **Sidebar.** The sidebar lists nine pages: General, Appearance, Providers, Plugins, Memory, Workspaces,
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
- **Desktop notifications.** A switch in the Notifications card. See "Desktop notifications" below.
- A write error shows inline, and stays until the next change succeeds.

#### Desktop notifications

- A notification fires for an approval that waits for you. It also fires for a finished or failed turn.
- A notification fires only when no Discard window is focused.
- The title is `<channel> · <agent>`.
- The body is `Needs approval: <tool>`, `Finished`, or `Failed`. It never shows message text.
- A cancelled turn sends nothing. A sub-agent thread turn sends nothing.
- A turn in a question thread notifies like a channel turn. Its title shows the parent channel name, and the parent's mute setting applies.
- A muted channel sends no finished or failed notification. Approvals still notify.
- A click raises the app only when a window is open. With every window hidden, a click does not open a window (known limit).
- A click only brings the app to the front. If a window gains focus within 10 seconds after a notification, it opens the channel of the newest notification.
- If the system permission is denied, no notification shows.
- The project menu items for notification modes stay planned ("Soon").

### Appearance page

Layout preferences apply to all conversations. Changes save automatically and update the main window.
The defaults preserve the existing appearance. Hand edits to these keys also apply without a restart.

From top to bottom:

- **Mini project rail.** Uses a 48 px rail with 32 px icons at the default scale. The default rail stays
  72 px wide, or 52 px in the narrow window layout. Tooltips, selection markers, and unread and approval
  indicators stay visible. Plugin font and spacing scales still apply.
- **Message density.** Comfortable keeps the existing spacing. Compact reduces row padding, block spacing,
  and gaps between messages. It keeps text size and author names unchanged. It applies to saved messages,
  streaming replies, and system notes.
- **Show message avatars.** On by default. Turn it off to remove the avatar column. Author names remain
  at the start of every message, on the same line as its text.
- **App icon.** A card below Theme shows seven icons: Classic, Landscape, Graffiti, Manga, Cyberpunk,
  Matrix, and Synthwave. Select one to change the icon in the Dock or taskbar at once. Hand edits to
  `appearance.app_icon` also apply without a restart.
- **Live preview.** A sample rail and conversation use the same components as the main window. The preview
  reflects all three preferences and the active theme. Preview controls do not perform actions.

Messages reserve a left column for grouped timestamps. A new author or minute starts a new group.
Date dividers mark the first visible date and each date change. Saved message IDs retain UTC creation
times; the bridge exposes them as Unix milliseconds. The UI uses the device's time zone and locale
for timestamps and date dividers, including daylight saving time. Messages without a valid saved time
leave the timestamp blank.

Saved messages show Copy, Reply, and a thread action on hover or keyboard focus.
The icon toolbar aligns with the message's top-right corner. Tooltips appear after 500 ms.
Text in messages, code blocks, diffs, and tool calls can be selected. The toolbar cannot be selected.

**Reply to message** sets a pending reply for the composer. It does not change the draft. It focuses
the composer. The reply shows as a chip above the input: "Replying to" the author, a one-line
excerpt, and **Cancel reply**. Escape in the input cancels the reply. Select the chip to open the
source message. A composer has one pending reply. A new reply replaces it. A failed send keeps the
reply. Reply is also in the message context menu. It is disabled when the composer is unavailable or
the message has no text.

Send joins the reply prefix and the typed text. The prefix is a Markdown quote of the source text
with a link to the source message. The agent receives the full source text. The transcript hides the
prefix and shows a reply row above the message: a reply icon, the source author, and an excerpt.
Select the row to open the source message in its conversation and highlight it. A source in the open
thread scrolls the thread panel. When the source is gone, a toast says "Original message is not
available." The row stays.

A thread is a side conversation about one message. The thread action is **Start thread** on a message
with no thread. It opens the thread panel with the message and an empty composer. No thread exists
until the first send, and closing the panel discards the draft. A message that has a thread shows
**Open thread** instead, and a chip under it. The chip shows the thread name, the message count, the
last message, and an unread mark. Select the chip to open the thread. A **Follow** button on the
chip follows a thread that is not followed. Both actions are also in the message context menu. A
message that still streams has no thread action. The action is disabled when the channel is read only.

The thread panel takes the place of the context panel and closes back to it. It is 360 to 640 px wide.
On narrow screens it shows in the right sheet. Only one thread is open at a time. The header shows
the thread name and its channel, **Follow** or **Leave**, and a close button. The anchor message shows
at the top, then the thread messages. They use the same rows as the channel, so live replies, approvals,
and tool calls work the same way. Approvals and Stop act on the thread. A note says that threads
are read-only. The composer has the same attachments and mentions as the channel composer. A
reply to a thread message goes to the thread composer. The thread is marked read when its panel
shows the newest message.

The thread composer has an **Also post to #channel** box. It is off by default and applies to one
message. Every thread message, including an agent reply, has **Post to channel** in its hover
toolbar and context menu. A posted message shows in the channel as a row with a **from thread** link
that opens the thread. A posted message shows **Posted to channel** in the thread.

**Leave** removes a followed thread from the channel bar. The chip stays. Replying in a thread or
choosing **Follow** follows it again. A followed thread shows indented under its channel in the
channel bar and under its conversation in the direct message list. It shows unread, running, and
pending approval marks. Select it to open its channel with the thread panel. The thread row menu has
**Open thread**, **Leave thread**, **Rename thread**, and **Delete thread**. Thread activity does not mark
the parent channel unread.

The **Threads** button in the channel header opens a list of every thread of the channel, followed
or not. The newest activity comes first. A row shows the thread name, its kind (**Question** or
**Sub-agent**), the run state of a sub-agent, the message count, an unread mark, and **Follow** or
**Leave**. Select a row to open the thread. An empty channel says "No threads yet."

**Rename thread** is in the thread panel menu and the thread row menu. The dialog has one name field.
The name is trimmed, line breaks become spaces, and the length is 1 to 80 characters. Names can repeat.
The new name shows on the chip, the channel bar, the panel header, the Threads list, and in the label
`[From thread "<name>"]` that the channel agent reads.

**Delete thread** is in the same menus. A confirm dialog opens, with Cancel in focus. The item is
disabled while a turn runs in the thread, while its sub-agent task runs, and while an approval waits.
No Soon label shows, because the state is temporary. A message that you posted to the channel stays
there as a normal message, at the same place, with the same role, author, content, and attachments.
It loses its **from thread** link and its replay label, so the channel agent's context does not
change. The dialog says how many posted messages stay. The delete removes the thread, its messages,
approvals, and read marker. The panel closes, and the chip under the anchor message goes.

**Remove from channel** is in the hover toolbar and the context menu of a posted row. It removes the
row from the channel only. The message stays in the thread, and **Post to channel** works on it
again. A confirm dialog always shows. When an agent message follows the row in the channel, the dialog
warns that the agent already read the message, that earlier replies can still refer to it, and that
later turns will not see it. The row goes in both cases. The owner chose these rules for the delete and
the removal. The design is in [Threads — design](superpowers/specs/2026-11-10-threads-design.md).

A **sub-agent thread** is a read-only research run. An agent starts it with the `delegate` tool.
You cannot start one. Each task of one `delegate` call makes one thread. During the run, the chips
show under the live message. After the turn, they show under the saved response, in task order,
below the question thread chip if there is one. A chip shows the title, the sub-agent's name, and
the state: **Running** (with the spinner), **Done**, **Failed**, **Cancelled**, or **Timed out**.
A chip also shows an unread mark, a **Follow** button, and the number of pending approvals. Select
a chip to open the thread, also while it runs. A sub-agent thread is not followed at creation. It
shows in the channel bar only after you follow it or a follow trigger fires. Its row has its own
icon, and it shows running, unread, and pending approval marks. A failed, cancelled, or timed out
thread keeps a mark in the row.

The sub-agent thread panel header shows the title, the sub-agent's name, and the state. While the
parent turn runs, the thread has no anchor, so no anchor message shows. After the turn, the anchor
is the saved response. The first message is the brief. It shows the parent agent as its author, not
**You**. The transcript is live while the sub-agent runs. **Stop** in the thread stops that
sub-agent only. The parent agent gets "cancelled" for that task and continues. **Post to channel**,
Copy, Reply, **Follow**, and **Leave** work. A reply from a sub-agent thread goes to the channel
composer. The composer is on. While the sub-agent works, it reads **Steer this sub-agent**: a message stops the
current turn, keeps the partial work, and starts a new turn with your message. The task does not end,
and the parent agent gets the final message of the last turn. That result starts with a note that says you sent messages, and quotes the latest one. After the task ended, the composer reads
**Reply in thread**, and a message starts a normal turn in the thread. The state does not change, and
the parent agent gets nothing. **Stop** works in both cases. **Also post** works as in a question
thread. A sub-agent thread cannot start another thread.

The **Delegation** switch is in the agent's tool permissions. It is off by default. When it is on,
the agent can start read-only sub-agents in threads in Conversation and Workspace responses. Turning
it on goes through the same confirmation as the other permission changes.

An agent that wants a sub-agent that is not a project member asks with `agent_request_add`. The
channel then shows an **Add an agent to this project?** card. It names the requesting agent and the
requested agent with its provider and model. The reason shows as a quote in plain text: it is the
agent's own words. **Add to project** makes the agent a project member with no end date and
continues the requesting agent. **Deny** continues the requesting agent with the note that you
declined. Both buttons are off while a response runs. A decided or failed request keeps its card
with the result. Restarting Discard cancels pending requests. Direct messages show no card.

Approvals that wait in a thread, followed or not, count with the channel approvals in the navigation
and in the status line. **Approvals** in the navigation goes to the first approval that waits: the selected
channel, then its threads, then the other channels, then the other threads. A thread opens in its
channel with the thread panel.

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

### Agent profiles (main window)

The profile editor has a **Keep provider session** switch (off by default). It keeps this agent's provider session between responses. The provider stores the conversation on disk. The setting grants no new tool access. When it is on, a response sends only new messages, and the context panel shows "Resumed session" or "Fresh replay: <reason>". Discard never deletes the provider's saved conversation, also when the setting is turned off. See [native provider resume](runtime/conversation-context.md#native-provider-resume).

In the profile editor, **Generate** turns a short description into Personality text using the provider, model, and reasoning level currently selected in the form. **Undo** restores the previous text; nothing is saved until you choose **Save profile**.

Direct Messages reopens the last viewed DM. This choice stays saved across restarts. If no saved DM is available, it opens Agents. Choose **Agents** to find an agent or choose **Add agent**. The add dialog asks for a generated editable name, connected provider, model, and reasoning level. Provider and Model use Select menus. Changing the provider clears the model and loads its available models from the installed CLI. Choose a model before adding the agent. Loading failures offer Retry models. Claude aliases use the resolved model ID when available. Select an agent to open its direct chat. The direct-message header profile button toggles a short profile sidebar. Project pages hide this button, and project navigation omits the agent and conversation list. Choose **View full profile**, then **Edit profile** to change identity, agent icon and color, model, personality, or tool permissions. The icon picker opens in a Popover and searches every icon in the installed Lucide library. Agent color has presets and a custom color control. Color is independent of the icon: choosing a color keeps initials when no icon is selected. The saved appearance appears in the agent list, direct-message navigation, profiles, member cards, and chat messages. Existing image avatars remain visible until replaced. The profile editor also uses Select menus for Provider, Model ID, and Reasoning level. Both agent forms load supported reasoning levels from the selected model’s CLI metadata. Provider default leaves reasoning to the provider. Changing the model or provider resets reasoning to Provider default. The saved level applies to the next response. If model metadata is unavailable, the profile editor keeps the saved level and lets you restore Provider default. It keeps the saved model available if the provider no longer lists it. Changing providers requires a new model selection. Agents do not have a Settings page. **Agents** in the navigation lists all agents. Choose **DM** to open the chat and add the agent to the Direct Messages list. Creating an agent alone does not add it to that list. The list stays saved across restarts. Each response receives the agent's current name together with its personality and memory. A name change applies to the next response; it does not rewrite saved messages. DMs show the agent name, a short-profile toggle, and a compact history search field. Project channel labels and conversation tabs do not appear in DMs. The DM composer stays at the bottom without a footer.

The chat header searches saved direct message text, code, and diffs for the selected agent across its direct conversations, including archived chats. Project conversations are excluded. Search returns the newest 50 matches and asks you to refine the query when more exist. Select a result to open its conversation and highlight the matching message. Multiple agents can use the same model. Personality stays separate from learned memory. A looser permission policy or enabling `plugin.write` requires explicit confirmation. Unsupported choices show the reason.

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

## The channel list

The channel list in the main window has these sections:

- **Text channels** holds `#general` and the text channels of the project. `#general` comes first.
  Text channels follow in the order you created them. A text channel has no branch and works in the
  project folder. It uses the project main agent. When the main agent changes, each active text
  channel changes with it.
- **Branch channels** holds one row for each Git branch of the project.
- **Archived** holds archived channels and saved history. It is hidden by default.

Use the **+** button of Text channels, or **New text channel** in the project menu, to create a text
channel. Enter a name. The dialog shows the saved name as `#name`. Discard saves the name in lower
case. It joins words with hyphens. A name holds 1 to 50 letters, numbers, hyphens, and underscores.
A name cannot match another active text channel or `general`. The new channel opens at once.

**Rename Channel…** works in the row menu of a text channel. The dialog has one name field with the
current name. It uses the checks of channel creation: the same name rules, and a name that no other
active text channel or `general` has. An unchanged name closes the dialog with no change. `#general`
and branch channels cannot be renamed here, and `#general` has no rename item. **Archive Channel** in
the menu bar archives the open text channel. An archived text channel shows in Archived.

### Row menu actions

The row menus of the channel list have working Mute, Archive, Git branch, and Git worktree items. A
rule that blocks an item greys it with no Soon label. Discard checks every rule again when the item
runs, so a stale view cannot start a blocked action. The design is in
`docs/superpowers/specs/2026-11-11-navigation-menu-actions-design.md`.

A channel is busy while a turn runs in it, while a sub-agent task runs in one of its threads, while an
approval waits in it or in one of its threads, and while an archive runs.

- **Mute Channel** and **Unmute Channel** work at once, with no dialog. Discard saves the flag with the
  channel (schema version 14). A muted channel row shows no unread marker, dims its name, and shows a bell-off
  icon labelled `Muted`. A muted channel does not count toward the unread marker of its project on the project
  rail. Unread threads of the channel still show as unread in its Threads list. Pending approval badges do not
  change. Mute changes no read marker, message, agent, or turn, so Unmute shows the unread marker again when
  the channel has unread messages. `#general` can be muted. A thread cannot.
- **Archive Channel…** archives the channel of the row with the rules, the confirm step, and the store path of
  the menu bar item. It is greyed for `#general`, for an archived channel, and for a busy channel. A dialog
  names the channel and opens with Cancel in focus. If the archive would delete a worktree that holds files,
  the dialog lists them and asks for a second confirm. When the archived channel was open, `#general` opens.
  There is no restore action yet.
- **Create Worktree…** (branch row) creates a linked worktree for the branch. The dialog shows the branch and
  the target path. It is greyed when a working tree holds the branch, or when the channel of the branch already
  has a worktree. A branch with no channel gets a channel with the worktree. A Git failure shows as an error
  toast and saves nothing.
- **Rename Branch…** (branch row) runs `git branch -m` and moves the channel binding to the new name. The
  channel keeps its messages, sessions, and worktree record. The dialog checks the name with
  `git check-ref-format --branch` and refuses an empty name, the current name, and the name of another local
  branch. It works for a checked-out branch. It is greyed while the channel is busy. If the store update
  fails, Discard renames the branch back.
- **Delete Branch…** (branch row) is greyed for the default branch (the branch of the main checkout), for a
  branch that a working tree holds, and while the channel is busy. The dialog opens with Cancel in focus. With
  no commits that the default branch lacks, one confirm runs `git branch -d`. Otherwise the dialog warns with
  the count and asks you to choose **Delete N unmerged commits** before it runs `git branch -D`. Discard counts
  the commits again when you confirm, and refuses unmerged commits without that choice. The channel follows the
  existing rule for a deleted branch: it moves to Archived only when it has a conversation.
- **Create Branch…** (worktree row) runs `git switch -c` in the working tree. The branch starts at the HEAD of
  the tree, and uncommitted changes stay. It is greyed while a channel that uses the tree is busy.
- **Remove Worktree…** (worktree row) is greyed for the main checkout and while a channel that uses the tree is
  busy. The dialog shows the path and says that the branch and its commits stay. A clean tree needs one confirm.
  A tree with changed or untracked files lists up to ten of them with the count of the rest, and asks you to
  choose **Discard these changes** before it removes the tree. Discard reads the status again when you confirm.
  The saved worktree record of each channel bound to the path goes. The channels stay.

After a Git action, Discard refreshes the branch list.

Archived holds a channel when one of these is true:

- The channel is archived.
- The channel is saved history from an older version.
- The channel belongs to a branch that Git no longer lists, and the channel has a saved message.

A deleted branch whose channel has no message never shows in Archived. Discard keeps its record. If
the branch comes back, Discard opens the same channel again.

To show Archived, open **Project settings**. In the **Channel list** card, turn on **Show archived
channels**. The setting belongs to one project. Archived stays hidden when the setting is off, even
when the open channel is in it. Archived also stays hidden when it has no channel.

## Context menus and archive

Right-click a message to copy its text, a code block to copy its code, or a channel row to copy its name or archive it.
Right-click a diff to copy it, a tool call to copy its input or output, or an agent to copy its name or view its profile. Every project icon has a context menu. It can open Project Settings, create a channel, reveal the folder in Finder, remove the project after confirmation, or copy its name, path, or ID. Opening a menu does not select the project. Settings and channel creation select the target project through the existing idle-agent guard. Copy, Reveal, and Remove use the clicked project's identity.

The **Move to…** submenu has working **Group** items: **No Group**, one item per group, and **New Group…**. **No Group** is disabled while the project has no group. **First in Group** and **Last in Group** move the project to the start or end of its group, or of the rail when it has no group. **New Group…** creates a group that holds the project and opens the rename dialog.

Right-click a group tile or the folder button of an open group for the group menu. **Rename Group…** opens a dialog; an empty name shows the project names. **Group Color** offers **Default** and six theme colors. **Collapse** and **Expand** toggle the group. **Ungroup** returns the projects to the rail at the position of the group. **Mark Group as Read** marks every channel of every project in the group as read, and every thread under those channels. It leaves pending approval badges and direct messages unchanged. You can also drag project icons: drop one on another project to make a group, drop it on a group to add it, drop it between icons to reorder, or drag it out of an open group. Drag a group tile or folder button to move the group. Press Escape to cancel a drag. Groups never change folders on disk.

**Mark Project as Read** in the project menu marks every channel of the project as read, and every thread under those channels. It leaves pending approval badges and direct messages unchanged. It works while a navigation is busy and does not select the project.

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

When a turn runs and `confirm_while_working` is `true`, Quit and the Safe Mode relaunch do not act at once. The focused
app window comes to the front and shows a notice with **Cancel** and **Confirm** buttons. When no app window is
visible, the main window shows first and shows the notice:

- "The agent is working. Quit Discard and stop the turn?"
- "The agent is working. Relaunch in safe mode and stop the turn?"
- "The agent is working. Relaunch normally and stop the turn?"

Closing the last app window (the red close button, `Cmd+W`, or File > Close Window) follows the Quit rule and shows
the Quit notice, but only when the app does not run in the background. Otherwise the window closes or hides and the
app goes on. See "Closing the main window". Closing the Settings window never asks.

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

Closing an app window (the main window or a project window) closes that window only. The main window hides, and
agents continue. When `keep_running_in_background` is `true` (the default on macOS and Windows), the close of the last app window does not
end the app. On macOS the Dock icon shows the main window again. On Windows and Linux a tray icon has the items
`Main Window` and `Quit Discard`, and on Windows a left click on the icon shows the main window. If the tray icon cannot
be created, the app does not run in the background in that session, and the General page says why.

When `keep_running_in_background` is `false` (the default on Linux), or the tray icon is not available, the close of the last app window
quits the app. While a turn runs and `confirm_while_working` is `true`, the focused app window shows the Quit notice
first. The main window shows first when no app window is visible.

## Limits you should know

- The default provider applies at the next launch.
- Relaunch works on unix only. On other systems the code starts the new process at once, without waiting for the old
  one to exit. Windows is out of scope for v1.
- The relaunched process has no input or standard output. `relaunch.rs` sets them to null. Its stderr is inherited,
  so the relaunched Discard writes its logs where the first one did.
- The confirm notice comes from the Quit action (`Cmd+Q` and Quit Discard), closing the last app window when the app
  does not run in the background, and the Safe Mode relaunch. Discard has no hook that can stop a Quit from the Dock or the system, so those do not ask. They end
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
- **Terminal colors.** The terminal uses the 16 palette colors of the theme file, plus its background, foreground,
  cursor, and selection colors. Plugin tokens `ansi_1` through `ansi_6` replace palette colors 1 through 6. The font is
  the Ghostty `font-family`. The size is 13 px, scaled by `font_scale`. A change applies to an open terminal at once.
- **Diff colors.** Added and removed lines in a plugin review use palette 2 and palette 1, through the contrast rule.
- **A theme that fails.** A missing theme, a missing Ghostty config, or a bad value falls back to the default pair.
  A bad line in a theme file is skipped. The General page lists each problem with the file and line.

## Window state

`$DISCARD_HOME/state.toml` stores the main window position and size and the channel and member list widths.
Discard restores bounds that are large enough and visible on a current display. Otherwise it opens the window centered.
Drag the dividers to resize the lists from 180 to 400 pixels. Each list defaults to 240 pixels.
Discard saves window changes and panel widths in this file. Sidebars keep their pixel widths when the window changes size; the chat panel takes the remaining space. At narrow window sizes, navigation and profile panels use the compact layout.

The terminal panel height and visibility are saved in the webview storage of the window, not in `state.toml`.

The composer sends with Enter. Shift+Enter inserts a newline.
Plugin theme tokens `font_scale` and `spacing_scale` adjust text and spacing. Ignore plugin themes disables both tokens.
