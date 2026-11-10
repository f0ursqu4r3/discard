# Conversation runtime

Regular agent chats use `SessionMode::Conversation`. The deliberate `--builder` launcher uses Coding mode. Coding keeps provider-native resume and workspace behavior.

Before each conversation response, Engine loads the saved agent and conversation. An archived or deleted agent cannot start a response. The configured provider must be available and the model must be explicit. Engine holds that snapshot until the response ends. Provider/model changes keep the transcript and add a saved system message.

Conversation mode builds fresh provider input from this conversation's messages, the saved personality, and current applicable memory. It never supplies a native resume handle. Memory receives the actual optional project and source conversation ID. Conversation mode disables plugin-authoring tools and skips git worktree preparation. Provider adapters create their own empty working directory.

Memory instructions and preferences always enter the prompt in domain precedence order. Other live items use word overlap with the current request and the two latest messages in this conversation. Matching uses Unicode alphanumeric words of at least three bytes, normalized to lowercase. This is a simple lexical retrieval policy. The scoped memory search/read tools remain available for further retrieval. Current user instructions take precedence over memory. Capture guidance and the current capture-pause state enter each response prompt.

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

The scoped MCP server provides `project_list_files`, `project_read_file`, and `project_search` for follow-up
questions. These tools validate the token's project and conversation on each call. General channels can use
them without a worktree binding. Worktree-only tools are not listed for general channels. Summary sessions
and plugin tokens cannot use project tools. File content is reference data, not agent instructions.

File tools accept relative paths. They reject parent traversal, symlinks, special files, hidden paths except
`.github`, common build directories, and key files. Git projects expose tracked and nonignored files.
Other folders use a bounded directory walk. File reads accept UTF-8 text up to 256 KiB and return up to
200 lines or 8 KiB per call. Listing supports offsets. Search uses literal, case-sensitive text and scans
up to 100 files with up to 40 matches or 8 KiB of excerpts. Results report continuation, skipped files,
and partial search results. Large directory listings ask for a narrower directory.

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
