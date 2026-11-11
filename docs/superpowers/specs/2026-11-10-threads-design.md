# Threads — design

**Date:** 2026-11-10
**Status:** Approved by the owner through the design interview on 2026-11-10.
**Builds on:** [Project, branch, channel, and agent orchestration](2026-11-05-project-channel-orchestration-design.md), [Agent workspace writes](2026-11-09-agent-workspace-writes-design.md)

## Intent

A thread is a side conversation that starts from one message in a channel. Threads have two uses:

- **Question threads.** The user asks a side question about a message. The main channel stays clean.
- **Sub-agent threads.** A sub-agent run gets its own transcript, apart from the parent channel.

A thread message can also be posted to the parent channel. Behaviour follows Discord where this document does not state a difference.

The work has two slices. Slice 1 is question threads. Slice 2 is sub-agent threads. This document specifies slice 1 in full. For slice 2 it records the decisions already made and the open questions.

## Current state

- No parent or child ids exist on channels or messages. `Channel` is `{id, project_id, name, agent_id, workspace_mode}` (`crates/discard-core/src/model.rs:124`). `Message` is `{id, channel_id, role, blocks}` (`model.rs:205`).
- Storage is SQLite (`store.rs`), with `user_version` migrations 1 to 10. Each channel has rows in `sessions`, `turns`, `events`, `approvals`, and `messages`.
- Read markers are per channel in `state.toml` `[last_read]` (`window_state.rs:71,144`).
- Conversation and Workspace sessions replay the saved transcript on each turn (`conversation_context.rs:55-121`). Coding mode uses native provider resume. No provider fork support exists.
- User messages can select one agent with `@name` (`mentions.rs`). The parser ignores model output (`docs/runtime/conversation-context.md:117`).
- Disabled "Start thread · Soon" controls exist in `MessageRow.vue:218-239` and `ContextMenuHost.vue:65-69`.
- `App.vue:781-1149` has three panels: navigation, main, and an optional right panel (240–400px) that holds `ContextPanel` or `AgentHistorySearch`. Narrow screens use left and right `Sheet`s.
- Sub-agents are off. Claude tools are limited to `Read,Glob,Grep,Edit,Write,Bash` (`claude/provider.rs:118`). Codex disables `agents` and `multi_agent` (`conversation.rs:114,127-128`). The Claude parser ignores `parent_tool_use_id`. The workspace-writes spec turns off delegation.
- Plugins can add menu items for the `message` and `tool_call` targets (`docs/plugins.md:132-138`). Plugins cannot render message blocks.

## Terms

- **Parent channel.** The channel that contains the anchor message.
- **Anchor message.** The message a thread starts from.
- **Thread.** A channel with a parent channel and an anchor message.
- **Posted message.** A thread message that also shows in the parent channel.
- **Followed.** A thread that shows in the channel bar.

## Decisions — slice 1, question threads

### Data model

1. A thread is a `channels` row. New columns:
   - `parent_channel_id` — the parent channel. Null for a normal channel.
   - `anchor_message_id` — the anchor message. Null for a normal channel.
   - `thread_kind` — `question` or `sub_agent`. Slice 1 writes only `question`.
   - `followed` — boolean.
2. A thread reuses `sessions`, `turns`, `events`, `approvals`, and `messages` without change.
3. A posted message is stored once, in the thread. The parent channel holds a reference row that points to it. The reference row takes its position in the parent at the time of posting.
4. Thread read markers stay in `state.toml` with the other read markers.
5. Threads are flat. A thread cannot be the parent of a thread.
6. An anchor message has at most one thread.

### Anchor rules

7. Any completed user or agent message can anchor a thread. This applies to all channel kinds, including direct messages. The direct-message "Question" concept in `docs/ui-design-reference/README.md` is separate and does not change.
8. "Start thread" is disabled on a message that still streams. This is a temporary state and needs no `TODO`.
9. "Start thread" is absent inside the thread panel.
10. After a thread exists, the action on the anchor message is "Open thread".
11. A reference row in the parent shows "Open thread" for its source thread. It cannot anchor a new thread.

### Creation

12. "Start thread" opens an empty thread panel that shows the anchor message. No row exists yet.
13. The first sent message creates the thread row. No empty thread exists in the database or the channel bar.
14. The thread name is the first thread message, truncated to about 40 characters. No name prompt shows.

### Agent and context

15. The thread agent is the author of the anchor message. If the anchor is a user message, the thread agent is the project main agent, resolved as `mentions::default_agent` does.
16. A user `@mention` in a thread message selects another agent for one response, as in channels.
17. Thread context is the parent transcript up to and including the anchor message, then the thread messages. Later parent messages do not enter the thread.
18. A question thread always runs in Conversation mode: read-only tools and transcript replay. The parent channel's mode does not change this. A Coding-mode parent gives the thread its saved transcript, not its provider session.
19. A thread turn can run while the parent channel turn runs.
20. Thread messages do not enter the parent agent's context. A posted message is the only exception.

### Post to channel

21. The thread composer has an "Also post to channel" checkbox. The default is off. It applies to one message.
22. Every thread message, including an agent reply, has a "Post to channel" action. The agent cannot post to the parent by its own decision.
23. The posted message shows in the parent channel with a link to the thread.
24. Parent replay includes a posted message with the label `[From thread "<name>"]` and its original role.
25. A posted message enters the parent context for the next parent turn. It does not start a turn.

### Channel bar and follow state

26. A followed thread shows indented under its parent channel. Selecting it opens the parent channel with the thread panel open.
27. A user-initiated thread starts followed.
28. "Leave" sets the thread to not followed and removes it from the channel bar.
29. A thread becomes followed again when the user replies in it or chooses "Follow".
30. The mention trigger (decision 44) is slice 2.

### Layout

31. The thread panel takes the right panel slot. It replaces `ContextPanel` while open. Closing the thread restores the context panel.
32. The thread panel size range is about 360–640px.
33. One thread panel is open at a time.
34. Narrow screens show the thread in the right `Sheet`.
35. A chip shows under the anchor message at all times. It carries the thread name, the message count, the last message preview, and the Follow action. The chip is the only entry point to a thread that is not followed.

### Unread

36. Each thread has its own read marker.
37. A followed thread shows unread state in the channel bar. A thread that is not followed shows unread state only on its chip.
38. Thread activity does not mark the parent channel unread. A posted message is a parent channel message and counts there.

### Plugins

39. Plugin menu items for the `message` and `tool_call` targets apply to thread messages.
40. The thread panel has no plugin slot in slice 1. Its header uses app components only.

### Planned actions

These controls are visible and disabled, with the label `Soon` and a `TODO` comment, per `AGENTS.md`:

- **Threads list** in the channel header. Missing: a list of all threads in the channel, including threads that are not followed.
- **Rename thread.** Missing: edit of the thread name.
- **Delete thread.** Missing: deletion, with rules for posted messages.
- **Remove from channel** on a posted message. Missing: removal of the reference row, with rules for a parent agent that already read it.

## Decisions — slice 2, sub-agent threads

Slice 2 needs its own design session. These decisions are fixed:

41. When an agent starts a sub-agent, Discard creates a thread with `thread_kind = sub_agent`. The anchor is the message that contains the spawning tool call.
42. The sub-agent transcript streams into the thread. The parent channel shows only the spawn chip and the final result.
43. A sub-agent thread is read-only for the user. Steering a running sub-agent is a planned action: visible, disabled, `Soon`, with a `TODO`.
44. A thread becomes followed when its agent mentions the user in plain text outside code and quotes, or when the thread has a pending approval. The text parser reuses the rules in `mentions.rs`.
45. A sub-agent that starts a sub-agent gets a sibling thread in the same parent channel.

Open questions for slice 2:

- Which session modes enable sub-agents.
- The change to the delegation rule in the workspace-writes spec.
- Provider parsing: Claude `parent_tool_use_id` and sidechain messages, and the Codex equivalent.
- The form of the final-result post in the parent channel.

## Out of scope

- Standalone threads with no anchor message.
- Auto-archive.
- Native provider session fork.
- Write access in a question thread.
