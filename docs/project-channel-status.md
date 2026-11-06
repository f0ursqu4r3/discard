# Project channel foundation status

Updated: 2026-11-06

The project-channel foundation is implemented. It adds durable project channel identities, saved project navigation, a project main-agent setting, channel memberships, versioned topics, project navigation and details, and topic context in Conversation-mode prompts. The approved design in `docs/superpowers/specs/2026-11-05-project-channel-orchestration-design.md` describes a larger orchestration system. This status page describes the implemented foundation only.

## Implemented behavior

- The channel bar discovers repositories in the project folder and its descendants. Each repository has collapsible Branches and Worktrees lists. With multiple repositories, each has a collapsible heading. With one repository, the heading is hidden. Collapse state is saved per project and repository.
- Discovery groups linked worktrees under one repository and includes registered worktrees outside the project folder. It does not follow directory symlinks. It skips Git metadata, dependency folders, and common build folders. Duplicate repository names use relative paths. The existing five-second Git refresh also refreshes discovery.
- Schema version 7 scopes branch channel identity by repository. Repositories can have branches with the same name without sharing channels. Existing project-root branch channels retain their identity. Worktree preparation uses the channel's repository.

- Schema version 6 adds an optional saved project icon and color. Project settings can edit the display name or alias without changing the project ID or folder. The rail, project header, and overview use the saved appearance.
- Schema version 5 extends the project-channel metadata contract for retained bootstrap channels. Schema version 4 migrates existing project conversations. It preserves messages and sessions, selects an active canonical `general` channel and active channel per branch when an active candidate exists, and keeps duplicate or unbound histories as linked archives. If every candidate is archived, migration leaves the group without an active canonical channel.
- Active workspace channels created by the retained Builder/bootstrap path use kind `legacy` when they have no general or branch identity. This kind preserves their channel ID, Coding mode, archive state, session, history, and agent membership. Bootstrap registers metadata and membership on creation and reuse. These channels remain listed and openable by saved channel ID. They do not become canonical `general` or branch channels implicitly.
- Project selection restores the project's last active channel. A first visit opens `general`. Direct-chat selection does not change project navigation.
- Branch selection reuses the canonical branch conversation and its history. Store APIs can rename a branch binding or retire it to release the name for reuse. App detection of Git branch renames and user-facing branch retirement remain pending. Branch selection does not switch Git branches or create worktrees.
- The project main agent is stored per project. New project setup uses the first active configured agent with an explicit model in Store order if no main agent is saved. All new unmentioned Conversation-mode replies use the current project main agent, including existing branch and worktree channels.
- Project and channel memberships persist. Membership alone does not dispatch a mention or start a response.
- Topics use optimistic version checks and retain revision history. The latest topic enters each Conversation-mode prompt as labelled context outside the transcript. Topic text does not dispatch tools or mentions, and explicit system instructions and the user message take precedence.
- New replies persist their author ID, name, provider, and avatar. Live replies and transcript history show that author. Older replies without author metadata use the saved channel identity.
- A user can mention one configured agent with `@Name` or `@"Full Name"`. The composer offers autocomplete. The selected agent joins the project and channel and answers with its own configuration and memory. Unknown or duplicate names fail before sending.
- Project settings expose the Primary / Moderator agent selector. This uses the existing project main-agent setting. It sets the default for each new unmentioned reply in every project channel. Existing branch and detached worktree channels read this setting at send time. The selector is disabled while the general channel is running.
- Normal Conversation mode does not materialize branch worktrees. Worktree behavior remains separate from opening a project or branch conversation.

## Remaining approved design

These items are later implementation slices:

- Concurrent multi-agent replies, structured mention tokens, and agent-initiated delegation.
- A durable task graph, queued steering, bounded delegation, project concurrency, and worktree mutation locks.
- Main-agent tools for branch and channel creation, topic handoffs, progress links, and integration.
- Descendant cancellation, restart reconciliation, paused-task recovery, and branch lifecycle detection.
- Provider-cache-aware input and context retrieval that retains required evidence.

The current main-agent setting assigns the general channel's responder. It does not schedule specialists. The conversation flow does not create, switch, or mutate branch worktrees, and the app does not claim full task orchestration or recovery.
