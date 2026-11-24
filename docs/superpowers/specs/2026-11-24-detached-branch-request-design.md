# Create a branch here: design

Date: 2026-11-24. Size: project. Status: approved (the user said to build it; the calls below are mine).

## Intent

An agent in a detached worktree can edit files but cannot commit. Slice A spec decision 17 promised "create a branch here": the agent asks, the user approves, Discard creates the branch at HEAD and the channel continues on that branch. The existing **Create Branch…** menu action has a related defect: after it runs, the detached channel keeps a detached-style binding, so `verify_binding` refuses its writes and the conversation is stranded.

## Goals

- A write-capable agent in a detached worktree channel can call `worktree_request_branch` with a branch name.
- The user approves or denies on a protected card in the channel. Approval creates the branch at the current HEAD of the worktree and rebinds the same channel to it. The channel keeps its identity and history.
- After approval the agent continues with a new response and can commit (`worktree_commit` works on a branch channel).
- **Create Branch…** from the menu rebinds the detached channels of that worktree the same way. No channel is stranded.

## Non-goals

- A move to an existing branch, or a move to another channel (slice C already has `conversation_request_move`).
- Creating a worktree. The worktree already exists.
- Main-checkout detached HEAD. The tool needs a linked worktree with a saved worktree record, as detached channels have today.
- Codex-specific changes beyond the tool allowlists that every request tool needs.

## Anti-goals

- An agent that creates a branch or rebinds a channel without the user's approval.
- Approval that acts on a different commit or worktree than the card showed.
- A stranded channel: after any successful branch creation on a worktree, no active channel of that worktree keeps a detached-style binding while HEAD has a branch.

## Approaches

- **A. A new request kind in the slice B write-request store. (Chosen.)** `WriteRequestKind::BranchCreate` reuses the TTL, one-pending-per-channel-and-kind rule, card polling, and the continue-turn path. The gate differs from the moderator-only move: any agent with `workspace_write` and policy Ask in a detached worktree channel.
- **B. Extend `conversation_request_move` with a "here" target.** Rejected: that tool is moderator-only and copies a handoff. This action keeps the same conversation and needs no handoff.
- **C. Let the agent run `git switch -c` in its sandbox.** Rejected: the sandbox blocks writes to `.git`, and the action needs the user's approval.

## Design

- **Tool.** `worktree_request_branch` with one argument, `name` (a branch name). It does nothing until the user approves. Listed only when the channel is a detached worktree channel, the agent has `workspace_write` with policy Ask, and `identity.can_write` holds. Returns `{approvalRequired, request, message}` like the other request tools.
- **Request.** `WriteRequestKind::BranchCreate`, with the new branch name, the worktree path, and the HEAD commit at request time. One pending request per channel and kind; TTL and eviction as for the other kinds. Denial text: "Branch creation was denied."
- **Gate (`write_request_target` for BranchCreate).** Project exists, channel active, agent opted in with policy Ask, channel is a Legacy channel with a worktree record whose worktree is a linked worktree with a detached HEAD at its recorded commit, and the name passes `validate_branch_name` for that repository (valid, not existing). Otherwise the call fails with a plain reason.
- **Approval.** `decide_write_request` for BranchCreate holds the channel (`begin_write_approval`), re-runs the gate, and compares the name, path, and commit with the request ("The request target changed. Ask again."). It then runs `branch_actions::create_branch(root, name)`, rebinds the channel, refreshes branches, and finishes the request. It queues the same scoped continue turn as WorktreeCreate. A Git failure fails the request with Git's message and rebinds nothing.
- **Rebind (shared).** One function converts the detached channels that use a worktree path to branch channels of the new branch: `bind_repository_branch(channel, repository, name)` and `worktrees.branch = name`. `Channels::create_branch` (menu) calls it for the detached channels of the tree when the tree was detached. If rebinding fails after the branch exists, the error says so and the branch stays.
- **Card.** The web card for `branch_create` reads "Create branch `name` here?" and shows the worktree path and short commit. Buttons: Create branch, Deny. Plain text only.
- **Providers.** Add the tool to `MEMORY_TOOLS` and the Claude allowlist string, and to the hard-coded assertion in `codex_process.rs`. Add one prompt line for detached channels in `service.rs`.

## Left to the builder

- The exact field for the commit in `WriteRequest`.
- The module that holds the shared rebind (store method vs `Channels` helper).
- Card layout details.

## Tests

- Core: gate accepts a detached worktree channel; refuses a branch channel, an existing name, a bad name, a non-opted-in agent; request dedupe and denial text; rebind keeps channel ID and history and makes `verify_binding` pass.
- App: approve creates the branch at the commit and rebinds; deny changes nothing; approval after HEAD moved refuses; Git failure rebinds nothing; menu **Create Branch…** rebinds a detached channel (the stranding defect).
- MCP: the tool lists only for a detached channel with an opted-in agent; call returns approvalRequired.
- Web: card title, approve, deny.

## Docs

`docs/worktrees.md` (not-built list and the worktree section), `docs/project-channel-status.md` (planned item), the slice C plan's mention.
