# Project channel foundation status

Updated: 2026-11-12

The project-channel foundation is implemented. It adds durable project channel identities, saved project navigation, a project main-agent setting, channel memberships, versioned topics, project navigation and details, and topic context in Conversation-mode prompts. The approved design in `docs/superpowers/specs/2026-11-05-project-channel-orchestration-design.md` describes a larger orchestration system. This status page describes the implemented foundation only.

## Implemented behavior

- The channel bar discovers repositories in the project folder and its descendants. Each repository has a collapsible Branch channels list. Worktrees nest under their branch and show their folder name; the main checkout has a Main badge. The section shows branch and worktree counts. Detached worktrees appear in a separate section. With multiple repositories, each has a collapsible heading. With one repository, the heading is hidden. Collapse state is saved per project and repository.
- General and branch rows share a fixed height and padding, including in crowded sidebars. The selected conversation uses a background highlight. Checked-out branches use stronger text and show their nested worktrees. Checkout state follows the Git scan, independently of conversation selection. Worktree rows show the full path on hover and retain their context menu. New branch channel, Open worktree folder, and Manage worktrees controls are visible, disabled, and labelled Soon until implemented.
- Discovery groups linked worktrees under one repository and includes registered worktrees outside the project folder. It does not follow directory symlinks. It skips Git metadata, dependency folders, and common build folders. Duplicate repository names use relative paths. The existing five-second Git refresh also refreshes discovery.
- Schema version 13 adds the `text` project channel kind and the per-project `show_archived` setting. A text channel has no branch, uses the project main agent, and works in the main checkout. It is never the canonical general channel. Each active text channel follows the project main agent. The channel bar lists `#general` and text channels in a Text channels section. The Archived section (formerly Saved channels) holds archived channels, legacy history, and channels of deleted branches that have a saved message. It is hidden until the project setting **Show archived channels** is on.
- Schema version 15 adds rail groups and a saved rail order. The rail has one ordered top-level list. Each item is an ungrouped project or a group. Each group has an ordered list of member projects, an optional name, an optional color, and a collapsed flag. A project is in at most one group. A group with no members is deleted. A group with one member is allowed. Projects without a saved position keep their `visible_projects` order. A new or restored project goes last. Hiding a project removes it from its group. Groups never touch folders on disk. The store has one transaction per change: move a project to the top level or into a group, create a group, move a group, rename it, set its color, collapse it, and ungroup it. The `get_rail_layout` command returns the layout. Each change command emits `discard://projects`. The rail shows a collapsed group as one tile with up to four member avatars. It shows an open group as a tinted container with a folder header. A collapsed group shows unread when any member is unread, using the muted-channel rule, and the sum of approval badges. Drag and drop uses pointer events. The project menu has working **Group**, **First in Group**, and **Last in Group** items. The group menu has **Rename Group…**, **Group Color**, **Collapse** or **Expand**, and **Ungroup**. **Mark Group as Read** is visible, disabled, and labelled Soon until read markers exist.
- Schema version 14 adds a per-channel mute flag. A muted channel shows no unread marker on its row and does not count toward the unread marker of its project. Pending approval badges still show. The row menus of the channel list have working **Mute Channel**, **Archive Channel…**, **Create Worktree…**, **Rename Branch…**, **Delete Branch…**, **Create Branch…**, and **Remove Worktree…** items, with confirm dialogs and backend checks. `docs/settings.md` describes each one. `docs/superpowers/specs/2026-11-11-navigation-menu-actions-design.md` is the design.
- Schema version 7 scopes branch channel identity by repository. Repositories can have branches with the same name without sharing channels. Existing project-root branch channels retain their identity. Worktree preparation uses the channel's repository.

- Schema version 6 adds an optional saved project icon and color. Project settings can edit the display name or alias without changing the project ID or folder. The rail, project header, and overview use the saved appearance.
- Schema version 5 extends the project-channel metadata contract for retained bootstrap channels. Schema version 4 migrates existing project conversations. It preserves messages and sessions, selects an active canonical `general` channel and active channel per branch when an active candidate exists, and keeps duplicate or unbound histories as linked archives. If every candidate is archived, migration leaves the group without an active canonical channel.
- Active workspace channels created by the retained Builder/bootstrap path use kind `legacy` when they have no general or branch identity. This kind preserves their channel ID, Coding mode, archive state, session, history, and agent membership. Bootstrap registers metadata and membership on creation and reuse. These channels remain listed and openable by saved channel ID. They do not become canonical `general` or branch channels implicitly.
- Project selection restores the project's last active channel. A first visit opens `general`. Direct-chat selection does not change project navigation.
- Branch selection reuses the canonical branch conversation and its history. Store APIs can rename a branch binding or retire it to release the name for reuse. **Rename Branch…** in the row menu renames the Git branch and moves the binding with the rename API. Detection of a branch renamed outside Discard and user-facing branch retirement remain pending. Branch selection does not switch Git branches or create worktrees.
- The project main agent is stored per project. New project setup uses the first active configured agent with an explicit model in Store order if no main agent is saved. All new unmentioned Conversation-mode replies use the current project main agent, including existing branch and worktree channels.
- Project and channel memberships persist. Membership alone does not dispatch a mention or start a response.
- Topics use optimistic version checks and retain revision history. The latest topic enters each Conversation-mode prompt as labelled context outside the transcript. Topic text does not dispatch tools or mentions, and explicit system instructions and the user message take precedence.
- New replies persist their author ID, name, provider, and avatar. Live replies and transcript history show that author. Older replies without author metadata use the saved channel identity.
- A user can mention one configured agent with `@Name` or `@"Full Name"`. The composer offers autocomplete. The selected agent joins the project and channel and answers with its own configuration and memory. Unknown or duplicate names fail before sending.
- Project settings expose the Primary / Moderator agent selector. This uses the existing project main-agent setting. It sets the default for each new unmentioned reply in every project channel. Existing branch and detached worktree channels read this setting at send time. The selector is disabled while the general channel or a text channel is running.
- Normal Conversation mode does not materialize branch worktrees. Worktree behavior remains separate from opening a project or branch conversation.

## Remaining approved design

These items are later implementation slices:

- Concurrent multi-agent replies, structured mention tokens, and agent-initiated delegation.
- A durable task graph, queued steering, bounded delegation, project concurrency, and task orchestration locks.
- Main-agent tools for branch and channel creation, topic handoffs, progress links, and integration.
- Descendant cancellation, restart reconciliation, paused-task recovery, and branch lifecycle detection.
- Provider-cache-aware input and context retrieval that retains required evidence.

The current main-agent setting assigns the general channel's responder. It does not schedule specialists. Conversation tools can inspect a bound worktree and request app-approved removal. They do not expose general shell access, and the app does not claim full task orchestration or recovery.

## Agent workspace writes

Slices A and B are implemented. Agents can opt in to sandboxed writes in bound branch and detached worktree channels. Discard serializes write responses by canonical worktree path, approves in-root edits, asks for shell approvals, and separates full-access approvals. Scoped `worktree_commit` calls disable hooks and create response-linked audit records. Protected changes cards close write responses. Replay compacts tool calls and changes cards. Ineligible responses remain read-only with a reason. See [worktree writes](worktrees.md#agent-workspace-writes-slices-a-and-b).

Opted-in agents can request a worktree through an app approval card. Approval
creates an owned worktree and starts `Command::Continue` for that agent. Branch
channels bound to the main checkout offer one-response grants to Claude agents.
Claude's `denyWrite` blocks shell writes to `.git`; native file-tool edits under
`.git` require the full-access card. Codex agents cannot request or receive these
grants because approved Codex commands and patches can change `.git`. Their tool
list omits the request, and their prompt explains the limit.

Always-allow rules save a user-selected shell-command prefix or an exact network
host for one agent in one project. Project settings list rules and allow revocation.
Compound shell syntax and full-access requests never match rules. Codex command
approvals retain the full-access fallback.

Sandbox autorun is implemented for Claude and confirmed by a live probe on Claude Code 2.1.321. Codex autorun passed its spike on codex-cli 0.166.0 (macOS, linked worktree); see [Codex autorun spike results](provider-codex.md#autorun-spike-results). See [worktree writes](worktrees.md#agent-workspace-writes-slices-a-and-b).

Slice C remains planned: conversation moves, new-branch targets, and branch
creation for detached worktrees. Native provider resume per agent is also planned.
This does not implement multi-agent task orchestration.

## Terminal

Each project channel can have one shell session. Use **Toggle Terminal** in the channel header, View > Toggle Terminal, or `` Ctrl+` ``. The terminal is a panel at the bottom of the main panel. It is for the user only. Agents, MCP tools, plugins, and the plugin event stream cannot read or write it. Discard never logs terminal input or output.

- **Folder.** A session starts in the worktree that the channel is bound to, if it exists. Otherwise it starts in the main checkout of the channel repository, then in the project folder. Direct messages have no terminal. The header button is disabled and gives the reason.
- **Shell.** Discard starts `$SHELL` as a login, interactive shell (`-l -i`). It falls back to `/bin/zsh` on macOS and `/bin/sh` elsewhere. The shell gets `TERM=xterm-256color`, `COLORTERM=truecolor`, and the environment of the app. Discard removes `DISCARD_PLUGIN_PASSPHRASE` and the `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, and `TERM_SESSION_ID` variables of the program that started the app.
- **Sessions stay.** A session keeps running when you switch channels or hide the panel. It keeps about 1 MiB of output. The panel replays that output when you return. A reload of the window does the same.
- **Exit.** When the shell ends, the panel shows the exit code and a **Restart** button. The session keeps its output until you restart or close it. **Kill** ends the shell and its process group.
- **Session ends.** Discard ends the session of a channel when the channel is archived, when its worktree is removed (Remove Worktree…, an approved agent request, or a folder deleted outside Discard), or when its project is removed. Discard ends all sessions when the app exits. The panel shows why a session ended.
- **Layout.** Drag the top edge to resize the panel from 120 px up to 70% of the main panel. The arrow keys also move the edge. Height and visibility are saved for the window. Visibility is the same for every channel.
- **Keys.** The shell gets every key except Toggle Terminal, the Cmd keys, and the shortcuts that the webview handles. `Cmd+C` copies a selection. `Cmd+V` pastes with bracketed paste. `Ctrl+C` stays with the shell.
- **Colors and font.** The terminal follows the app theme. See Themes in [settings](settings.md).
- **Not built.** **New Terminal Tab** is visible, disabled, and labelled Soon. One channel has one session.
