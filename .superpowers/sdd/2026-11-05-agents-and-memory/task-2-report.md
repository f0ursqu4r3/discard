# Task 2 report

Implemented the four memory domains and versioned memory backend. Runtime context and UI work remain in Tasks 3–5.

## Storage and retrieval

- `Scope::AgentProject(AgentId, ProjectId)` uses `agent-projects/<agent>/<project>` for legacy raw files. Both IDs and all existing path components are validated. Scope discovery includes orphan domain folders and item domains.
- `Scope::applicable(agent, Option<project>)` returns agent-project, project, agent, global in project chats; agent, global in direct chats.
- Each item has one authoritative `.items/<UUID>.md` document. A JSON metadata comment contains stable ID, domain, kind, source conversation, timestamps, version, history, deletion state, and recapture suppression records. The readable current markdown body is authoritative. The metadata's duplicate `content` field is empty on disk.
- Current live bodies appear under virtual `items/<UUID>.md` paths in existing list/read/search APIs. Hidden history and proposals never enter FTS or prompt blocks. Preferences and instructions enter core blocks on every prepare, in domain precedence order.
- Writes use the existing `MemoryStore.write_lock` and atomic document replacement. Moves change one document, not two scope files. The index is rebuildable. Search validates virtual hits against current scope, live state, and body hash, including a crash between markdown rename and FTS update.
- Raw mutation APIs reject virtual item paths. Existing explicit plugin grants can still use legacy raw files; agent tools cannot use raw append/edit to bypass item rules.

## Public APIs

All item types live in `discard_core::memory_items`.

- `ItemKind::{Fact, Preference, Instruction, Decision, Commitment}`.
- `MemoryItem`: UUID string `id`, `scope`, `kind`, `content`, `source_conversation: Option<ChannelId>`, millisecond timestamps, `version: u64`, `deleted`, `origin`, `history: Vec<ItemRevision>`, `recapture_suppression`, `proposal_id`. `path()` returns the virtual path.
- `MutationOrigin::{Automatic, User, Approved}`. App-only direct APIs use User or Approved. Never map a model JSON field to this enum.
- `create_item(scope, kind, content, Option<source>, origin)`, `list_items(scopes)`, `read_item(id)`.
- `update_item(id, expected_version, content, origin)`, `forget_item(id, expected_version)`, `move_item(id, expected_version, target_scope)`, `undo_item(id, expected_version)`.
- Mutations retain previous snapshots and increment versions. Stale edits, moves, Forget, approvals, and Undo return `MemoryError::Conflict`. Automatic replacements return `ApprovalRequired`. Undo of creation forgets the item.
- `capture_item(agent, Option<project>, source_conversation, target_scope, kind, content) -> SaveOutcome::{Saved(MemoryItem), Proposed(MemoryProposal)}`. Pass identity arguments from runtime state. Private automatic capture uses Agent outside a project or the matching AgentProject inside a project. Shared domains create proposals.
- `propose_update(agent, Option<project>, source, item_id, expected_version, content)` and `propose_forget(agent, Option<project>, source, item_id, expected_version)` enforce applicable scope access and preserve pending user authorization.
- `MemoryProposal` retains proposer `agent`, `source_conversation`, creation time, action, and resolution state. `ProposalAction` supports Create (scope/kind/content), Update (item/version/content), and Forget (item/version).
- `pending_proposals()` returns persisted pending records. App-only `approve_proposal(id)` and `reject_proposal(id)` resolve them. Approval refuses stale target versions. There is no MCP approval tool. Before approval, app callers must also validate that the proposer still exists and is active, using the agent Store; MemoryStore has no agent registry.
- `memory_changes(source_conversation) -> Vec<MemoryChange>` returns pollable Saved/Proposed records. Saved records use stable `item_id:version` keys; proposals use stable proposal IDs. Consumers deduplicate those keys. The saved record is the current item snapshot, not an append-only event stream. Item source stays its original conversation; proposals retain the requesting conversation.
- `private_scopes(agent)` enumerates private folders and item domains independently of conversation records. `delete_private_memory(agent)` removes Agent/AgentProject memory and that agent's proposals. It preserves Global/Project memory.
- New mutation errors include `ApprovalRequired`, `Suppressed`, `Forbidden`, and `InvalidDocument`.

## MCP and plugin interfaces

- `MemoryService::prepare(&AgentId, Option<&ProjectId>, &ChannelId source, Option<&str> base_prompt, bool plugin_write)`.
- `Identity.project` is optional. `source_conversation` is runtime-bound. `scopes` grants reads; `write_scopes` separately grants automatic private writes. `can_write` still controls the mutation tool surface.
- Tools: memory_list, memory_read, memory_search, memory_save, memory_edit, memory_forget. memory_save replaces arbitrary memory_append. memory_edit and memory_forget create proposals. Model fields such as approved, agent, project, source_conversation, and write_scopes cannot grant authorization.
- PluginMemory accepts optional projects, including an explicit project-less callback override. It supports agent-project names. The Lua manifest accepts agent-project grants.
- The current Engine compile caller still supplies its existing project as Some. Task 3 must change its runtime context model to pass the actual optional project. No fake project was added.

## Forget suppression and limits

Forget stores exact SHA-256-derived hashes of trimmed content with named conversation IDs in the item document. Direct app Forget suppresses its original source conversation. Approved in-chat Forget also suppresses the requesting conversation. Automatic capture of the same trimmed content from those conversations returns Suppressed across domains. This remains in effect while the authoritative suppression record exists. Explicit user/app saves bypass suppression. Undo of creation also suppresses its source. Undo does not erase suppression history.

This detects exact trimmed content only. It does not detect paraphrases, related facts, or recapture from a different conversation. It does not erase transcript messages. Capture guidance excludes temporary facts, guesses, passwords, and tokens; the backend does not claim semantic secret classification.

The store serializes mutations through one MemoryStore instance. It does not add a cross-process filesystem lock. Pollable change records replace live notifications for now; the runtime/UI can publish them without another memory subsystem.

## Verification

RED: initial memory integration tests failed to compile because Scope::AgentProject, memory_items, and mutation APIs did not exist. The later Forget proposal test failed because propose_forget did not exist. Both now pass.

GREEN:

- `cargo test -p discard-core -p discard-mcp -p discard-lua`: pass; 755 tests.
- `cargo test -p discard-app plugin_memory`: pass; 8 tests.
- `cargo check --workspace`: pass.
- `cargo test --workspace --no-run`: pass; all workspace test targets compile.
- `git diff --check`: pass.

Behavioral tests cover the complete two-agent/two-project matrix, direct-chat access, discovery, both ID/path checks, symlink rejection, stale edits/Undo/delete approvals, moves, history inspection, rejected proposals, shared self-approval attempts, separate read/write grants, bound source identity, original/requesting conversation recapture suppression, private cleanup, and reopen/reconcile/index rebuild/crash recovery.

Serena could not address the worktree. Shell editing used the parent-authorized worktree fallback. No merge or push performed.
