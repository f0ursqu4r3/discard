# Task 2 report

Implemented the four memory domains and versioned memory backend. Provider controls, runtime context, and UI work remain in Tasks 3–6.

## Storage and retrieval

- `Scope::AgentProject(AgentId, ProjectId)` uses `agent-projects/<agent>/<project>` for legacy raw files. Both IDs and all existing path components are validated. Scope discovery includes orphan domain folders and item domains.
- `Scope::applicable(agent, Option<project>)` returns agent-project, project, agent, global in project chats; agent, global in direct chats.
- Each item has one authoritative `.items/<UUID>.md` document. A JSON metadata comment contains stable ID, domain, kind, source conversation, timestamps, version, history, deletion state, and recapture suppression records. The readable current markdown body is authoritative. Metadata retains `committed_body_hash` and a committed `content` snapshot for stale-write detection and import history. The snapshot does not override the readable body.
- Current live bodies appear under virtual `items/<UUID>.md` paths in existing list/read/search APIs. Hidden history and proposals never enter FTS or prompt blocks. Preferences and instructions enter core blocks on every prepare, in domain precedence order.
- Writes use the existing `MemoryStore.write_lock` and atomic document replacement. Moves change one document, not two scope files. The index is rebuildable. Search validates virtual hits against current scope, live state, and body hash, including a crash between markdown rename and FTS update.
- Raw mutation APIs reject virtual item paths. Existing explicit plugin grants can still use legacy raw files; agent tools cannot use raw append/edit to bypass item rules.

## Public APIs

All item types live in `discard_core::memory_items`.

- `ItemKind::{Fact, Preference, Instruction, Decision, Commitment}`.
- `MemoryItem`: UUID string `id`, `scope`, `kind`, `content`, `source_conversation: Option<ChannelId>`, millisecond timestamps, `version: u64`, `deleted`, `origin`, `history: Vec<ItemRevision>`, `recapture_suppression`, `paused_capture_conversations`, `change_conversation`, `proposal_id`. `path()` returns the virtual path.
- `MutationOrigin::{Automatic, User, Approved}`. App-only direct APIs use User or Approved. Never map a model JSON field to this enum.
- `create_item(scope, kind, content, Option<source>, origin)`, `list_items(scopes)`, `read_item(id)`.
- `update_item(id, expected_version, content, origin)`, `forget_item(id, expected_version)` / `forget_item_in(id, expected_version, Option<&ChannelId> requester)`, `move_item(id, expected_version, target_scope)`, `undo_item(id, expected_version)`.
- Mutations retain previous snapshots and increment versions. Stale edits, moves, Forget, approvals, and Undo return `MemoryError::Conflict`. Automatic replacements return `ApprovalRequired`. Undo of creation forgets the item.
- `capture_item(agent, Option<project>, source_conversation, target_scope, kind, content) -> SaveOutcome::{Saved(MemoryItem), Proposed(MemoryProposal)}`. Pass identity arguments from runtime state. Private automatic capture uses Agent outside a project or the matching AgentProject inside a project. Shared domains create proposals.
- `propose_update(agent, Option<project>, source, item_id, expected_version, content)` and `propose_forget(agent, Option<project>, source, item_id, expected_version)` enforce applicable scope access and preserve pending user authorization.
- `MemoryProposal` retains proposer `agent`, `source_conversation`, creation time, action, and resolution state. `ProposalAction` supports Create (scope/kind/content), Update (item/version/content), and Forget (item/version).
- `pending_proposals()` returns persisted pending records. App-only `approve_proposal(id)` and `reject_proposal(id)` resolve them. Approval refuses stale target versions. There is no MCP approval tool. Before approval, app callers must also validate that the proposer still exists and is active, using the agent Store; MemoryStore has no agent registry.
- `memory_changes(source_conversation) -> Vec<MemoryChange>` returns pollable Saved/Proposed records. Saved records use stable `item_id:version` keys; proposals use stable proposal IDs. Consumers deduplicate those keys. The saved record is the current item snapshot, not an append-only event stream. Item source stays its original conversation. `change_conversation` associates the current saved mutation with its requesting conversation; revision snapshots retain that association too. Approved corrections and Forget use their proposal source. Saved records appear once under the requesting conversation with the stable item/version key.
- `private_scopes(agent)` enumerates private folders and item domains independently of conversation records. `delete_private_memory(agent)` removes Agent/AgentProject memory and that agent's proposals. It preserves Global/Project memory.
- New mutation errors include `ApprovalRequired`, `Suppressed`, `Forbidden`, and `InvalidDocument`.

## MCP and plugin interfaces

- `MemoryService::prepare(&AgentId, Option<&ProjectId>, &ChannelId source, Option<&str> base_prompt, bool plugin_write)`.
- `Identity.project` is optional. `source_conversation` is runtime-bound. `scopes` grants reads; `write_scopes` separately grants automatic private writes. `can_write` still controls the mutation tool surface.
- Tools: memory_list, memory_read, memory_search, memory_save, memory_edit, memory_forget. memory_save replaces arbitrary memory_append. memory_edit and memory_forget create proposals. Model fields such as approved, agent, project, source_conversation, and write_scopes cannot grant authorization.
- PluginMemory accepts optional projects, including an explicit project-less callback override. It supports agent-project names. The Lua manifest accepts agent-project grants.
- The current Engine compile caller still supplies its existing project as Some. Task 3 must change its runtime context model to pass the actual optional project. No fake project was added.

## Forget suppression and limits

Forget pauses all automatic capture for the item's original source conversation. Direct app `forget_item_in` and approved in-chat Forget also pause the requesting conversation. Undo of creation pauses its source conversation. The pause blocks paraphrases, old corrected values, unrelated new facts, and automatic shared-save proposals without trying to recognize semantic equivalence. A fresh conversation has a new ID and no inherited pause. Direct user/approved saves bypass the pause without lifting it. Undo does not lift it.

`capture_paused(&ChannelId) -> Result<bool>` exposes controller status. App-only `resume_capture(&ChannelId) -> Result<()>` clears that conversation's pause flags under the store lock. No MCP tool or JSON field can resume capture. `MemoryError::CapturePaused` gives the UI a distinct state. Pause flags live in authoritative item documents and survive reopen; no separate metadata shadow exists. Resume changes controller metadata, not item content/version, so the UI must refresh pause status independently of saved item/version keys. If a multi-document resume fails, remaining flags conservatively keep capture paused.

Forget also retains exact SHA-256-derived hashes of trimmed current and all prior item bodies for each affected conversation. These hashes continue to reject exact recapture with `Suppressed` after an explicit resume. Explicit user/approved saves bypass those hashes. Hashes do not detect paraphrases; the conversation pause is what blocks every automatic capture until resume. Neither mechanism erases transcript messages. Capture guidance excludes temporary facts, guesses, passwords, and tokens; the backend does not claim semantic secret classification.

The store serializes mutations through one MemoryStore instance. It does not add a cross-process filesystem lock. Pollable change records replace live notifications for now; the runtime/UI can publish them without another memory subsystem.

## Verification

RED: initial memory integration tests failed to compile because Scope::AgentProject, memory_items, and mutation APIs did not exist. The later Forget proposal test failed because propose_forget did not exist. Both now pass.

GREEN:

- `cargo test -p discard-core -p discard-mcp -p discard-lua`: pass; 759 tests.
- `cargo test -p discard-app plugin_memory`: pass; 8 tests.
- `cargo check --workspace`: pass.
- `cargo test --workspace --no-run`: pass; all workspace test targets compile.
- `git diff --check`: pass.

Behavioral tests cover the complete two-agent/two-project matrix, direct-chat access, discovery, both ID/path checks, symlink rejection, stale edits/Undo/delete approvals, moves, history inspection, rejected proposals, shared self-approval attempts, separate read/write grants, bound source identity, original/requesting conversation recapture suppression, private cleanup, and reopen/reconcile/index rebuild/crash recovery.

Serena could not address the worktree. Shell editing used the parent-authorized worktree fallback. No merge or push performed.

## Review fix round 1

Implemented the controller ruling for strict no-immediate-recapture. Conversation-level pauses now block all automatic capture, including paraphrases and old corrected values, until explicit app/user resume or a fresh conversation. Forget records all prior body hashes as well as the current body. Approved in-chat Forget covers original source and requesting conversation. Explicit saves remain possible without silently resuming automation.

Approved Update/Forget now set typed `change_conversation` metadata to the requesting conversation. `memory_changes(requester)` returns the completed Saved mutation there; it does not also emit that current key under the original source. Revision snapshots retain the previous requesting conversation. Reopen tests verify both pause state and completed mutation association.

RED: `cargo test -p discard-core --test memory_items forget_pauses` failed to compile because `capture_paused`, `resume_capture`, and `CapturePaused` did not exist. GREEN: the full memory integration target passes all 11 tests. New tests cover paraphrases, unrelated facts, old corrected bodies, all-version exact suppression after app resume, pause persistence and resume persistence, fresh conversations, explicit User/Approved saves, Undo of creation, requesting-source completion records, and MCP attempts to resume via JSON or an invented tool.

Final checks after this fix: core/MCP/Lua suite passes (757 tests), workspace check passes, and all workspace test targets compile. No merge or push performed.

## Review fix round 2

Added committed-body protection for external markdown edits. The item document now persists `committed_body_hash` plus its committed body snapshot in metadata `content`. The readable body remains authoritative. This changes the current format; no migration is provided or required for this new application.

All versioned mutations compare the readable body with its committed hash before they write. A mismatch returns `MemoryError::Conflict` and instructs the caller to reconcile and reload. Update, Move, Forget, Undo, inferred proposal creation, and proposal approval use that check. App Resume also refuses to silently commit an external body while changing pause metadata.

`reconcile()` and `rebuild_index()` explicitly import an external readable-body edit as a new User revision. Import advances the numeric version, preserves the previous committed body in history, and updates the hash/index. A second reconcile is idempotent. A caller holding v1 still conflicts after import advances the item to v2. Reload the item after reconcile before retrying a user edit. The store does not lock out an external filesystem writer; its existing in-process lock serializes app mutations.

Added `forget_item_in(id, expected_version, Option<&ChannelId> requester)` for direct app Forget. It pauses both original source and requesting conversation and associates the completed change with the requester. It creates no model proposal. `forget_item` remains the convenience wrapper with no requester. Memory UI code should pass its current conversation to `forget_item_in`.

RED: the external-body regression test reproduced silent loss: A at v1, external B, stale update(v1,C) returned success and overwrote B. GREEN: all 13 memory integration tests pass. The mutation matrix now checks stale Update, Move, Forget, direct app Forget, Undo, Update approval, and Forget approval before and after reconcile; it also checks prior-body history, reload/reopen, and idempotent import. A direct app Forget test verifies both conversation pauses and requesting-source completed records without a fabricated proposal.

Final checks: core/MCP/Lua suite passes (759 tests); workspace check and all workspace test target compilation pass. No merge or push performed.
