# Saved memory

Discard stores each memory as an editable item. It refreshes applicable memory before every response, including replies in existing conversations. A restart is not required. Current user instructions take priority over saved memory.

## Domains and conversation context

| Domain | Visibility |
| --- | --- |
| Global | All agents in every conversation |
| Agent | One agent across projects and direct conversations |
| Project | All agents in one project |
| Agent-project | One agent in one project |

A direct conversation reads global and agent memory. A project conversation also reads its project and matching agent-project memory. The application enforces these boundaries. Selecting another project in the sidebar does not change the active conversation's memory.

Memory precedence is agent-project, project, agent, then global. Conflicting items remain available for inspection. All applicable instructions and preferences enter each response. Other items are selected by word overlap with the current request and recent messages in that conversation. Scoped read and search tools provide access to other applicable items. This selection is not semantic retrieval. Other conversations' transcripts are not loaded.

## Saves and approvals

Agents receive guidance to save durable facts, preferences, decisions, and ongoing commitments. Guidance excludes temporary details, guesses, passwords, and tokens. The application does not provide semantic secret detection.

Automatic saves use agent memory in direct conversations and agent-project memory in project conversations. Global and project writes require user approval. Memory proposals show the proposed text and domain with **Approve** and **Reject**. Inferred updates and contradictions also require clarification or approval. User corrections can replace saved text. Previous values remain in version history.

**Automatic save** cards appear beside the response that saved them. **Approved update** cards identify approved changes, including approvals made after the response ended. User edits in Settings are not labeled as automatic saves. **Undo** uses the saved version. A later change can make Undo or an approval stale. Discard then shows an error; reload memory and inspect the current item before retrying.

If memory approval succeeds but its response notice cannot be saved, Discard reports that memory is approved and saved. The decision controls close. **Reload memory** or **Retry memory notice** retries the saved association without approving again. The recovered card keeps the original approved version, even after a later edit. A stale Undo still requires inspection of the current item.

## Controls

Open the conversation's **Memory** panel to browse applicable items. Open **Settings → Memory** to browse all four domains, including retained items whose agents or projects are no longer present.

**Inspect** shows the item's text, kind, domain, version, creation and update times, source link, and version history. **Save edit** changes the text. **Move** changes its domain; a move to global or project memory explicitly shares the item. **Open source conversation** loads the saved conversation, including retained archived or deleted-agent history. An unavailable source is marked and does not prevent inspection.

**Reveal item file** selects the actual `.items/<UUID>.md` file. If you edit that file outside Discard, versioned changes fail until **Import file edits and reload** imports the readable body as a new user revision. Copy an unsaved editor draft before reload if you want to keep it. A normal reload does not import external file edits.

## Forget and automatic capture

**Forget** removes an item from retrieval. It preserves its version history. It pauses automatic capture in the item's source conversation and in the requesting conversation. Undo of a new automatic save also forgets the item and pauses capture. The pause survives restart and blocks paraphrases and unrelated automatic saves as well as exact recapture. Explicit user saves and approved updates remain possible.

The conversation shows the pause with **Resume automatic capture** and **Start fresh conversation**. Only an explicit app action can resume capture. Model tool calls and model JSON cannot resume it. After resume, exact trimmed values from the forgotten item's current and previous versions remain suppressed in the affected conversations. This exact check does not recognize paraphrases.

A fresh conversation uses the same agent and optional project. It copies neither the transcript nor the capture pause. Its applicable saved memory remains available. Forget does not erase transcript messages. Transcript deletion is a separate operation. Old source messages can still affect a response in the old conversation.

## Storage and tool boundaries

Under `$DISCARD_HOME` (default `~/.discard`):

| Path | Content |
| --- | --- |
| `memory/.items/<UUID>.md` | Authoritative readable item body and metadata, including domain and history |
| `memory/.proposals/<UUID>.md` | Pending or resolved proposal |
| `memory/agents/`, `memory/projects/`, `memory/agent-projects/`, `memory/global/` | Legacy raw files for explicitly granted plugin use |
| `memory-index.db` | Rebuildable FTS5 index of live memory |
| `run/` | Temporary scoped MCP configuration files, removed after the turn |

Only live current item content enters retrieval. Deleted values, history, and pending proposals do not enter the search index or prompts. Each item has one authoritative document. Moves update that document. Writes are atomic and serialized within one MemoryStore instance. The store does not lock external file editors or other processes.

Conversation providers receive a scoped loopback MCP server with six tools: `memory_list`, `memory_read`, `memory_search`, `memory_save`, `memory_edit`, and `memory_forget`. Agent tools cannot directly approve shared writes, change personality, or resume capture. Token-bound identity supplies the agent, optional project, and source conversation. Model fields cannot grant access to another scope. Summary requests use read-only memory access.

Scope IDs and relative markdown paths are validated. Symbolic links are rejected. Search treats query terms as plain text. Provider tokens are revoked after the turn and expire after six hours at the latest. The server rejects non-loopback Host headers and request bodies over 1 MB.

Tokens route requests and enforce application scope access. They do not isolate a hostile process with the user's filesystem access. Background memory consolidation is not implemented. No data migration is provided for this new application.
