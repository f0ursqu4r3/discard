# Project channel foundation implementation

Spec: `docs/superpowers/specs/2026-11-05-project-channel-orchestration-design.md`

## Scope

This is the first implementation slice of the approved design. Deliver durable project/channel identity, last-channel navigation, versioned topics, project main-agent and membership records, and an honest UI. Later slices implement live multi-agent routing, orchestrator tools, task scheduling/recovery, integration, and provider-cache optimization. Do not claim those later capabilities work in this slice.

## Global Constraints

- Preserve all existing messages and agent direct conversations.
- Each project has one `general` channel and at most one active canonical channel for a branch name. Channel IDs survive branch renames through app operations; a retired binding must not capture a later branch with the same name.
- Project selection restores the last viewed active project channel, falling back to `general`. Never restore a channel from another project.
- Topics are versioned with an editor identity. Optimistic version checks prevent lost updates. Topic changes do not edit messages or task state.
- Main-agent selection and membership are persisted, validated against configured active agents, and scoped to the correct project/channel.
- Do not change repository files merely by selecting a project or asking a question.
- Planned controls follow AGENTS.md: disabled, `Soon`, adjacent TODO, shared rail model, no handler.
- All implementation runs in `/private/tmp/discard-project-channels`, branch `codex/project-channel-orchestration`. Never edit the original checkout. Implement with `gpt-6-luna`, high reasoning. No worker subagents.
- Each task runs covering tests, self-reviews, commits scoped changes, and writes a report. The orchestrator reviews spec compliance and code quality and requests fixes.

## Task 1: Durable project channel store

Ownership: `crates/discard-core/src/model.rs`, `crates/discard-core/src/store.rs` and focused new core store modules/tests if useful. Do not edit engine, desktop, or frontend files.

Add a backwards-compatible migration and Store APIs for:

1. Project channel metadata: kind (`general`, `branch`, archived legacy), canonical branch binding, archive linkage. Keep existing Channel/Session public APIs compatible where possible; use separate metadata records to avoid forcing unrelated test fixture changes.
2. One project general channel, resolved/created atomically using a valid configured agent. Existing direct channels stay unchanged. Preserve project conversation histories. For existing duplicate branch channels, select the most recently viewed if evidence exists, otherwise a deterministic newest-session/ID fallback; archive/link alternatives without changing messages. Reuse a suitable existing unbound `general` channel where possible. Provide an explicit API to change the canonical selection without deleting history.
3. Saved last viewed channel per project with validation and fallback access. Selecting a direct channel must not overwrite a project's saved channel.
4. Project main agent and persistent project/channel membership. Initial general creation uses caller's validated default agent. Inviting a configured active agent into a channel atomically ensures project and channel membership, is idempotent, and does not run a response. Direct chats remain single-agent. Reject archived/deleted/unknown agents and cross-project channel references.
5. Versioned plain-text channel topic and author history. Atomic compare-and-set saves, bounded reasonable input, prior revisions readable. Readers return empty topic/version zero before the first edit. Validate author identity when it is an agent; user edits use an explicit user author kind.
6. Explicit branch rename/retirement APIs preserve channel identity, make uniqueness constraints enforceable, and allow branch-name reuse after retirement. Do not infer deletion solely from a failed Git query.

Use transactions for migrations and multi-record creation. Existing bootstrap callers may still create legacy channels, so make canonical APIs coexist safely until Task 2 integrates the app. Do not silently change all existing project conversations to coding mode.

Meaningful tests: old schema fixture with duplicate branch channels preserves messages and deterministic canonical choice; reopening is idempotent; general uniqueness; project selection isolation; active-agent validation and idempotent invitations; topic stale-version rejection/history; branch rename and retirement/name reuse. Run `cargo test -p discard-core` using shared `CARGO_TARGET_DIR=/Users/la.kyle.dougan/git/personal/discard/target` if needed.

Report exported API names and semantics for downstream tasks, tests with output summary, migration choices, and commit IDs. Commit message: `feat: persist canonical project channels and topics`.

## Task 2: Desktop channel service and context integration

Ownership: desktop channel service/core/DTO/commands/window registration, app launcher integration if needed, and minimal core engine/topic context changes. Do not edit Vue files. Use Task 1's APIs; do not introduce a second metadata store.

Project selection should resolve a default active agent, ensure the project's general channel, and open its saved valid last channel. If no active configured agent exists, show an actionable error and do not invent one. Retain direct chats. Select/start paths persist last-channel only for the actual project of the selected channel. Opening a branch must resolve its canonical channel and ensure membership using the project main agent instead of inheriting an unrelated active direct-chat agent. Avoid switching Git branches or creating worktrees on mere selection.

Expose typed backend APIs/DTOs for project channel listing (general + canonical branch channels + linked archives), channel details/topic history/topic update, members, main-agent selection and canonical selection where supported. Validate archived channels and cross-project IDs. Topic mutation must use expected version and author; user UI cannot impersonate an agent.

Include current topic as labelled channel context in subsequent Conversation-mode prompt construction. Preserve explicit instruction precedence, original messages, bounded context/error behavior and summary correctness. Topic content must never trigger mentions/tools. Keep full agent routing and coding/task runtime changes outside this slice, clearly documented.

Add tests for first visit, A -> B -> A restore, direct-channel selection isolation, canonical branch reuse despite different active agent, no-active-agent failure, topic API stale edits, and prompt topic refresh without transcript deletion. Run focused app tests and core tests covering changed engine logic. Commit `feat: open persistent project channels and expose details`.

## Task 3: Project channel navigation and editable details

Ownership: `web/src` bridge types/API/mock fixtures, channel navigation/header/details UI, App wiring, and focused Vue tests. Do not edit Rust unless reporting a required contract fix to the orchestrator.

Use actual Task 2 APIs. Show `#general` persistently in the project channel rail; selecting it opens the project general channel. Branch rows resolve canonical channels through the existing branch action. Project selection opens the restored channel without a separate start-conversation flow. Direct chats remain functional.

Add a channel details surface with current topic, editable topic, save/cancel and stale-update feedback, topic history author/version, channel identity/branch association, main agent and participants. Wire supported main-agent selection and explicit canonical selection when exposed. Preserve archived histories and distinguish them from active channels. Keep topology and branch labels accurate; do not expose raw implementation/session details.

Do not add working mention autocomplete or agent orchestration controls without their dispatch implementation. If a related existing planned control is changed, keep it disabled with `Soon` and an adjacent TODO.

Tests should cover general navigation, topic save/version handling and persistence reload, project restore event, direct-chat regression, and planned-control honesty. Run frontend tests, typecheck and build. Commit `feat: add project channel navigation and topic details`.

## Task 4: Integration verification and documentation

Ownership: runtime documentation, focused integration tests/fixtures, and only review-requested integration fixes. Read prior task reports and ledger before changing anything.

Update `docs/runtime/conversation-context.md` and applicable project docs to describe implemented behavior accurately. Add a roadmap/status document distinguishing this slice from the remaining approved orchestration design. Preserve existing provider and direct-chat behavior. Add or adapt end-to-end service tests for migration -> project open -> branch reuse -> topic edit -> reload.

Run `cargo test --workspace`, frontend tests, typecheck, build, and `git diff --check`. Inspect any failures before changing code. Report test counts and limitations. Commit `docs: record project channel foundation and next steps` (use a fix commit separately if code changes are necessary).

## Next implementation slices

- Explicit mention syntax/composer tokens, recruitment, per-agent execution contexts and response attribution.
- Durable task graph, queued steering, bounded delegation, project concurrency and worktree mutation locks.
- Main-agent tool access for branch/channel creation, topic handoffs, progress links and integration.
- Stop descendants, restart reconciliation, paused recovery and branch lifecycle detection.
- Cache-aware provider input and context retrieval without dropping required evidence.
