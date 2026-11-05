# Agents and Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create persistent agents with independent model selection, project-less direct chats, and four domains of inspectable memory.

**Architecture:** Keep agent identity in the core store and make conversation project context optional. Resolve agent configuration and memory at each turn, and build provider context from the stored transcript rather than relying on a provider's stale native session. Extend the existing memory store with individual versioned items and enforce memory access through session-bound identities.

**Tech Stack:** Rust, SQLite, local markdown memory, MCP, Tauri, Vue 3, TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-05-agents-and-memory-design.md`

## Global Constraints

- No data migration or backward-compatibility layer is required.
- Preserve stable agent IDs across name, personality, provider, and model changes.
- General conversations have no project. Never represent them using a fake project ID.
- Memory precedence: agent-project, project, agent, global; current user instructions take priority.
- Automatic writes are private. Shared writes require a user instruction or approval enforced by the application.
- No silent model fallback. Apply changed configuration after an active response finishes.
- Planned controls are disabled, show `Soon`, have a nearby `TODO`, and have no executable handler.
- Use short, direct user-facing text.

## Review Focus

- A direct chat must never inherit the selected project's memory or working directory.
- A model edit during a running response must not change that response or drop conversation history.
- Memory edits, deletion, and Undo must reject stale versions and must remove old values from retrieval.
- An agent must not grant itself shared-memory write access through tool arguments.
- Deletion and archive must preserve shared memory and avoid orphaned visible conversations.

### Task 1: Agent lifecycle and conversation identity

**Files:** `crates/discard-core/src/agent.rs`, `agent_edit.rs`, `model.rs`, `store.rs`, `bootstrap.rs`; affected constructors and tests across the workspace.

**Interfaces:** `Channel.project_id: Option<ProjectId>`; agent lifecycle operations in `Store`; `AgentDraft` includes editable name, avatar, provider, model, and personality alongside existing policy fields. Use `system_prompt` as the stored personality field. Add archive state to stored agent records, not to their IDs.

- [ ] Add behavior tests for generated editable names, two agents sharing a model, archive/restore, and permanent deletion choices that preserve shared data.
- [ ] Add store tests for one default direct conversation per agent, multiple additional conversations, nullable project context, and context fixed at creation.
- [ ] Implement `Store::create_agent(&AgentConfig) -> Result<AgentId>`, `Store::set_agent_archived(&AgentId, bool) -> Result<()>`, and `Store::agent_archived(&AgentId) -> Result<bool>`.
- [ ] Implement `Store::create_conversation(&AgentId, Option<&ProjectId>, &str) -> Result<Channel>` and `Store::conversations_for_agent(&AgentId) -> Result<Vec<Channel>>`; creation also creates the initial session.
- [ ] Add a generated-name helper and validate nonempty names and provider/model choices. Keep separate IDs even when names match.
- [ ] Change the fresh database schema and adapt callers to optional projects. Do not add compatibility migrations for this feature.
- [ ] Run `cargo test -p discard-core` and compile the workspace to locate all interface callers.

### Task 2: Four-domain memory items

**Files:** `crates/discard-core/src/memory.rs`, new `memory_items.rs`, `memory_page.rs`, `lib.rs`; `crates/discard-mcp/src/identity.rs`, `tools.rs`, `service.rs`; plugin memory scope adapters.

**Interfaces:** Add `Scope::AgentProject(AgentId, ProjectId)` and `ScopeKind::AgentProject`. Add `Scope::applicable(&AgentId, Option<&ProjectId>) -> Vec<Scope>` in precedence order. MCP identity binds the agent, optional project, source conversation, and separate read/write grants. Item mutations use expected versions.

- [ ] Test the complete domain matrix with two agents and two projects, including project-less access, scope discovery, search, and path validation for both IDs.
- [ ] Add item storage containing content, kind (fact/preference/instruction/decision/commitment), source conversation, timestamps, and version history. Keep markdown authoritative and the index rebuildable.
- [ ] Implement list/read/create/update/delete/move/undo operations with optimistic concurrency. Forgotten items and superseded versions must not enter retrieval. Keep a suppression record to prevent immediate automatic recapture from the same conversation.
- [ ] Separate read grants from write grants. Automatic tool writes target only the matching private domain. Shared writes produce a proposal for user approval; callers cannot self-authorize by setting a JSON flag.
- [ ] Add memory delete and correction behavior. Inferred replacements require clarification; explicit user edits and approved proposals may replace an item.
- [ ] Add tests for stale edit, stale Undo, deleted search hits, move between domains, history inspection, and failed shared-write escalation.
- [ ] Run `cargo test -p discard-core -p discard-mcp -p discard-lua`.

### Task 3: Per-turn agent configuration and context

**Files:** `crates/discard-core/src/engine.rs`, `provider.rs`, new `conversation_context.rs`, `observer.rs`, `ui_state.rs`; `crates/discard-app/src/main.rs`; provider adapters and runtime tests.

**Interfaces:** Resolve a `Provider` from the current `AgentConfig.provider` at each turn. `MemoryService::prepare` takes optional project context and source conversation. Context contains the current transcript, personality, and current applicable memory; it never imports another conversation's transcript.

- [ ] Add fake-provider tests for live model/personality edits, edits during a response, no fallback on provider failure, and conversation continuity across provider changes.
- [ ] Refresh configuration before each queued turn. Record a transcript marker when the provider/model differs from the previous turn. Archived/deleted agents cannot start a new turn.
- [ ] Rebuild provider input from the app's transcript and current memory. Do not resume a provider-native session that retains stale prompts or a previous model's context.
- [ ] Implement context limits and summaries of older messages, preserving the complete transcript. Include relevant memory and all applicable instructions/preferences; report when a summary is used.
- [ ] Make memory capture part of the turn workflow. Prompt/tool guidance must exclude guesses and secrets, obey private defaults, and surface saved-item events and shared-write proposals to the UI.
- [ ] Route approval decisions back to the bound proposal. Ensure a rejected proposal cannot mutate shared memory.
- [ ] In conversation-only mode, expose memory tools without external write actions. Project-less chats use an app-owned empty working directory and cannot inherit project plugins or filesystem access.
- [ ] Run focused engine/context/provider tests, then `cargo test --workspace`.

### Task 4: Desktop agent and conversation controls

**Files:** `crates/discard-app/src/desktop/settings/agents.rs`, `settings/mod.rs`, `channels.rs`, `commands.rs`, `core.rs`, `dto.rs`, `mod.rs`; `web/src/bridge/api.ts`, `types.ts`, `store.ts`; agent settings and workspace components.

**Interfaces:** Tauri commands create/edit/archive/restore/delete agents and create/select direct or project conversations. DTOs expose agent IDs, optional project context, archive state, and provider/model labels. Keep bridge types and command registration synchronized.

- [ ] Add create/edit forms with a generated editable name, provider/model selection, optional avatar and personality, and clear validation errors.
- [ ] Add agent navigation and a default direct chat. Support additional conversations and project conversations with explicit agent selection.
- [ ] Add archive/restore and permanent-delete choices. Prevent running-response deletion; preserve shared memory. Kept history remains readable even if its agent is deleted.
- [ ] Show model-change markers, context-summary notices, and model errors with retry/model-selection controls.
- [ ] Add Vue and Rust tests for command payloads, direct-chat selection without a project, multiple conversations, pending edits, and deletion choices.
- [ ] Run `pnpm --dir web test`, `pnpm --dir web typecheck`, and focused desktop tests.

### Task 5: Memory controls and final integration

**Files:** `crates/discard-app/src/desktop/settings/memory.rs`, command registration and DTOs; `web/src/settings/pages/Memory.vue`, conversation memory components and tests; `docs/memory.md`, `docs/settings.md`, provider docs as affected.

**Interfaces:** User memory commands expose items, versions, domain choices, source links, history, and mutation results. Conversation events expose saved items and pending shared-memory proposals. User edits use the same versioned storage as agent tools.

- [ ] Implement all-domain Settings browsing and conversation-scoped memory browsing with inspect/edit/delete/move controls.
- [ ] Show automatic saves beside responses with Undo. Show shared-write proposals with approve/reject. Reject stale approvals and stale Undo operations with a clear error.
- [ ] Add a Forget action with an explanation that transcript deletion is separate and a working fresh-conversation action.
- [ ] Test memory freshness on the next response, source links, scope restrictions, global/project approval, contradiction handling, and forgotten-item suppression across a fresh conversation.
- [ ] Update documentation to describe actual new behavior and remove obsolete restart/session-start restrictions.
- [ ] Run workspace tests, web tests, type checks, lint, and formatting checks. Inspect the working UI where the available runtime permits it.
- [ ] Review the complete change against every requirement in the spec. Resolve important findings before reporting completion.
