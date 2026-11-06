# Conversation runtime

Regular agent chats use `SessionMode::Conversation`. The deliberate `--builder` launcher uses Coding mode. Coding keeps provider-native resume and workspace behavior.

Before each conversation response, Engine loads the saved agent and conversation. An archived or deleted agent cannot start a response. The configured provider must be available and the model must be explicit. Engine holds that snapshot until the response ends. Provider/model changes keep the transcript and add a saved system message.

Conversation mode builds fresh provider input from this conversation's messages, the saved personality, and current applicable memory. It never supplies a native resume handle. Memory receives the actual optional project and source conversation ID. Conversation mode disables plugin-authoring tools and skips git worktree preparation. Provider adapters create their own empty working directory.

Memory instructions and preferences always enter the prompt in domain precedence order. Other live items use word overlap with the current request and the two latest messages in this conversation. Matching uses Unicode alphanumeric words of at least three bytes, normalized to lowercase. This is a simple lexical retrieval policy. The scoped memory search/read tools remain available for further retrieval. Current user instructions take precedence over memory. Capture guidance and the current capture-pause state enter each response prompt.

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

Normal startup creates no project, agent, conversation, or bootstrap workspace. It restores the independently saved conversation when that conversation still exists. With no saved conversation, the app opens agent navigation. Creating an agent creates its default direct conversation. Additional direct and project conversations use an explicit agent and a fixed optional project. Selecting a sidebar project does not change the active conversation. The deliberate `--builder` path retains Coding bootstrap. Agent forms and memory controls apply edits without restarting; active responses finish with their original configuration.


Changing a project folder restarts only cached transports for conversations in that project. Each conversation keeps its transcript, retained unsaved reply, notice, and selection state. Direct conversations and other projects keep their transports. Each transport has a channel-specific generation token; updates from a replaced transport cannot overwrite retained state. The project-change guard blocks sends during the change. A failed launch leaves the affected transport closed. Selecting that conversation still opens its retained history and retries the launch. If the retry fails, selection succeeds and the shared conversation-opened notification opens the transcript. The transcript shows an actionable restart notice beside the existing unsaved-reply notice. Repeated identical failures do not duplicate the notice. A failed history selection remains an error and does not emit an open notification.

## Approval receipts and response notices

An approval commits an immutable `ApprovedSnapshot` in the authoritative item document. This flat metadata record contains the approved content, scope, origin, version, source, identifiers, timestamps, and kind. It contains no nested MemoryItem, approval snapshots, or copied history. The item's ordinary history retains its revisions. Later edits, moves, Forget, Undo, and capture Resume do not change that receipt. Only the live current body enters prompt/search retrieval; receipts and revision history remain outside retrieval.

The runtime reads approval receipts after persisting its change rows. This closes the ordering where approval occurs after its pending-proposal snapshot but before the response association exists. Store associates an approval with the proposal's original persisted response, including when a later turn observes the approval. Notifications use that exact response ID.

App approval returns a committed outcome separately from response-notice persistence. If SQLite cannot save the association after memory commits, clients refresh and show that memory was approved. Durable receipts and existing proposal rows support retry without approving again. Memory reads retry the association and can show the truthful approved card while writes remain unavailable. Each card keeps the original approved version for Undo. A later item edit makes that Undo stale. The item document and SQLite are separate stores; this protocol does not claim a cross-store transaction.
