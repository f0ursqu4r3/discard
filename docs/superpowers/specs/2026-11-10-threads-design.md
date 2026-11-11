# Threads — design

**Date:** 2026-11-10
**Status:** Approved by the owner through the design interview on 2026-11-10.
**Builds on:** [Project, branch, channel, and agent orchestration](2026-11-05-project-channel-orchestration-design.md), [Agent workspace writes](2026-11-09-agent-workspace-writes-design.md)

## Intent

A thread is a side conversation that starts from one message in a channel. Threads have two uses:

- **Question threads.** The user asks a side question about a message. The main channel stays clean.
- **Sub-agent threads.** A sub-agent run gets its own transcript, apart from the parent channel.

A thread message can also be posted to the parent channel. Behaviour follows Discord where this document does not state a difference.

The work has two slices. Slice 1 is question threads. Slice 2 is sub-agent threads. This document specifies both.

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
30. The follow triggers (decisions 71–73) are slice 2.

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

None. Slice 4 implements the four actions that were planned here: the Threads list, Rename thread, Delete thread, and Remove from channel.

## Decisions — slice 2, sub-agent threads

Approved by the owner through the design interview on 2026-11-10.

### Facts that shape slice 2

- Provider-native sub-agents need one parser for each provider. Claude marks sub-agent lines with `parent_tool_use_id`; Codex reports `collabAgentToolCall` and `subAgentActivity` items on separate provider threads. Discard could not apply its approval rules or sandbox to them.
- The MCP handler has an `Identity` (agent, project, channel) but no access to `Channels` or `AppCore`, and no turn id or tool-call id (`crates/discard-mcp/src/identity.rs:17-28`).
- The response message id is created only when the message is saved at the end of the turn (`engine.rs`, `store_message`). `agent_commits.response` already handles this: it is null during the turn and set at the save.
- Claude aborts an HTTP MCP call that sends no response for 5 minutes, unless the server entry has a `timeout`. Codex `tool_timeout_sec` defaults to 60 seconds. Discard sets neither today.
- Codex `supports_parallel_tool_calls` is off by default. Parallel MCP calls are not verified for Claude.
- Replay cuts a tool output to 1024 bytes in later turns (`conversation_context::compact_blocks`).
- Request-style tools return "approval required" at once. On approval, Discard sends `Command::Continue` and the agent continues (`discard-mcp/src/tools.rs`, `engine.rs`).

### Mechanism and scope

41. Discard runs the sub-agent. The parent agent calls the Discard MCP tool `delegate`. Provider-native sub-agents stay off.
42. A sub-agent is for read-only research. It runs with the read-only Conversation profile, as every thread does.
43. Only agents start sub-agent threads. A user command to delegate is out of scope.
44. A sub-agent cannot delegate. Depth is 1. The sub-agent session has no `delegate`, `agents_list`, or `agent_request_add` tool.
45. Conversation and Workspace parent sessions get the tools. Coding mode does not. A thread session does not.
46. Each agent has `delegation: bool` in `AgentConfig`, false by default. Turning it on uses the policy-loosening confirmation. An agent without it does not get the tools. A `delegate` call shows no approval card.

### The `delegate` call

47. One call takes a list of 1 to 4 tasks. Each task has a `title`, a `brief`, and an optional `agent` name.
48. The tasks run at the same time. The call blocks until all finish, then returns one result for each task, in task order.
49. The default sub-agent is the calling agent's own configuration. A named agent must be active and a member of the project. In a direct message only the default is permitted.
50. If any task names an agent that is not a member, or is invalid in another way, the whole call fails before any sub-agent starts. The error names `agent_request_add` for a non-member.
51. A task result is the sub-agent's final message text. A failed, cancelled, or timed-out task returns an error text that states which.
52. A sub-agent turn has a limit of 10 minutes. At the limit Discard stops it and returns "timed out".
53. Discard sets the Claude per-server `timeout` and the Codex `tool_timeout_sec` for the Discard server to that limit plus a margin.
54. Replay keeps a `delegate` result up to 8 KB in later turns.
55. Stopping the parent turn stops its sub-agents. Stopping one sub-agent from its thread returns "cancelled" to the parent and does not stop the parent.
56. If the app quits during a run, both turns stop. The thread keeps its partial transcript and its state is "cancelled".

### The sub-agent thread

57. Each task gets a new thread with `thread_kind = sub_agent` and one sub-agent turn. A later `delegate` call makes new threads; it does not continue an old one.
58. The thread name is the `title`, cut to about 40 characters. An empty title uses the start of the brief.
59. The brief is the first message of the thread. Its author is the parent agent. The sub-agent receives it as its request.
60. Sub-agent context is the brief, the normal project context, and the sub-agent's own memory. The parent transcript is not replayed.
61. The sub-agent has the same tools as a normal read-only Conversation response of that agent, including memory save.
62. The thread has no anchor during the run. At the end of the parent turn the engine sets the anchor to the saved response message. If the parent turn saves no response, the anchor is the user message that started the turn.
63. "One thread for each message" applies to question threads only. A response can hold several sub-agent threads and one question thread.
64. The "parent must be the active channel" check does not apply to sub-agent threads.
65. A sub-agent thread is not followed at creation. It shows in the channel bar only after a follow trigger or the Follow action.
66. A sub-agent thread has a state: running, done, failed, cancelled, or timed out.

### UI

67. The chip shows the title, the sub-agent's name, and the state. During the run the chips show on the live message. After the turn they show under the saved response, in task order.
68. Opening the chip shows the thread panel with the live transcript.
69. In a sub-agent thread, Post to channel, Copy, Quote, Follow, and Leave work. The composer is disabled, labelled `Soon`, with a `TODO`: steering a sub-agent and follow-up turns. Slice 3 enables the composer (decisions 81-93).
70. The read-only note covers both kinds: "threads are read-only".

### Follow triggers, all thread kinds

71. A thread becomes followed when it has a pending approval.
72. A thread becomes followed when a saved agent message in it contains `@you` in plain text outside code and quotes. The parser reuses the rules in `mentions.rs`. The thread agent's prompt states the `@you` syntax.
73. Thread approvals show on the Approvals page and in the pending-approval counts.

### Agents outside the project

74. `agents_list` returns each active agent's name, provider, model, member flag, and the first 200 characters of its personality prompt.
75. `agent_request_add(name, reason)` returns "approval required" at once and shows an approval card in the parent channel.
76. The card shows the requesting agent, the requested agent with its provider and model, and the reason as a quote. The choices are "Add to project" and "Deny".
77. "Add to project" makes the agent a normal project member with no end date, and continues the parent agent.
78. "Deny" continues the parent agent with the note that the user declined. A repeat request for the same agent in the same channel is refused until the next user message.
79. In a direct message `agent_request_add` returns an error.

### Spec change

80. `2026-11-09-agent-workspace-writes-design.md` decision on the tool set: provider-native sub-agents stay off. Discard delegation to read-only sub-agents is on.

### Out of scope for slice 2

- Sub-agents that write.
- Nesting.
- Follow-up turns and steering. Slice 3 covers them (decisions 81-93).
- A user command to delegate.
- An agent description field.
- Settings for the limits.

## Out of scope

- Standalone threads with no anchor message.
- Auto-archive.
- Native provider session fork.
- Write access in a question thread.

## Decisions — slice 3, steering and follow-up turns

The owner skipped the design interview for this slice. The owner approved decisions 83 and 85 on 2026-11-10. The other decisions are the designer's calls.

### Follow-up turns

81. A user message in a sub-agent thread with a final state (done, failed, cancelled, timed out) starts a normal thread turn, as in a question thread. The turn is read-only and uses the Conversation profile. The replay holds this thread only: the brief, the answers, and later messages. The parent transcript is still not replayed. An `@mention` of another agent selects that agent for one response. "Also post to channel" and attachments work. The message follows the thread.
82. A follow-up turn does not change the thread state and returns nothing to the parent agent. The `delegate` call already returned. The chip keeps its final state.

### Steering

83. A user message in a sub-agent thread whose turn runs is a steer. The turn that runs is the brief turn or a steer turn. The engine stops the provider turn, keeps the partial response that a cancelled turn normally saves, saves the user message, and starts a new turn with it at once. The replay gives the sub-agent the brief, its partial work, and the steer message.
84. A steer does not end the task. The thread state stays `running`. The engine sends no `Update::SubAgentFinished` for the interrupted turn, and the `delegate` call keeps waiting. The task result is the final message of the last turn.
85. The 10-minute limit counts from the `delegate` call. A steer does not extend it.
86. A stop (`Command::Cancel`: the user Stop, a parent stop, or the timeout) that arrives after the engine accepted a steer and before the steer turn starts drops the steer. The task ends as a normal cancel. A stop during the steer turn cancels that turn and settles the task as before. A state that is already final in the store, for example `timed_out`, stays.
87. A second steer while a steer is pending replaces nothing. The first accepted steer runs. The engine answers the second with the existing "A turn is already running" error. A steer during a later steer turn is allowed, by the rule in decision 83.
88. Steering applies only to sub-agent threads, and only while the task runs. A question thread keeps the "turn is already running" refusal. A message during a follow-up turn gets the same refusal.
89. A second brief is still refused. `SendBrief` in a thread that has messages is still refused.
90. A steer or follow-up message is a user message. It follows the thread (`follow_and_post`), and `also_post` works.

### Prompt

91. The sub-agent system prompt note says that later messages in the thread come from the user. If one arrives while the sub-agent works on the brief, it follows that message and still ends with one final message that stands alone, which goes back to the agent. After the result returns, replies stay in this thread.

### UI

92. The composer in a sub-agent thread is on, unless the thread is blocked for the existing read-only reasons. The placeholder is `Steer this sub-agent` while the task state is running, and `Reply in thread` after. While the task runs, the composer shows Send and Stop. Send is on when the composer has text. The `steering_soon` reason is gone from the Rust DTO, the TS type, and all users. The `Soon` label is gone from the thread composer. The `soon` prop is gone from `Composer.vue`, because no other caller used it.
93. The four `TODO` comments about steering are gone, and so are the three refusals they guarded.
94. When the user sent one or more messages to a sub-agent during its task, the `delegate` result for that task starts with a note, then a blank line, then the normal result text. The note has this form: `Note: the user sent this sub-agent <N> message(s) during the task. The result follows the user's instructions, which can differ from your brief. Latest user message: "<text>"`. `<N>` is the count of user messages saved in the thread during the task. The brief is written by the parent agent and does not count. `<N> message(s)` reads `1 message` or `2 messages`. `<text>` is the plain text of the latest such message, cut to 500 characters with `…` when cut, with `"` replaced by `'`. The note applies to every final state: done, failed, cancelled, and timed out. For a state other than done, the normal text is the existing error text. A steer that the engine accepted but dropped before it was saved (a stop between turns, or a failed turn) does not count. With no steer, the result is unchanged. A follow-up turn after the task ended never changes a result: the call already returned. The note is part of the tool result only. It is never a saved message, so the thread transcript does not change. The replay limit of decision 54 stays at 8 KB, and the note counts toward it.

### Implementation notes

- A steer stops the turn like Stop does. The engine keeps the steer in its run state, and `run_steers` runs it after the stopped turn ends. The interrupted turn sends no `TurnFinished`, so the UI keeps its running turn. The engine shows the steer with `Update::UserMessageAdded`, after the partial response.
- A steer cannot `@mention` another agent. The engine refuses it with a message, and the turn goes on.
- A message that reaches the engine before the brief turn starts is refused: there is no turn to steer.

### Out of scope for slice 3

- Sub-agents that write.
- Nesting.
- A user command to delegate.
- Provider-native mid-turn injection.
- A change to the timeout.
- Thread menu actions.

## Decisions — slice 4, thread actions

The owner chose the posted-message rule of decision 97 and the warning rule of decision 98. The other decisions are the designer's calls. Decision 99 (rename of a text channel) is in [Channel bar sections, text channels, and Reply](2026-11-10-channel-bar-and-reply-design.md).

### Threads list

95. The Threads button in the channel header opens a popover. It lists every thread of the channel: question threads and sub-agent threads, followed or not. The order is latest activity first. The latest activity is the newest user or agent message of the thread, or the creation of the thread while it has none. A question thread that has no message is not listed. Each row shows the name, the kind (Question or Sub-agent), the run state of a sub-agent thread, the message count, an unread mark, and a Follow or Leave control. A click on a row opens the thread panel and closes the popover. A channel with no thread shows "No threads yet." The button is enabled wherever it shows. The snapshot carries the list (`channelThreads`).

### Rename

96. Rename thread is in the thread panel menu and the thread row menu. The dialog has one name field with the current name. The app trims the name, changes line breaks to spaces, and accepts 1 to 80 characters. Duplicate names are allowed. The rule applies to question threads and sub-agent threads. The new name shows on the chip, the channel bar row, the panel header, the Threads list, the reference rows, and in the replay label `[From thread "<name>"]`. The label always uses the current name. An unchanged name closes the dialog with no write.

### Delete

97. Delete thread is in the same two menus. A confirm dialog opens with Cancel in focus. The item is disabled, with no `Soon` label and no `TODO`, while a turn runs in the thread, while its sub-agent task state is `running`, and while it has a pending approval. The store and the app check the same rules again when the user confirms. The delete is one transaction:
    - Each message of the thread that was posted to the parent channel becomes a normal message of the parent. It keeps its role, author, content, attachments, and its row, so it stays at the place of its reference row. It has no thread link and no `[From thread …]` label after that. The parent agent's context does not change for a posted message.
    - Then the app removes the thread with its sessions, turns, events, approvals, messages, and read marker (`state.toml`).
    - The dialog states how many posted messages stay in the channel when the count is above zero.
    - If the panel shows the thread, it closes. The chip under the anchor message goes. A loaded provider session of the thread closes before the rows go. The anchor message can start a new thread.

### Remove from channel

98. Remove from channel is in the hover toolbar and the context menu of a posted row. It removes the reference row from the parent channel only. The message stays in the thread, and Post to channel works on it again. A confirm dialog always shows. When an agent message follows the row in the parent channel, the dialog adds a warning: the agent already read the message, earlier replies can still refer to it, and later turns will not see it. Another reference row does not count as an agent message. The row goes in both cases. If the replay summary of the channel ended at the row, it ends at the row before.

### Unchanged

100. Mute Channel and Archive Channel… stay planned: disabled, `Soon`, with their `TODO`. The Git branch and worktree items stay planned.

### Shared rules

101. The `TODO` comments and planned entries of the five implemented actions are gone. The shared planned-item model stays for the planned items that remain. An action that a state blocks is disabled without `Soon`.
102. A backend error shows as a toast. The dialog stays open and the input stays as the user typed it.

### Implementation notes

- All writes are Tauri commands with checks in Rust: `rename_thread`, `delete_thread`, `remove_posted_message`, and `rename_text_channel`. `thread_posted_count` is a read command for the delete dialog. The web checks are for feedback only.
- One host (`ThreadActionDialogs.vue`) shows the four dialogs. A control sets `pendingAction` (`thread-actions.ts`), so no menu needs an event chain.
- In a confirm dialog, Enter activates the button that has the focus. Cancel has the focus when the dialog opens, so Enter confirms only when the focus is not on a button. Escape and Cancel close the dialog.
- The delete keeps the reference row and copies the content of its source into it. The row ID does not change, so read markers and replay summaries stay valid.
