# Channel navigation implementation

Implemented after the first visual pass. This pass adds real channel creation and switching. The original palette tokens and Ghostty theme bridge remain unchanged.

## Panel behavior

Navigation now shows only Git Branches and Worktrees. Selecting an entry resolves its saved conversation through the backend. Conversations are not a separate navigation section. Both Git lists refresh automatically.

- The project header opens a menu for the overview, new channels, agent settings, project settings, and CLI connections.
- Channels have stable IDs, selected states, unread dots, running indicators, and approval counts. The order stays stable when selection changes.
- The Channels, Branches & worktrees, and Integrations sections collapse. Section preferences are saved locally for each project. Collapsing Channels keeps the selected channel visible.
- Branch expansion and branch selection have separate controls. Worktree rows show an availability dot and a monitor icon.
- Approvals opens a channel with pending decisions. CLI connections opens the Providers settings page.
- The existing shared user footer, aligned headers, resize handles, and panel scroll boundaries remain in place.

## Create and switch channels

New channel opens a shadcn-vue dialog. It asks for a name and workspace mode. Names are checked for length, characters, and duplicates. The new channel uses the selected channel's agent, including its provider, model, instructions, and approval policy.

In a Git project, the default is a new isolated worktree. The branch and worktree are created on the first turn. The Project folder option uses the existing project folder. Non-Git projects use that option.

The backend saves the channel, session, and optional branch/worktree binding in one transaction. Active channels are restored from storage when the application starts. Startup channel selection still follows the existing bootstrap policy.

Each channel has a separate runtime, transcript, live output, approval state, and read marker. An agent can continue working in a background channel. The frontend retains drafts and transcript positions per channel for the current app session. It does not save drafts across restarts.

Switching is blocked while an archive operation or confirmation needs a decision. Quit and relaunch checks include background turns. A confirmation clears when the last turn finishes.

## Branch and worktree selection

Selecting a row reuses a compatible active channel. Otherwise, it creates a channel for that branch or worktree.

- An existing checkout is borrowed. Archiving the channel keeps its directory.
- A branch without a checkout gets a worktree on the first turn. Discard uses that branch instead of creating a new `discard/` branch.
- A recorded session must still match its worktree and branch. If Git changes the checkout outside Discard, the branch row opens a compatible channel or creates a new one.
- Archiving an owned worktree checks for other active channel bindings. Shared or borrowed directories remain intact.
- Git operations keep hooks disabled. Worktree creation is serialized by the common Git directory.

Storage schema version 3 adds worktree ownership and optional channel-to-branch bindings. Existing worktree records retain their previous ownership behavior.

## Bridge changes

| Contract | Purpose |
| --- | --- |
| `SnapshotDto.channels` | Channel IDs, names, agents, providers, workspace modes, running states, unread states, and approval counts |
| `BranchRowDto.hasChildren` | Independent expansion controls |
| `create_channel(name, workspaceMode, sourceChannel)` | Persist and select a new channel with the source channel's agent |
| `switch_channel(channelId)` | Select a saved channel without stopping background turns |
| `open_branch_channel(key)` | Resolve a current Git row to a channel |
| `get_history()` and history events | Return `{ channelId, rev, messages }` |
| Send, stop, approval, and read commands | Include the originating channel ID |

The frontend rejects stale snapshots and history from other channels. The backend checks channel IDs before sending or applying decisions. Plugin renders and callbacks keep one agent memory scope for the full callback. Engine event callbacks use the originating agent's scope.

## Validation and boundaries

Rust tests cover persistence and restoration, independent sessions, background completion, stale commands, branch changes, borrowed worktrees, archive guards, and plugin memory scope. Frontend tests cover section controls, channel selection, stale history, listener cleanup, and approval routing.

Browser checks use test-only bridge responses. They cover creation, switching, separate drafts, branch controls, alignment, panel resizing, scrolling, light and dark palettes, compact navigation, and the developer overlay. The application contains no sample conversation fixtures.

This pass does not add agent invitations, agent creation, persistent DMs, or multi-agent dispatch within one channel. Each channel retains one assigned agent. Live Claude and Codex calls are not part of these automated checks.

The rail now keeps Direct Messages visible and disabled with a Soon label until persistent DMs are implemented. Each project has a context menu, including inactive projects. Planned actions remain visible and disabled with TODO markers in code. Settings and New Channel select the target project through the existing guarded flow. Copy, Reveal, and Remove use the clicked project directly. Remove requires confirmation.

## Project folders

The rail's add button opens the native folder picker. Importing a folder reuses its saved project when the canonical folder matches. Each project has its own rail entry. The selected project loads in the running app and remains selected on the next start. Import does not change another project’s folder.

Project settings includes Change folder. This keeps the project ID, agents, and message history. It creates new provider sessions and clears channel bindings to the previous repository. Existing worktree directories remain on disk. If the folder already has a project, current channels move into that project. IDs and histories remain intact; duplicate channel names receive a numeric suffix. The source project record remains available for its project-scoped memory.

Folder selection requires all agents to be idle. Sending, archiving, and channel switching are blocked while the picker is open. Canceling releases that block.

Import, folder changes, project selection, and removal do not restart the app. Project changes require idle agents. Remove project hides the rail entry and keeps files, messages, and memory. Importing that folder restores the entry. Removing the last project shows the import screen. Plugin callbacks retain the agent and project memory scope of their originating runtime.
