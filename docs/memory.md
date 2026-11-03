# Agent memory

Memory is a set of markdown files. They are the source of truth. A SQLite FTS5 index makes them searchable,
and the index can always be rebuilt from the files.

## Layout

Under `$DISCARD_HOME` (default `~/.discard`):

| Path | Content |
| --- | --- |
| `memory/agents/<agent-id>/` | Private to one agent |
| `memory/projects/<project-id>/` | Shared by agents in one project |
| `memory/global/` | Shared by all agents |
| `memory-index.db` | FTS5 index. Safe to delete. Discard rebuilds it at startup. |
| `run/` | Per-session MCP config files (mode 0600, directory mode 0700). Deleted when a turn ends. |

Files under `core/` in a scope are **core blocks**. Discard adds them to the system prompt at the start of a
session. Other files are reached through the tools.

## Tools

The Claude session connects to a local MCP server (HTTP on `127.0.0.1`, random port, bearer token).

| Tool | Purpose |
| --- | --- |
| `memory_list` | List files in a scope |
| `memory_read` | Read a file. The result starts with the file version. |
| `memory_search` | Full-text search over the scopes the session may use |
| `memory_append` | Add a note. Creates the file if needed. |
| `memory_edit` | Replace one piece of text. Needs the version from `memory_read`. A stale version is refused. |

Discard approves calls to its own memory tools by policy (`mcp__discard-memory__*`). Each decision is written to
the approval audit table with the decider `Policy("builtin-memory-tools")`. All other tools still ask the user.

## Safety rules

- Scope IDs are canonical. A caller names `agent`, `project`, or `global`. Discard maps that to the IDs bound to
  the session token. Callers never pass paths for scopes.
- Paths are relative, end in `.md`, use only `a-z 0-9 . _ -` and `/`, and are at most 5 levels deep.
  Symbolic links are rejected at every level.
- Writes are atomic (temporary file, then rename). Writes are serialized.
- A file holds at most 64 KB. A scope holds at most 500 files.
- Search queries are plain words. FTS5 operators in a query are treated as text.
- Each provider session gets its own token (64 hex characters). The token is revoked when the turn ends, and it
  expires after 6 hours at the latest.
- The server refuses requests whose `Host` header is not loopback (DNS rebinding defense), and bodies over 1 MB.
- Text in a memory block is wrapped in a `<memory>` tag. A closing tag inside the text is escaped. The system
  prompt tells the agent that memory text is data, not instructions.

## Limits you should know

- **Session tokens are not a defense against a hostile agent with shell access.** An agent that can run
  commands as your user can read other session config files. The tokens route calls and give an audit trail.
  They do not isolate agents from each other at the operating system level.
- **A resumed Claude session keeps its first system prompt.** Claude records the system prompt once per
  conversation. Core blocks edited after the first turn do not change the prompt of that conversation. They
  appear in the next new session. The conversation history still holds what the agent saved earlier.
- Background consolidation of memory ("sleep-time" agents) is not built.
