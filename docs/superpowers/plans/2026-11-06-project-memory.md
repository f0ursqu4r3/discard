# Project Memory Implementation Plan

**Goal:** Show and manage memory for the selected project without requiring an active conversation.

**Design:** The user approved the proposed project memory view in chat. Keep Markdown items and SQLite FTS5. Show shared project memory, separate agent-project groups, inherited global memory, and pending proposals from project conversations. Reuse versioned item editing, moves, Forget, and source navigation. Add search, kind/scope/agent filters, and explicit shared project creation. Move maintenance controls into a secondary menu. Conversation capture controls stay in conversation views.

**Architecture:** Add project-scoped backend helpers and optional project context to desktop memory commands. Extend the shared memory browser with an explicit project mode. Provider scope and retrieval stay unchanged.

**Tech stack:** Rust, Tauri, Vue, existing UI components, Vitest.

## Constraints and review focus

- Preserve unrelated working-tree changes. Do not commit them.
- Project views must exclude other projects and agent-wide private memory.
- Retained agent-project items remain inspectable when an agent is unavailable.
- Mutations and proposal approvals must validate project context on the backend.
- Project switches must clear drafts and reject stale asynchronous results.
- Global edits must clearly state that they affect all projects.
- Empty, filtered-empty, loading, and error states must be distinct.
- All visible new controls must work. Any planned action must be disabled and labeled Soon.

## Task 1: Project memory backend

- [x] Add tests for project-wide views, isolation, creation, version checks, and proposals.
- [x] Add project view and mutation helpers; connect desktop commands.
- [x] Run `cargo test -p discard-app --test project_memory --test memory_controls`.

## Task 2: Project memory UI

- [x] Add tests for project navigation without a conversation, groups, filters, creation, and switching.
- [x] Extend the browser and load it with the selected project ID.
- [x] Run the memory, workspace, and conversation UI tests and TypeScript checks.

## Task 3: Review and documentation

- [x] Update docs/memory.md with project tab behavior.
- [x] Review the task diff and run focused formatting/lint checks.
- [x] Obtain a fresh code review and fix material findings.

## Execution record

- Ruling: The existing conversation supplies the approved design and implementation authorization. No further design approval is needed.
- Ruling: Use the current checkout on a feature branch to preserve the user's active changes. Leave all changes uncommitted.
- Ruling: Semantic retrieval is deferred as recommended; this task does not add an unmeasured storage dependency.

- Completed: 69 focused UI tests and 32 Rust memory tests passed. TypeScript, the production web build, Vue lint, and whitespace checks passed.
- Review: Fixed proposal target-scope checks and idempotent creation after a partial index failure. Fresh follow-up review found no blocking issues.
- The desktop app was not manually exercised. Changes remain uncommitted.
