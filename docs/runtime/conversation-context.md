# Conversation runtime

Regular agent chats use `SessionMode::Conversation`. The deliberate `--builder` launcher uses Coding mode. Coding keeps provider-native resume and workspace behavior.

Before each conversation response, Engine loads the saved agent and conversation. An archived or deleted agent cannot start a response. The configured provider must be available and the model must be explicit. Engine holds that snapshot until the response ends. Provider/model changes keep the transcript and add a saved system message.

Conversation mode builds fresh provider input from this conversation's messages, the saved personality, and current applicable memory. It supplies a native resume handle only for an agent with **Keep provider session** on (see [Native provider resume](#native-provider-resume)). Memory receives the actual optional project and source conversation ID. Conversation mode disables plugin-authoring tools and skips git worktree preparation. Provider adapters create their own empty working directory, except for a resumable session, which uses a stable Discard-owned directory.

Memory instructions and preferences always enter the prompt in domain precedence order. Other live items use word overlap with the current request and the two latest messages in this conversation. Matching uses Unicode alphanumeric words of at least three bytes, normalized to lowercase. This is a simple lexical retrieval policy. The scoped memory search/read tools remain available for further retrieval. Current user instructions take precedence over memory. Capture guidance and the current capture-pause state enter each response prompt.

## Native provider resume

An agent with **Keep provider session** on (`AgentConfig.native_resume`, off by default) keeps one provider session for each channel and agent. The next response continues that session and sends only what is new. The provider stores the conversation on disk, outside Discard's control. Discard never deletes the provider's stored sessions, also when it drops a handle, turns the setting off, or deletes a channel or agent. The feature is for Conversation and Workspace responses in a channel. Thread turns, summary sessions, and the `--builder` Coding path never resume. If the engine cannot find a stable session directory or a readable MCP config, the response replays as usual.

### Session record

The table `agent_sessions` has one row for each (channel, agent). It holds the provider handle, the session fingerprint (as JSON), `delivered_through` (the last saved message the session has seen), the memory items given so far (item ID to version), the topic version, the capture-pause flag, `bytes_in_session`, the session directory, and `dropped_reason`. Deleting or archiving a channel, or deleting the agent, deletes its rows. Turning the setting off deletes the row at that agent's next response in that channel. A change of provider or model replaces the row: the next response replays and saves a row for the new session. A Workspace response that falls back to a read-only session because the sandbox is unavailable deletes the row, and the context panel shows "Fresh replay: the workspace sandbox is unavailable, so this response is read-only".

### Fixed prompt and per-turn context

The system prompt has a fixed part and a volatile part. The fixed part holds the personality, fixed instructions, repository instructions, the tool-naming lines, the delegation line, and the core memory blocks. Its hash is in the fingerprint. The volatile part holds the worktree approval outcomes, the "no worktree" line, main-checkout availability, and the Primary / Moderator move line. Discard sends it every turn, with the worktree snapshot and the project context. For an agent with the setting on, the volatile lines go in a labelled `<session-state>` block that replaces every earlier one. The block says `none` when there are no lines, so a line that disappears (for example, after another agent becomes the moderator) is withdrawn from the session. The snapshots have a label that says they replace earlier snapshots, or that there is none. Agents with the setting off get the same input as before.

Discard sends the fixed system prompt on every resume. Claude does not store it. Codex freezes developer instructions at thread start, and the fixed-prompt hash guarantees that they equal the current ones.

### What a resumed response sends

1. Every saved non-system message after `delivered_through`, in order, framed and compacted as replay frames it. The session's own replies up to `delivered_through` are not sent again. A reply after a message posted during its turn is sent once more (see below).
2. A `<context-update>` block, only when something changed: memory items that are new or have a new version, items that were forgotten, a new topic, and a changed capture-pause flag. Memory selection is the same as for replay. A selected item is sent when the session has not seen its current version. A delivered item that changed and is not selected now is withdrawn with a `<changed-memory>` element; its new text goes in when it is next selected. Text in the block is escaped.
3. The labelled session state, the worktree snapshot, and the project context.
4. The current user message.

After a completed response, Discard saves `delivered_through` as the end of the contiguous run of replay chunks that the session has seen: messages that existed at the start of the turn, the user message, and the saved reply. A message that another author posts during the turn stops the run, so the agent's own reply is sent once again in the next delta. This is a known limit. It never loses a message.

### Fingerprint

A handle is usable only when the saved fingerprint equals the current one. The fields are: provider, model, reasoning effort, session mode, session directory, the hash of the fixed prompt, the list of Discard MCP tools, the delegation flag, the sandbox flag, the autorun flag, the provider CLI version, the summary boundary, and a digest of the replayed history through `delivered_through`. The digest changes when a message at or before that point is edited, deleted, or removed. The first differing field names the reason in the notice.

A new, changed, or forgotten `instruction` or `preference` memory item changes the fixed prompt, so it drops the handle. Other items go in the context update.

### When Discard drops the handle

- The fingerprint differs.
- The previous response was cancelled, failed, or did not save its reply. A steer does not count. Before a resumed response starts its provider run, Discard marks the row "the previous response did not finish". A completed response clears the mark. So an app exit, a crash, or a stopped engine during the response also drops the handle.
- The provider rejects the handle. Claude reports this on the first send. Codex reports it at start. Discard replays once in the same response and never loops.
- The saved bytes plus the new input exceed the input budget for the model (the saved bytes count each reply with its full tool outputs, as the provider session holds them), or the response failed with an error that reads like a context limit. The check is a text heuristic on the provider error.
- The provider compacted the session. Discard turns provider compaction off where it can (`DISABLE_AUTO_COMPACT=1` for Claude, a very high `model_auto_compact_token_limit` for Codex). The adapters still detect a compaction event. The session stays saved, and the next response replays.
- A Codex thread resumes on a model other than the selected one.

A drop shows a notice that names the reason, for example "Fresh replay: the model changed." The replay saves its new handle.

### Summary headroom

The rolling summary is stored per channel, not per agent. When a native agent's replay summarizes down to about 50% of the budget, the shorter history also applies to agents in the same channel that replay.

A replay that summarizes history normally fills the budget. A resumed turn would then exceed the budget and replay again. For an agent with the setting on, a summarizing replay shrinks the input to half of the budget (`conversation_context::summary_target`). Other agents keep the full budget. A new summary changes the fingerprint, so each summary costs one replay.

### Indicator

The context panel shows "Resumed session" when the response continued a saved session. It shows "Fresh replay: <reason>" when the response replayed after a drop. A first session and an agent with the setting off show nothing. The state stays until the next response starts.

## Codex model-cache recovery

Conversation and Workspace setup check the installed Codex version before using model metadata. If the cache is missing, malformed, from another version, or missing the exact selected model, Discard makes one bounded `model/list` probe with the same executable and Codex home. The CLI refreshes its own cache. Discard then reloads and validates it before building the restricted conversation catalog.

The probe starts no conversation thread and sends no user prompt. A valid cache skips recovery. Recovery never changes the selected model or relaxes conversation controls. If refresh fails or the refreshed cache is still invalid, setup reports an error and remains available for retry.

## Project channel context

Selecting a project opens its saved active channel, or `general` on a first visit. Channel and project navigation preserve in-flight responses and continue receiving their updates in the background. Folder changes and project removal still require agents to stop. The project main agent is saved. If a project has no main agent, first setup uses the first active agent with an explicit model in Store order. Direct conversations keep their own selection and do not change the saved project channel.

Project channels have durable IDs and metadata. A branch resolves to its canonical channel, so reopening it reuses its transcript even when another direct conversation is active. Store branch-rename operations preserve channel identity, and Store retirement operations release a name for later reuse. App detection of Git branch renames and user-facing branch retirement are not implemented. Migration preserves old messages and sessions, chooses a canonical channel from each branch and `general` group that has an active candidate, and keeps duplicate histories as linked archives. If every candidate is archived, migration leaves no active canonical channel for that group.

Each channel can have a versioned plain-text topic. Topic edits use an expected version and retain earlier revisions with the editor kind. Topics are metadata; they do not change transcript messages or task state. Before each Conversation-mode response, Engine reads the latest topic and adds it as labelled context data outside the transcript. The prompt states that a topic is not a tool request and that explicit system instructions and the user's message take precedence. Channel membership is saved. Membership alone does not start a response.

Selecting a project or branch channel does not switch Git branches or create a worktree. Normal Conversation mode still uses the provider adapter's empty working directory. Each unmentioned project message uses the current project main agent, including messages in existing branch and detached worktree channels. The channel's saved agent is a fallback when no project main agent exists. Each response keeps the configuration selected when it starts; a default change affects the next response.

## Project files in conversations

Before each Conversation-mode reply, Discard resolves the project from the saved conversation. It supplies
the project name, channel, folder, a bounded file list, and a README excerpt. A saved worktree takes precedence
over the project folder. Without a worktree binding, the context explicitly describes the project folder's
current checkout. It does not claim that this checkout matches a branch channel's name. Direct chats without
a project receive no project context. Sidebar selection and the provider's empty working directory do not
change this scope. The context counts toward the input budget and never becomes a transcript message.

The scoped MCP server provides `project_list_files`, `project_read_file`, `project_search`,
`project_git_branches`, `project_git_log`, and `project_git_compare` for follow-up
questions. These tools validate the token's project and conversation on each call. General channels can use
them without a worktree binding. Worktree-only tools are not listed for general channels. Summary sessions
and plugin tokens cannot use project tools. File content is reference data, not agent instructions.

File tools accept relative paths. They reject parent traversal, symlinks, special files, hidden paths except
`.github`, common build directories, and key files. Git projects expose tracked and nonignored files.
Other folders use a bounded directory walk. File reads accept UTF-8 text up to 256 KiB and return up to
200 lines or 8 KiB per call. Listing supports offsets. Search defaults to literal, case-sensitive text and scans
up to 100 files with up to 40 matches or 8 KiB of excerpts. Results report continuation, skipped files,
and partial search results. Large directory listings ask for a narrower directory.

`project_search` defaults to literal, case-sensitive matching. Set `ignore_case` for
case-insensitive matching or `regex` for Rust regular expressions. Both options can be combined.
Git tools inspect local branches, worktrees, commit history, and three-dot comparisons.
They resolve refs before use, never contact remotes, and return at most 8 KiB of untrusted repository data.

The prompt asks the agent to answer from supplied facts, use project tools for missing details, and retain
conversation context for follow-ups. It should not ask users to paste accessible files or treat an empty
generic MCP resource list as proof that the project is unavailable.

## Worktree snapshots

Before each Conversation-mode reply in a channel with a saved worktree binding, Discard runs bounded read-only
Git queries in that worktree. The prompt includes the saved path and repository, current HEAD and file status,
five recent commits, and up to 12 local or cached remote refs that contain HEAD. The configured model provider
receives this snapshot as context data. Direct chats and channels without a worktree binding receive no snapshot.

The snapshot follows the stored conversation binding, never the selected sidebar project or provider working
directory. Each query has a two-second timeout and a 4 KiB output limit. Failed or oversized queries report an
unknown state. Git hooks, filesystem monitors, and optional lock writes are disabled. The queries do not fetch,
change files, switch branches, or create worktrees. Ignored files and submodule contents remain uninspected.

Prompt guidance asks the agent to state the observed worktree state, suggest a useful next step, and explain
missing evidence. It identifies repository strings as untrusted data. The snapshot does not grant shell or Git
mutation tools. The snapshot counts toward the input budget and is not inserted into saved transcript messages.

## Threads

A thread is a channel row with `parent_channel_id`, `anchor_message_id`, `thread_kind` (`question` or `sub_agent`) and `followed`. A sub-agent thread also has `thread_state` and `origin_message_id`. It inherits its parent's project and has its own session, transcript, summary state and read marker. A parent that is itself a thread cannot start a thread. A message that was posted from a thread cannot start one. Each message anchors at most one question thread. A response can anchor several sub-agent threads, in addition to its question thread (the unique index covers question threads only). Threads never appear in `channels_for_project`, `all_channels_for_project`, `all_conversations`, `conversations_for_agent`, the saved project channel, or direct-history search.

A thread always runs as Conversation mode with the read-only profile and transcript replay. The process `SessionMode` (including Coding) and the parent's write access do not change this. A thread turn gets no workspace prompt, no autorun, no main-checkout request tool and no resume handle. The read-only note says that threads are read-only, for both kinds.

A thread reads the files of its parent channel. `Store::binding_channel` gives the channel whose binding a read uses: the parent for a thread, the channel itself otherwise. `ProjectContext::for_channel` resolves the root from that channel's worktree record, or from the project folder when the parent has none, and refuses a thread of an archived parent. The prompt names the checkout `worktree of the parent channel`. The Discard MCP `project_*` read tools (`project_list_files`, `project_read_file`, `project_search`, `project_git_*`) call `WorktreeAccess::project_context`, so a thread sees the same files as a read-only response in its parent, including uncommitted edits. Only read tools use the parent's binding. A thread has no worktree row, no branch binding, no workspace grant and no worktree lock of its own, so `WorktreeAccess::authorize` and `GitWorkspace::write_request_target` fail for it: it gets no `worktree_inspect`, `worktree_request_removal`, `worktree_commit`, `worktree_request_creation` or `main_checkout_request_write` tool, and no worktree snapshot. The worktree snapshot prompt (`worktree_context::for_channel`) still uses the channel's own binding.

A question thread answers as the author of its anchor message when that author is an active agent. Otherwise it uses the parent's default agent. A user `@mention` still selects another agent for one response. A sub-agent thread answers as its own agent.

Question-thread replay is the parent channel's messages up to and including the anchor, then the thread's messages. Sub-agent replay is the thread's own messages only: no parent prefix (`Store::channel_history`). Later parent messages never enter it. The thread saves its own `ConversationContext`. `through` can point into the parent part of the replay. That part never changes after the thread exists, so the boundary stays valid. The parent's own summary is not reused.

Posting a thread message stores no copy. The parent gets a reference row (`messages.source_message_id`) at the time of posting. Loading the parent returns the reference with the source's role, blocks and author. `Store::posted_sources` maps each reference to its source message and thread. Posting the same message again returns the existing reference. A parent replay shows each reference at its position with the label `[From thread "<name>"]` and the source's role. Quotes in the name become apostrophes. Thread messages that were not posted never enter the parent replay. Posting starts no parent turn. The posted message is part of the parent context from the next parent turn on.

`Command::SendInThread { text, attachments, also_post }` sends a user message in a thread. With `also_post`, the engine posts the saved message right after saving it and sends `Update::MessagePosted`. Any user message sent in a question thread sets `followed`; a sub-agent brief does not. `Send` and `SendAttachments` never post. `also_post` is an error outside a thread.

A thread has its own engine, session and event sequence. A thread turn and a parent turn can run at the same time. They share no lock.

### Follow triggers

Every thread kind becomes followed (`Store::set_thread_followed`, then `Update::ThreadFollowed` on the thread's channel) when:

1. an approval request that waits for the user arrives in a thread turn (an approval that policy decides does not count), or
2. an agent message saved in the thread has `@you` in plain Markdown text, in any letter case. `mentions::addresses_user` uses the masking rules of the mention parser: inline code, fenced and indented code, quotes, escaped `@`, and `mail@you.com` do not count. Only Markdown blocks are read.

Thread system prompts say it in one sentence (`engine::AT_YOU_NOTE`). A thread that is already followed sends no update. The app reads the thread again on `Update::ThreadFollowed`.

## Sub-agent threads and delegation

An agent with `AgentConfig.delegation` (off by default; turning it on needs the same user confirmation as other policy loosening, `AgentChange::SetDelegation`) gets three tools on the Discard MCP server in Conversation and Workspace responses. Coding mode, thread sessions of either kind, and summarization never get them. The engine sets `MemoryOptions::delegation`; `MemoryRuntime` turns it into `Identity::delegation` only when it was started with `DelegationServices`. Provider adapters allow the tool names only when the session's MCP config carries the delegation `timeout`, which Discard writes exactly for these sessions.

- `delegate({tasks:[{title, brief, agent?}]})` takes 1 to 4 tasks and blocks until every task has a final state. It returns `{"results":[{task, thread, state, result | error}]}` in task order. `error` says failed, cancelled, or timed out. `delegation::plan` checks the whole call first, so one invalid task fails it before any sub-agent starts: the calling agent needs `delegation`, the caller must not be in a thread, each brief is non-empty, a named agent must be active with a model, and in a project channel a named agent must be a member (the error names `agent_request_add`). In a direct message only the calling agent is permitted.
- `agents_list()` lists active agents: name, provider, model, member flag, and the first 200 characters of the personality prompt.
- `agent_request_add({name, reason})` returns `approvalRequired` at once and records an `AgentAddRequest` in `AgentRequests` (ephemeral, like `WriteRequests`). It is an error in a direct message, for an unknown, archived, or member agent, and for an agent the user declined since their latest message. `AgentRequests::decide` adds the agent with `Store::invite_agent_to_channel` (approve) or records the refusal, and returns a `FollowUp`. The app sends it to the parent agent as `Command::Continue` (added) or `Command::ContinueDeclined` (declined).

The MCP server serves calls concurrently. `delegate` holds the store lock only while it validates. The `Delegator` comes from a `DelegatorSlot`, which starts empty and which the app fills after its channels exist; a call before that returns "not ready".

Each task gets a new thread (`Store::create_sub_agent_thread`): `thread_kind = sub_agent`, `thread_state = running`, not followed, no anchor, and `origin_message_id` set to the parent's latest user message. The brief goes in as `Command::SendBrief { text, author }`. The engine saves it as the first message, a user message whose author is the delegating agent, and the thread's agent answers it. The text never selects a recipient. A sub-agent thread takes this one request: any other message, and a second brief, are refused. The turn is a read-only Conversation response with no delegation tools.

A `delegate` call belongs to a running turn of its parent. The app refuses it when the parent channel has no running turn, and when the parent already runs 4 sub-agents (`MAX_TASKS`), counting the tasks of other calls and the tasks of this call that did not start. Both checks happen before any start, and the error says so. `SubAgents::begin_call` makes the check and the reservation in one step, so parallel calls cannot pass the limit together. Stopping a parent turn (`cancel_in`, `cancel_all`) stops its running tasks and records the stop for the parent until `TurnFinished` or `SetupNeeded` ends the turn. A task registered later in the same call starts with its stop requested. Before it creates each task, the call checks the recorded stop and that the parent turn still runs. A task that did not start returns `cancelled` and has no thread, so no chip stays `running`.

At the end of the parent turn the engine calls `Store::anchor_sub_agent_threads`: the threads of that turn take the saved response as their anchor, or the user message that started the turn when no response was saved (`Update::ThreadsAnchored`, sent after `ResponseCompleted`). A thread left over from an older turn falls back to its own `origin_message_id`.

The end of a sub-agent turn sets `thread_state` (`done`, `failed`, `cancelled`) and sends `Update::SubAgentFinished { state, text }` just before `TurnFinished`. `Store::finish_sub_agent_thread` changes only a running thread, so the first final state wins: the delegator sets `timed_out` before it stops a turn at the 10-minute limit (`delegation::SUB_AGENT_TIMEOUT`), and the engine's `cancelled` then changes nothing. `text` is `delegation::final_text` of the thread's last agent message: all its Markdown, code and diff blocks in order, joined with blank lines, with tool calls left out. It is cut at 32 KB (`MAX_RESULT_BYTES`, on a character boundary) with a note that the full text is in the thread. When the engine cannot read the thread row, it still sets `failed` where it can and sends `SubAgentFinished { state: Failed }` with the error text, so the waiting call does not run to its timeout. The registry of tasks (`SubAgents`) learns of `SubAgentFinished`, of `Error`, `ResponseSaveFailed` and `SetupNeeded` texts (kept as the detail of a task that does not finish), and of a closed update stream, even when the thread runtime is no longer loaded. A start that fails after the thread was added (the brief cannot be sent) drops the parked runtime and stops its engine; the thread stays `failed` in storage. `Update::ThreadsAnchored` sets only the anchor of a loaded thread record: it never replaces a final state with `running` and never clears `followed`. Opening the store (`Store::open`) turns every thread still `running` into `cancelled`.

The provider tool timeout for the Discard server is the limit plus a 60-second margin (`delegation::provider_tool_timeout`). Claude reads it as the server entry `timeout` (milliseconds). Codex reads `tool_timeout_sec` (seconds), which the Codex adapter derives from it.

Replay keeps a `delegate` tool result up to 8 KB instead of 1,024 bytes (`conversation_context::DELEGATE_REPLAY_BYTES`). The tool is matched by the last `__` segment of its name.

## Context budget

The app counts one UTF-8 byte as one input token. This deliberately overestimates ordinary text token use. With provider metadata, the app uses the exact selected model's limit, capped at 131,072 tokens. Without metadata, the app uses 32,768 tokens. It reserves 8,192 tokens for output and protocol overhead. The unknown-model input budget is therefore 24,576 bytes. This app budget is an estimate; the provider can still return a context-limit error.

Required current input, personality and selected memory must fit. The app also reserves 4,096 bytes for a history summary and 512 bytes for framing. Oversized required input returns an error with three actions: shorten the request, edit applicable memory, or select a larger model. The app does not truncate that input.

The app splits saved messages into stable 4,096-byte fragments at UTF-8 boundaries. When history does not fit, the selected model summarizes bounded batches. Each request includes the previous rolling summary. The prompt asks it to preserve decisions, constraints, corrections and unresolved questions. The output must be nonempty and no more than 4,096 bytes. The app saves the summary and fragment boundary only after all batches needed for the response succeed. It keeps every original message. It saves a visible summary notice when the boundary advances and shows a notice whenever the response uses a summary.

Summary requests use fresh Conversation sessions and separate read-only memory tokens. They have no plugin server, native resume, or write authority. The runtime rejects tool activity from a summary model. Cancellation, timeout, provider errors and oversized summary output cancel the summary session. Token cleanup uses an RAII guard. Setup has a 60-second timeout; summary generation has a 120-second timeout. A failed summary leaves the previous saved boundary unchanged. Cancellation returns a notice and a Cancelled turn outcome. It does not report a provider error.

## UI integration

`Store::conversation_context(channel)` returns `ConversationContext { provider, model, summary, through }`. `through` identifies a stable replay fragment (`<message-id>:part:<index>`), not a separate transcript message.

`Update::ConversationMarker(Message)` adds saved model-change/summary system messages. `Update::ResponseCompleted(Message)` gives the UI the exact persisted response ID. `Update::ResponseSaveFailed(String)` reports failed response persistence. UiState keeps the live reply available for copying until the next send. The runtime emits no saved response ID or response-memory linkage for that failure. A failed user-message insert prevents the provider turn.

`Update::MemoryChanges { response, changes }` supplies new typed Saved/Proposed records. `Store::response_memory(channel)` returns saved `(MessageId, MemoryChange)` pairs for reload and memory controls.

The runtime polls change records before and after a response. It records only changes absent from the starting snapshot, then deduplicates by channel and stable change key. It excludes Saved records with `MutationOrigin::User`, so edits from the user during streaming do not appear as assistant saves. Saved records retain their typed `Automatic` or `Approved` origin. The UI must label Approved records as approved updates, rather than automatic saves. Proposed records remain typed proposals. Memory changes can still attach to an empty response if a model saves memory without producing text. It never approves a shared-memory proposal. Application approval code must validate the proposal's agent is still active before calling MemoryStore approval APIs.

Conversation deletion removes saved summary state and response-memory records before messages and channels. Retained conversations keep both records. Private-memory deletion remains separate from transcript deletion.

Desktop `AppCore::active_context()` returns the active agent and optional conversation project under one lock. Engine plugin events capture their source context. Slot, action and menu callbacks pin the captured context. The sidebar's selected project must not supply direct-chat memory scope.

Normal startup creates no project, agent, conversation, or bootstrap workspace. It restores the independently saved conversation when that conversation still exists. With no saved conversation, the app opens agent navigation. Creating an agent creates its default direct conversation. Additional direct and project conversations use an explicit agent and a fixed optional project. Selecting a sidebar project activates it and opens its saved channel; the selected project does not change direct-chat memory scope. The deliberate `--builder` path retains Coding bootstrap. Agent forms and memory controls apply edits without restarting; active responses finish with their original configuration.


Changing a project folder restarts only cached transports for conversations in that project. Each conversation keeps its transcript, retained unsaved reply, notice, and selection state. Direct conversations and other projects keep their transports. Each transport has a channel-specific generation token; updates from a replaced transport cannot overwrite retained state. The project-change guard blocks sends during the change. A failed launch leaves the affected transport closed. Selecting that conversation still opens its retained history and retries the launch. If the retry fails, selection succeeds and the shared conversation-opened notification opens the transcript. The transcript shows an actionable restart notice beside the existing unsaved-reply notice. Repeated identical failures do not duplicate the notice. A failed history selection remains an error and does not emit an open notification.

## Approval receipts and response notices

An approval commits an immutable `ApprovedSnapshot` in the authoritative item document. This flat metadata record contains the approved content, scope, origin, version, source, identifiers, timestamps, and kind. It contains no nested MemoryItem, approval snapshots, or copied history. The item's ordinary history retains its revisions. Later edits, moves, Forget, Undo, and capture Resume do not change that receipt. Only the live current body enters prompt/search retrieval; receipts and revision history remain outside retrieval.

The runtime reads approval receipts after persisting its change rows. This closes the ordering where approval occurs after its pending-proposal snapshot but before the response association exists. Store associates an approval with the proposal's original persisted response, including when a later turn observes the approval. Notifications use that exact response ID.

App approval returns a committed outcome separately from response-notice persistence. If SQLite cannot save the association after memory commits, clients refresh and show that memory was approved. Durable receipts and existing proposal rows support retry without approving again. Memory reads retry the association and can show the truthful approved card while writes remain unavailable. Each card keeps the original approved version for Undo. A later item edit makes that Undo stale. The item document and SQLite are separate stores; this protocol does not claim a cross-store transaction.

## Agent mentions

In Conversation mode, a new user message can select one configured agent with `@Jordan` or `@"Jordan Lee"`. The composer filters active agents with configured models and lists project members first. Enter or Tab inserts the selected name; arrow keys select another result, and Escape closes the list. The mention button opens the same list.

An explicit mention overrides the project default for that response. Discard adds the agent to the project and channel membership sets. It uses that agent's provider, model, personality, and private memory, with the same project and conversation context. The next unmentioned message uses the current project default. This also applies to channels created before the default changed. Direct chats remain bound to their own agent.

Unknown names, duplicate names, and more than one recipient produce an error before sending. Mentions in code, blockquotes, email addresses, topics, old messages, and model output do not route a response. Mentions do not create agents or start concurrent responses.

New replies store their author ID, name, provider, and avatar. The transcript and live response use this author, and model history includes it. Older replies without author metadata retain the channel identity as a display fallback.

## Scoped actions and response presentation

Conversation mode retains its empty provider working directory and disabled native shell tools. The scoped Discard MCP server can also expose worktree inspection and removal requests when the app starts it with the worktree service. Calls bind to the authenticated source conversation. The agent can request removal, but it cannot approve it. The app rechecks the worktree and executes removal only after a user approval. Summary tokens cannot use these tools. See [worktree tools](../worktrees.md#conversation-worktree-tools).

The response prompt leads with the current answer or next action, limits repeated snapshot facts and warnings, and distinguishes pending approval from confirmed completion. Agent personality and user instructions can still set the desired detail. Message prose and code blocks use the same base text size. Paragraph and code-block spacing separate explanations from commands.

## Workspace replay

Eligible project worktree responses use `SessionMode::Workspace`. They replay saved history with the same context budget and read-only summary sessions as Conversation mode. The `--builder` Coding path keeps native resume.

Replay keeps each tool name, error status, and target fields (`command`, `cmd`, `file_path`, `path`, `paths`, `notebook_path`, `pattern`). It limits tool output to the first 1024 bytes on a UTF-8 boundary and reports the omitted byte count. Changes cards replay their state, file paths and statuses, and commit IDs. Other blocks stay unchanged. Store retains all original tool inputs, outputs, and changes cards.

Compaction can reduce a message's fragment count. If the saved summary boundary no longer exists, replay starts after the last fragment of that same message. If the message is absent, replay retains the existing fallback and includes all available fragments.


## Approved write requests

`Command::Continue { agent, note }` starts a fresh response for the requesting
agent. It reads current configuration and workspace eligibility. It skips mention
resolution and stores no new user message. The current input explains the app
approval and tells the agent to continue the latest user request without repeating
completed work. The existing transcript framing stays in place.

Worktree creation approval binds an owned worktree before Continue. Main-checkout
approval uses an in-memory grant for one agent, one channel, and one response.
The engine consumes it when verifying that response's main-checkout binding.
The token carries the grant to `worktree_commit`. Provider protection checks still
apply. Claude enables main-checkout writes: `denyWrite` blocks shell writes to
`.git`, and file-tool edits under `.git` require a full-access card. Codex cannot
protect `.git` after approval, so it gets no main-checkout request tool or grant.
The engine uses the responding provider's capability when it prepares the MCP
identity and when it verifies a pending grant. Legacy memory preparation methods
default to no main-checkout requests.

Workspace approvals consult the current Always-allow rule table for each request.
Command rules match parsed word prefixes. Host rules match exact normalized hosts.
The audit uses `Policy("always_allow:<id>")`. Saving a rule sends a single Approve
to the provider. Coding keeps its provider-session Always allow behavior. Rules do
not approve sandbox escapes or app-owned write requests.

## Conversation moves

An approved move gives the target two replayed messages. The handoff is an agent message
authored by the project main agent. The user's latest message, with its attachments,
is a normal user message. Both are saved in the target and enter its replay. The
source transcript is not copied.

A new channel also gets the handoff as its topic. An existing target channel keeps
its topic.

The "Moved from" and "Moved to" link blocks are `System` messages. They never enter
replay. See [conversation moves](../worktrees.md#conversation-moves-slice-c).
