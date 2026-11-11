# Channel bar sections, text channels, and Reply

Date: 2026-11-10

Seven observations from the owner. Items 2 to 6 change the channel bar. Items 1 and 7 change the
message view.

1. Text in the message view can be selected.
2. "Saved channels" becomes "Archived".
3. A deleted branch moves to Archived only if its channel has a conversation.
4. A per-project setting shows the Archived section.
5. `#general` sits in a new "Text channels" section.
6. The user can create text channels for project conversations.
7. "Quote message" becomes "Reply to message". A reply links to the source message. It does not
   paste the source content.

## Decisions

The owner made these decisions:

- The Archived section is hidden by default. A switch on the Project settings page shows it.
- A deleted branch whose channel has no saved message never shows in Archived. Its record stays in
  the database. If the branch comes back, the same channel is used again.
- Text channels ship with create only. Rename and archive stay visible as disabled `Soon` items. Rename
  became a working action in decision 99 below.
- The agent receives the full source text of a reply. The UI shows only a link row.

These are implementation calls, not owner decisions:

- A reply keeps the wire format that Quote used. No schema change is needed for replies.
- A text channel is a new project channel kind, `text`. Schema version 13 adds it.
- The per-project setting is a column on `projects`.
- Text channel names are ASCII only.
- Row tags in Archived (`History`, `Saved`, `Channel`) do not change.
- The Archived section stays hidden when the setting is off, even if the open channel is in it.

## Terms

- **Text channel**: a project channel with no branch. `#general` is the first text channel.
- **Archived**: the channel bar section that was "Saved channels".
- **Reply**: a user message that points to one source message.
- **Reply row**: the compact link row above a reply in the transcript.
- **Reply chip**: the row above the composer while a reply is pending.

## 1. Text selection

Cause: `ContextMenuTrigger` adds `select-none`, and `ContextMenuHost` wraps every message.

- `ContextMenuHost` passes `select-text` to its trigger when the target type is `message`,
  `code_block`, `diff_block`, or `tool_call`. Other targets keep `select-none`.
- Buttons and the message toolbar stay unselectable.
- The context menu still opens on right-click.

## 2. Archived section

- The section label is `Archived`.
- UI ids: `archived-channel-navigation`, `archived-channel-toggle`, `archived-channel-<id>`.
- Code uses "archived", not "saved", for this section.

## 3. Deleted branches

`ProjectChannelDto` gets `hasMessages`. It is true when the channel, or a thread of the channel,
has one or more saved messages.

The Archived list holds a project channel when one of these is true:

- The channel is archived.
- The kind is `legacy` or `archived_legacy`.
- The kind is `branch`, the branch is not in Git, and `hasMessages` is true.

The list never holds an active `general` or `text` channel. Discard deletes no data for a hidden
channel.

## 4. Show archived channels

- Schema 13 adds `projects.show_archived INTEGER NOT NULL DEFAULT 0`.
- Store: `project_show_archived(project)` and `set_project_show_archived(project, show)`.
- `get_projects` returns `showArchived` for each project.
- New command `set_project_show_archived(project_id, show)`. It emits `discard://projects`.
- The Project settings page gets a card named **Channel list** with one switch:
  **Show archived channels**. Help text: "Show the Archived section in the channel list. It holds
  archived channels and channels of deleted branches that have messages."
- The channel bar shows the Archived section only when the setting is on and the list is not empty.

## 5 and 6. Text channels

### Model

- `ProjectChannelKind::Text`, stored as `text`. The DTO kind is `"text"`.
- Schema 13 rebuilds `project_channels` so the `kind` CHECK allows `text`. It keeps every column,
  row, and index. Follow the version 5 rebuild. Keep the `repository` column from version 7.
- A text channel uses the project main agent and `WorkspaceMode::MainCheckout`, as `#general` does.
- Runtime code that gives `General` conversation treatment gives `Text` the same treatment. One
  example is `workspace_session::eligibility`.
- A text channel is never the canonical general channel. `ensure_project_general_channel`,
  `set_canonical_project_channel`, and `select_canonical_general` ignore it.
- When the project main agent changes, each active text channel follows it the way `#general` does.

### Names

`normalize_text_channel_name(input) -> Result<String, String>` in `discard-core`:

1. Trim. Change to lowercase.
2. Change each run of whitespace to one `-`.
3. Change each run of `-` to one `-`. Remove `-` from both ends.
4. Refuse a character outside `a-z`, `0-9`, `-`, `_`: "Use letters, numbers, hyphens, and
   underscores."
5. Refuse a length outside 1 to 50: "Enter a channel name of 1 to 50 characters."

The name must be unique among the active `general` and `text` channels of the project, without
regard to case. `general` is always taken. Error: "A channel with this name already exists."

### Create

- Store: `create_project_text_channel(project, name) -> Channel`. One transaction inserts the
  channel, a session, the `project_channels` row, and the `channel_agents` row. It fails when the
  project has no main agent.
- Desktop: `Channels::create_text_channel(project, name)` creates the channel, starts it, and
  selects it. Command: `create_text_channel(project_id, name) -> channel id`.

### Channel bar

- A **Text channels** section replaces the lone `#general` row. Its key in the section state is
  `text`. It is open by default. It has a count and a `+` button, like **Branch channels**.
- `#general` is the first row. Text channels follow in creation order. Each row looks like the
  `#general` row and shows its threads, spinner, and unread mark.
- The `+` button and the project menu item **New text channel** open a dialog. The menu item
  replaces the planned **New conversation** item.
- The dialog has one name field, a live `#name` preview of the normalized name, and this help text:
  "Use a text channel for a project conversation that has no branch." It shows a server error in
  the dialog.
- The context menu of a text channel row has **Mute Channel** and **Archive Channel…** as planned `Soon`
  items, each with a `TODO`, and **Rename Channel…** as a working item (decision 99). `#general` has no
  rename item.
- The **Archive Channel…** item of the menu bar works on a text channel as it does on any channel.
  An archived text channel shows in Archived.

### Rename (decision 99)

The designer chose this rule, in the thread actions slice ([Threads — design](2026-11-10-threads-design.md)).

- **Rename Channel…** is a working item in the row menu of a text channel. It sits between **Mute
  Channel** and **Archive Channel…**, which stay planned.
- The dialog has one name field with the current name and a live `#name` preview. It uses the same
  normalization (`normalize_text_channel_name`) and the same uniqueness check as creation: unique among
  the active `general` and `text` channels of the project, without regard to case. `general` is always
  taken. The check ignores the channel itself.
- `#general` and branch channels cannot be renamed here. `#general` has no rename item. An archived text
  channel cannot be renamed.
- An unchanged name closes the dialog with no write.
- Store: `rename_text_channel(project, channel, name) -> Channel`. Command:
  `rename_text_channel(project_id, channel_id, name) -> stored name`. A backend error shows as a toast,
  and the dialog stays open with the name intact.

## 7. Reply

### Labels

- Toolbar button: `Reply to message`. Context menu item: `Reply`.
- Code uses "reply", not "quote": `reply.ts`, `replyDisabled`, the `reply` event.

### Wire format

A reply is a user message whose text starts with a reply prefix. The prefix is the text that Quote
made, so older quoted messages are replies too.

```
> [Message from <author>](#discard-message/<channel id>/<message id>)
>
> <source text, each line quoted>

<typed text>
```

- The link is the reference to the source message.
- The quoted lines are the full source text. The agent receives them on the first send and on
  replay. They stay valid when the source message is gone.
- The prefix of a reply to a reply quotes only the typed text of that reply.

### Composer

- **Reply** sets a pending reply for that composer. It does not change the draft. It focuses the
  composer.
- Pending replies use the same keys as drafts: one for each channel and one for the thread panel.
- A composer has one pending reply. A new Reply replaces it.
- The reply chip shows above the input: "Replying to **author**", a one-line excerpt, and a
  **Cancel reply** button. Escape in the input also cancels the reply, unless another popup of
  the composer uses Escape first. A click on the chip jumps to the source.
- Send joins the prefix and the typed text. The send rule for an empty draft does not change.
- A sent reply clears the pending reply. A failed send keeps it.
- The first message of a new thread can be a reply.

### Transcript

- A user message with no author whose first markdown block starts with a reply prefix shows a
  reply row above the name line. The row shows a reply icon, the source author, and a one-line
  excerpt of the quoted text.
- The markdown block shows only the typed text. Block indexes do not change.
- A message with a quote later in the text shows as before.
- A click on the reply row jumps to the source: a thread message in the thread panel, any other
  message through the history jump.
- When the source is not found, a toast says "Original message is not available." The reply row
  stays, because the message holds the author and the text.

### Threads

Reply uses the routes that Quote used:

- A reply to a thread message, or to the anchor row of the thread panel, goes to the thread
  composer.
- A reply from a sub-agent thread goes to the channel composer.
- The disabled rules do not change.

### Limits

- **Copy Message Text** on a reply copies the stored text, with the prefix.
- History search matches the quoted text of a reply.

## Plugin-drawn UI

No Lua element draws the channel bar, a message row, or the composer. No plugin change is needed.

## Tests

- Core: name normalization, text channel create and unique name, schema 13 migration from 12,
  `show_archived` default and set, `hasMessages` with a thread message, main agent change.
- App: `create_text_channel`, DTO kind `text`, `hasMessages`.
- Web: Archived filter and label, setting hides and shows the section, Text channels section and
  dialog, selectable message wrapper, reply prefix and parse, reply chip, reply row, jump, missing
  source toast, thread routes.
