# Create a branch here Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent in a detached worktree channel can ask to create a branch at HEAD. On approval Discard creates it and rebinds the same channel to it. The menu **Create Branch…** rebinds detached channels the same way.

**Architecture:** A new `WriteRequestKind::BranchCreate` in the slice B write-request store, a new MCP tool `worktree_request_branch`, an approval arm in `decide_write_request`, and one shared rebind that converts detached channels of a worktree path into branch channels.

**Tech Stack:** Rust (`discard-core`, `discard-mcp`, `discard-app`, `discard-providers`), Vue 3 + TS (`web/`).

**Spec:** `docs/superpowers/specs/2026-11-24-detached-branch-request-design.md`

## Global Constraints

- Tool name, exact: `worktree_request_branch`, one string argument `name`.
- Request kind string in JSON/web, exact: `branch_create`. Denial text, exact: "Branch creation was denied."
- Retarget refusal text, exact: "The request target changed. Ask again."
- Git runs with argument lists and hooks off (existing helpers). The agent never creates a branch without approval.
- Only the user approves. Expiry 10 minutes, in memory, one pending request per channel and kind (existing rules).
- Web tests live in `__tests__` beside the code. No semicolons in web code. Web prose: short, direct, plain text.
- Run Rust tests with `CLAUDE*`/`ANTHROPIC*` env vars stripped. Web: `pnpm --dir web test`.

## Review Focus

- HEAD moved or a commit landed between request and approval: refuse, create nothing.
- The branch name was taken between request and approval: Git error shown, nothing rebound.
- Two worktrees at one commit: only the requesting channel's worktree path is touched.
- A second detached channel (archived or active) on the same path: no active channel keeps a detached binding afterwards.
- Menu **Create Branch…** on a detached worktree whose channel exists: channel can write afterwards.

---

### Task 1: Request kind, gate, and rebind (core)

**Files:**
- Modify: `crates/discard-core/src/write_requests.rs`, `crates/discard-core/src/workspace.rs` (`write_request_target`), `crates/discard-core/src/store.rs` (rebind), `crates/discard-core/src/workspace_session.rs` (only if the rebind needs it)
- Test: same files' test modules

**Interfaces:**
- Produces: `WriteRequestKind::BranchCreate`; `WriteRequest` gains the new branch name (reuse the `branch` field) and the HEAD commit (new optional field, camelCase `commit`); `GitWorkspace::write_request_target(.., WriteRequestKind::BranchCreate, name)` returning the worktree path, the HEAD commit, and the validated name, or a plain error; `Store::rebind_detached_to_branch(project: &ProjectId, repository: &str, worktree_path: &str, branch: &str) -> Result<Vec<ChannelId>>` which converts every active Legacy channel with a worktree record at that path to a branch channel of `branch` (use `bind_repository_branch` and set `worktrees.branch`) and returns the channels moved.

- [ ] **Step 1: Write failing tests.** Gate: accepts a detached linked-worktree channel; refuses a branch channel, an existing branch name, an invalid name, a non-opted-in agent, a main checkout. Request: one pending per channel and kind; denial text "Branch creation was denied."; TTL expiry. Rebind: channel ID and history unchanged after rebind; `eligibility`/`verify_binding` then grants writes with `branch = Some(name)`; archived channels untouched; a second active detached channel at the path also moves.
- [ ] **Step 2: Run** `cargo test -p discard-core branch_create` and the rebind test names — Expected: FAIL.
- [ ] **Step 3: Implement** the kind, the gate, and `rebind_detached_to_branch`. The gate reads HEAD with the existing Git helpers; detached means `rev-parse --abbrev-ref HEAD` gives `HEAD`.
- [ ] **Step 4: Run** the new tests and `cargo test -p discard-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: request kind and rebind for creating a branch in a detached worktree`.

### Task 2: Approval and menu rebind (app)

**Files:**
- Modify: `crates/discard-app/src/desktop/channels.rs` (`decide_write_request` arm; `create_branch` rebind)
- Test: `channels.rs` test module (copy `write_approval_queues_one_scoped_continue_and_no_user_message` and the move-approval tests for the pattern; reuse the `decide()` helper)

**Interfaces:**
- Consumes: Task 1 kind, gate, `Store::rebind_detached_to_branch`.
- Produces: approval of `BranchCreate` creates the branch at the request's commit, rebinds, refreshes branches, finishes the request, and queues the same scoped continue turn as WorktreeCreate.

- [ ] **Step 1: Write failing tests:** `branch_create_approval_creates_the_branch_and_rebinds_the_channel` (same channel ID, branch channel of the new name, worktree row branch set, continue queued once, no user message); `branch_create_denial_changes_nothing`; `branch_create_refuses_when_head_moved` ("The request target changed. Ask again."); `branch_create_git_failure_rebinds_nothing` (name taken after the request); `menu_create_branch_rebinds_a_detached_channel` (after **Create Branch…** on a detached worktree the channel is a branch channel and `verify_binding` passes).
- [ ] **Step 2: Run** `cargo test -p discard-app branch_create` and `menu_create_branch` — Expected: FAIL.
- [ ] **Step 3: Implement** the approval arm (hold the channel, re-run the gate, compare name, path, commit, create, rebind, `after_git`) and the `create_branch` rebind for trees that were detached. A rebind failure after the branch exists reports it and keeps the branch.
- [ ] **Step 4: Run** the new tests and the full `cargo test -p discard-app` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: approve a branch request and rebind detached channels`.

### Task 3: MCP tool and provider allowlists

**Files:**
- Modify: `crates/discard-mcp/src/tools.rs` (definition, `tool_list`, gate at ~250, dispatch at ~260, `run_write_request`), `crates/discard-mcp/src/service.rs` (one prompt line for detached channels), `crates/discard-providers/src/conversation.rs` (`MEMORY_TOOLS`), `crates/discard-providers/src/claude/provider.rs` (allowlist string), `crates/discard-providers/tests/codex_process.rs` (hard-coded list)
- Test: `tools.rs` test module (copy `creation_rig` and `creation_tool_is_listed_only_for_unbound_branch_channels_with_opted_in_agents`)

**Interfaces:**
- Consumes: Task 1 gate and kind.
- Produces: tool `worktree_request_branch(name: string)` returning `{approvalRequired: true, request, message}`; message exact: "Branch creation is pending in the app. No branch exists yet. You cannot approve this request."

- [ ] **Step 1: Write failing tests:** `branch_tool_is_listed_only_for_detached_worktree_channels_with_opted_in_agents`; `branch_tool_call_returns_approval_required_and_creates_nothing` (no branch exists after the call); `branch_tool_refuses_an_existing_branch_name`; update the `codex_process.rs` assertion and add the tool to the lists.
- [ ] **Step 2: Run** `cargo test -p discard-mcp branch_tool` — Expected: FAIL.
- [ ] **Step 3: Implement** the definition, listing, gate, dispatch, allowlists, and the prompt line.
- [ ] **Step 4: Run** `cargo test -p discard-mcp -p discard-providers` — Expected: PASS (resumable_parts goldens change only if the prompt line is in them; update with the repo's golden flow if so).
- [ ] **Step 5: Commit** `feat: worktree_request_branch tool`.

### Task 4: Approval card (web)

**Files:**
- Modify: `web/src/components/workspace/WriteAccessApprovals.vue`, `web/src/bridge/api.ts` (`WriteRequest` kind union and `commit`)
- Test: `web/src/components/workspace/__tests__/WriteAccessApprovals.test.ts`

- [ ] **Step 1: Write failing tests:** title "Create branch `name` here?", shows worktree path and short commit, approve and deny each call the command once.
- [ ] **Step 2: Run** `pnpm --dir web test WriteAccessApprovals` — Expected: FAIL.
- [ ] **Step 3: Implement** `title()`, `approveLabel()` ("Create branch") for `branch_create`, and the type changes. Plain text only.
- [ ] **Step 4: Run** `pnpm --dir web test` and `pnpm --dir web check` — Expected: PASS (`MoveRequestDetails.vue` format failure exists on main; ignore it).
- [ ] **Step 5: Commit** `feat: card for a branch request`.

### Task 5: Docs

**Files:**
- Modify: `docs/worktrees.md` ("Not built" list and a short section), `docs/project-channel-status.md` (planned item), `docs/superpowers/plans/2026-11-17-agent-workspace-writes-slice-c.md` (line 312 wording only if it says planned)

- [ ] **Step 1:** Replace "Not built" for this item with the behavior: the tool, the card, rebind, and that **Create Branch…** now rebinds detached channels. Keep "A move to an existing branch that has no channel" as not built.
- [ ] **Step 2: Commit** `docs: create a branch here`.
