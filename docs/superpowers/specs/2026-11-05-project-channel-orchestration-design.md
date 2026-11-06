# Project, branch, channel, and agent orchestration

Status: Approved through the design interview on 2026-11-05. Implementation authorized by the user.

## Purpose

Opening a project channel and asking a question must be sufficient to start work. The app owns durable conversation history and context management. A project main agent coordinates specialists and development branches.

## Objects and invariants

- A project has one main agent, project membership, channels, settings, and shared goals.
- `#general` is the only active project channel without a branch. Direct agent conversations outside a project remain supported.
- Each branch has one canonical conversation. A conversation has a permanent ID independent of the branch name. Renaming a branch preserves the channel. Reusing a deleted branch name creates a new channel.
- Worktrees are temporary working directories. Removing one preserves committed work and the conversation. Preserve uncommitted changes before removal.
- Each channel has a versioned plain-text topic visible in channel details. Users and the orchestrator can edit it. Record the author and previous versions.
- Topics hold purpose, goals, scope, constraints, decisions, and handoff context. Task status, assignments, dependencies, and verification results are separate records. Topic edits cannot override explicit user instructions or erase active work.

## Navigation

- Selecting a project restores its last viewed channel. First visit opens `#general`.
- `#general` remains available for project questions, planning, coordination, and progress.
- Asking a question does not create a branch.
- Opening a branch resolves its canonical channel, creating it when necessary.
- Merged/deleted branch channels remain readable and available for discussion. Further development moves to an active linked branch channel.
- A main agent redirect or delegation posts a link. It does not automatically change the user's selected channel.

## Agent roles and membership

- The main agent is a saved, changeable project setting. New projects use the app default agent. Transferring the role preserves channels, assignments, and history.
- The main agent coordinates project goals and tasks. A specialist may be task lead without becoming project main agent.
- Choose the most suitable invited agent for each request, favoring continuity where suitability is equal. Explicit mentions select the addressed agent.
- Mention autocomplete lists configured project agents first, then other configured agents. Indicate when a mention will add an agent to the project. Unknown names never create agents.
- An explicit mention in a new user or agent message adds the named configured agent to the project and current channel and requests a response.
- Membership persists project-wide. Channel participation is local. Membership alone does not activate an agent or send all channel messages to them.
- Quotes, code, topics, summaries, retrieved history, edited messages, and replayed messages do not dispatch mentions.
- The main agent schedules delegated work, deduplicates dispatch, and enforces concurrency. Default: one main agent plus two active specialists, with queued overflow and a configurable limit.

## Branches and worktrees

- The main agent may create a branch/channel for authorized development, invite agents, and record a link in the originating conversation.
- Related work continues on an existing branch. Independent work starts from the default branch. Dependent work starts from its dependency's branch. Record the starting commit and dependency.
- Never silently copy uncommitted changes between worktrees.
- Create or attach a worktree when code execution or changes require it. Project conversation mode must not accidentally edit the main checkout.
- One agent at a time may mutate a shared worktree, including commands that mutate files or Git state. Parallel readers use stable revisions. Concurrent development needs separate branches/worktrees.
- The main agent tracks integration dependencies, assigns conflict resolution, and verifies the combined result.
- Completion and merge are separate. Merge requires an explicit user request or explicit project policy. Completing work does not automatically delete branches, channels, or worktrees.

## Task lifecycle

- New messages in an active channel update the current task by default. Read them at the next safe stopping point. Corrections steer work. Separate tasks queue or run independently. Explicit mentions still select recipients.
- Stopping a task stops its descendants across channels, preserving history, task state, and changes. Independent tasks continue.
- Save assignments, progress, and handoffs. After restart or agent failure, interrupted tasks are paused and resumable.
- Recovery reconciles completed actions before replay. A missing completion message is not evidence that an external command did not run.
- Detailed work belongs in its branch channel. Results, blockers, and user decisions return to the originating channel.

## Context and cache

- Preserve the complete visible history. Each agent has its own execution context/provider session inside the channel.
- Build context from relevant messages, instructions, decisions, topic, applicable memory, code references, and task state. Retrieve omitted evidence when needed.
- Preserve original messages when summarizing. Provider sessions may be replaced without losing durable history.
- Reuse stable input where provider caching supports it. Avoid repeated input and unnecessary context rebuilds.
- Correctness takes priority over token reduction. Never silently drop required current instructions, unresolved questions, or supporting evidence to satisfy an arbitrary fixed limit.
- A new branch channel receives a focused handoff in its topic and links to its source conversation; it does not copy the entire source transcript.

## Migration and compatibility

- Preserve all existing conversations and messages.
- Select one canonical channel per branch, using the most recently viewed matching channel by default. Allow choosing another.
- Preserve other matching conversations as linked archives. Never concatenate transcripts.
- Existing agent direct conversations remain outside the project channel invariant.
- This design supersedes the prior agents-and-memory design's single-agent project conversation and deferred group-chat constraints, and the branch-tree design's multiple channels per worktree assumption.

## UI honesty

Follow AGENTS.md: planned actions stay visible, disabled, labelled `Soon`, with an adjacent code TODO. Planned rail items use the shared model and have no executable handler. Do not substitute no-ops or sample behavior for an implementation.

## Delivery and review

The user requested a checkpoint commit before implementation, Luna with high reasoning for implementation, and orchestrator review/critique with required fixes. Build in tested slices; keep the remaining roadmap explicit. A slice is not evidence that the entire orchestration design is implemented.
