# Project channel foundation status

Updated: 2026-11-06

The project-channel foundation is implemented. It adds durable project channel identities, saved project navigation, a project main-agent setting, channel memberships, versioned topics, project navigation and details, and topic context in Conversation-mode prompts. The approved design in `docs/superpowers/specs/2026-11-05-project-channel-orchestration-design.md` describes a larger orchestration system. This status page describes the implemented foundation only.

## Implemented behavior

- Schema version 4 migrates existing project conversations. It preserves messages and sessions, selects an active canonical `general` channel and active channel per branch when an active candidate exists, and keeps duplicate or unbound histories as linked archives. If every candidate is archived, migration leaves the group without an active canonical channel.
- Project selection restores the project's last active channel. A first visit opens `general`. Direct-chat selection does not change project navigation.
- Branch selection reuses the canonical branch conversation and its history. Store APIs can rename a branch binding or retire it to release the name for reuse. App detection of Git branch renames and user-facing branch retirement remain pending. Branch selection does not switch Git branches or create worktrees.
- The project main agent is stored per project. New project setup uses the first active configured agent with an explicit model in Store order if no main agent is saved. The general channel uses the project main agent. A branch channel uses its assigned channel agent.
- Project and channel memberships persist. Membership alone does not dispatch a mention or start a response.
- Topics use optimistic version checks and retain revision history. The latest topic enters each Conversation-mode prompt as labelled context outside the transcript. Topic text does not dispatch tools or mentions, and explicit system instructions and the user message take precedence.
- Project channel replies use a neutral `Agent` label because messages do not store durable author identity. Replies without recorded authors may remain neutral. Direct-chat labels remain unchanged.
- Normal Conversation mode does not materialize branch worktrees. Worktree behavior remains separate from opening a project or branch conversation.

## Remaining approved design

These items are later implementation slices:

- Explicit mention syntax and composer tokens, agent recruitment, per-agent execution contexts, and durable response attribution.
- A durable task graph, queued steering, bounded delegation, project concurrency, and worktree mutation locks.
- Main-agent tools for branch and channel creation, topic handoffs, progress links, and integration.
- Descendant cancellation, restart reconciliation, paused-task recovery, and branch lifecycle detection.
- Provider-cache-aware input and context retrieval that retains required evidence.

The current main-agent setting assigns the general channel's responder. It does not schedule specialists. The conversation flow does not create, switch, or mutate branch worktrees, and the app does not claim full task orchestration or recovery.
