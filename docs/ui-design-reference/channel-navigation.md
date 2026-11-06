# Conversation and channel navigation

Normal startup creates no project, agent, conversation, or bootstrap workspace. It restores the independently saved conversation when available. With no saved conversation, the app opens agent navigation. The deliberate `--builder` launcher retains Coding bootstrap and workspace behavior.

## Agents and conversations

Direct Messages opens agent navigation. Creating an agent uses a generated editable name, a connected provider, an exact model ID, and optional avatar and personality. It also creates that agent's default direct conversation. Agents are global identities and can share a provider/model.

Users can open a default direct conversation or create additional conversations with an explicit agent. Each conversation has a stable ID and one fixed context: no project or exactly one project. Multiple conversations can use the same agent and project. Creating or selecting a conversation does not assign it the sidebar's selected project.

Navigation lists stored conversations, including archived and retained deleted-agent histories. Selection loads only the selected conversation's history/runtime on demand. Archived conversations and archived/deleted-agent histories have a read-only composer. Restoring an agent or conversation reopens usable sessions without dropping another conversation's unsaved reply.

Each conversation has a separate transcript, live output, approval state, and read marker. Background responses continue when another conversation opens. The frontend retains drafts and transcript positions per conversation for the app session. It does not save drafts across restarts.

Switching is blocked while an archive operation or confirmation needs a decision. Agent lifecycle changes reject running responses. Quit and relaunch checks include background responses. A confirmation clears when the last response finishes.

## Project sidebar

The project rail and active conversation have separate selection state. Selecting a project changes the sidebar and project pages. It does not close or rebind an active direct or project conversation. Stored conversation selection is saved independently in state.toml.

Project pages list actual stored conversations and their agents. New conversation requires an explicit agent choice. An empty project creates no implicit agent or first conversation. The Memory page shows applicable memory only when the active conversation belongs to that project; Settings → Memory browses all domains.

Direct chats read global and agent memory. Project chats also read their matching project and agent-project memory. Backend context comes from the stored conversation. Sidebar selection never supplies a direct chat's memory scope or provider working directory.

## Branches and worktrees

Git Branches and Worktrees remain available for a matching project conversation. Selecting a row resolves its saved channel or creates a compatible binding. A direct conversation cannot run project branch actions through a selected sidebar project. A mismatch between the active conversation and selected project gives an error.

Borrowed checkout directories remain intact when their channels are archived. Owned worktree removal checks for other bindings and local changes. Discard does not force removal. Git operations keep hooks disabled and serialize worktree creation by the common Git directory.

Normal Conversation mode uses an empty app-owned provider working directory and exposes scoped memory tools. It does not prepare a git worktree or give the provider external write actions. The deliberate Coding path retains native resume and workspace preparation behavior.

## Memory links and fresh conversations

The conversation Memory panel and Settings item inspector provide a source link when the saved source still exists. The link uses lazy stored-history selection, including retained archived/deleted-agent history. An unavailable source is marked.

Successful source selection opens the transcript page in the main window, including when that conversation was already selected while a Memory page was visible. Start fresh conversation derives the agent and optional project from the stored source conversation. It creates and selects an empty conversation without copying the old transcript or capture pause. Both actions preserve separate sidebar project selection.

Forget removes saved memory from retrieval and pauses automatic capture. It does not delete transcript messages. The conversation provides explicit Resume and fresh-conversation controls. Shared/inferred proposals show real approve/reject controls. Approved updates retain the version used for Undo, including after a delayed response-notice retry. See [Saved memory](../memory.md).

## Bridge contracts

| Contract | Purpose |
| --- | --- |
| `SnapshotDto.channelId`, `agentId`, `activeProjectId` | Active conversation and immutable optional project context |
| `SnapshotDto.projectId` | Separate sidebar selection |
| `SnapshotDto.composerDisabled` | Read-only or absent conversation state; Rust also checks sends |
| `SnapshotDto.channels` | Loaded runtime state, running indicators, unread state, and approval counts |
| `get_conversations()` | Complete stored catalog, including retained histories |
| `open_agent_chat(agentId)` | Open the stored default direct conversation |
| `create_conversation(agentId, projectId, name)` | Create/select an explicit direct or project conversation |
| `switch_channel(channelId)` | Load/select stored history without stopping background responses |
| `create_channel(name, workspaceMode, sourceChannel)` | Guarded project channel/workspace creation |
| `open_branch_channel(key)` | Resolve a matching project Git row to a channel |
| `get_history()` and history events | Return `{ channelId, rev, messages }` |
| `SnapshotDto.memoryRev` | Refresh persisted memory cards independently from transcript IDs |
| `discard://agents`, `discard://conversations` | Refresh shared stored catalogs |
| `discard://conversation-opened` | Open the transcript page after a source/fresh action |

The frontend rejects stale snapshots and other conversations' history. Send, stop, approval, and read commands carry the originating channel ID. Backend checks remain authoritative. Plugin renders and callbacks retain their captured agent and optional project context.

## Project folders

The rail add button opens the native folder picker. Import reuses a saved project with the same canonical folder. Import selects its sidebar entry and does not change another project's folder or create a conversation.

Project settings includes Change folder. This preserves the project ID and stored histories where possible, clears bindings to the old repository, and rebuilds idle runtimes. Existing worktree directories remain on disk. If the folder already belongs to a project, stored channel records move to that project and duplicate names receive suffixes. The source project record remains available for its scoped memory.

Folder operations use the project-change guard and require idle responses. Sending and conflicting navigation remain blocked while the picker is open. Cancel releases the guard.

Removing a project hides its rail entry and preserves files, histories, and memory. Importing the same folder restores the entry. Removing the last project leaves agent/direct-conversation navigation available and preserves any active conversation. It does not force an import-only startup screen.

## Validation and planned controls

Rust and Vue tests cover startup without projects, fixed context, multiple conversations, independent selection, source links, fresh conversations, lifecycle guards, retained histories, unsaved reply recovery, stale commands, and memory boundaries. Tests use local providers/fixtures; live authenticated Claude and Codex calls are not part of these checks. Native GUI verification remains separate.

Agent invitations, group chats, unsolicited messages, and external actions remain deferred. Visible planned controls are disabled, show Soon, and have nearby TODO comments. Rail menus use the shared planned-item model. Each implemented conversation has one assigned agent.
