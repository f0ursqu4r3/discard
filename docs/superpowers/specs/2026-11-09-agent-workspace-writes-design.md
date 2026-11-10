# Agent workspace writes — design

**Date:** 2026-11-09
**Status:** Approved by the owner through the design interview on 2026-11-09.
**Builds on:** [Project, branch, channel, and agent orchestration](2026-11-05-project-channel-orchestration-design.md)

## Intent

Agents in project channels can only read today. They must be able to edit files, run commands, and commit in the branch or worktree of their channel. The user controls each grant of access through approvals.

## Current state

- `crates/discard-app/src/main.rs:193` sets one `SessionMode` for the whole process. A normal launch uses `Conversation`. `--builder` uses `Coding`.
- `Conversation` gives the provider an empty working directory (`crates/discard-providers/src/conversation.rs:19-24`). Native tools are off:
  - Claude: `--tools ""` and `--permission-mode manual` (`claude/provider.rs:55-114`).
  - Codex: the model catalog removes shell and patch tools (`conversation.rs:88-93`), `codex_controls` turns off tool features (`conversation.rs:116-142`), and the mode table gives `("never", "read-only")` (`codex/provider.rs:130-133`).
  - Only the scoped Discard MCP tools are available.
- `Coding` has native tools, a worktree working directory (`engine.rs:349-366`), and host approvals. It has no transcript replay, summaries, mentions, or topic, project, and worktree context.
- Approvals: a provider request becomes `EventKind::ApprovalRequest`, renders as `ApprovalCard.vue`, and returns through `Command::Decide` to `respond_approval`. Decisions are audited in `approvals` and `approval_audit` (`store.rs:79-93`). "Always allow" lasts only for one provider session. Discard keeps no rule table.
- Worktrees: `WorktreeManager::prepare` creates `discard/<slug>-<id>` branches, and `prepare_branch` attaches an existing branch. A branch that is checked out anywhere, including the main checkout, is borrowed with `owned=false` (`worktree.rs:349-351`). Locks cover only `prepare` and `remove`.
- The transcript saves `ToolCall` and `Approval` blocks. Conversation replay includes them in full (`conversation_context.rs:26-55`).
- Agents have `policy: Ask | DenyAll` and `plugin_write`. The confirmation for loosening a policy exists only in the settings flow (`agent.rs:102-109`, `settings/agents.rs:104`).
- Plugins can hold `approvals.decide` (`discard-lua/src/manifest.rs:26`).

## Decisions

### Where agents write

1. Only branch channels and worktree channels get write sessions. This includes detached worktrees and worktrees that Discard did not create. The agent works in the branch or worktree of its current channel. `#general` and direct chats stay read-only.
2. Agents never write in the main checkout. There is one exception. In the channel whose branch is checked out in the main checkout, the agent can request main-checkout write access on an approval card. The grant lasts for one response. Discard never takes the grant from message text, topics, or files. When a user asks for main-checkout work in `#general`, the agent links to that branch channel.
3. Each agent has `workspace_write: bool` in `AgentConfig`, false by default. Turning it on uses the existing policy-loosening confirmation (`AgentChange::needs_user_approval`). Agents with policy `DenyAll` never get write sessions.
4. A response gets a write session only when all of these are true:
   - The conversation is a project channel bound to a branch or worktree.
   - The bound directory exists. It is not the main checkout, unless a main-checkout grant is active for this response.
   - The responding agent has `workspace_write` and policy `Ask`.
   - The provider can enforce the sandbox (decision 11).

   Otherwise the response uses the read-only Conversation profile. Its prompt states the reason once: no worktree, agent not opted in, main checkout, or sandbox not available.

### Session profile

5. Add `SessionMode::Workspace`. The engine chooses it for each response, not for the process.
   - The engine treats it like `Conversation` for prompt building, replay, memory, topic, project and worktree context, mentions, summaries, and author metadata.
   - Provider adapters treat it like `Conversation` for isolation: no native resume, no user MCP servers, no hooks, no automatic provider memory, and no provider plugins.
   - Adapters set the working directory to the granted directory (the session root) and turn on the write tool set.
   - The `--builder` `Coding` path does not change.
6. Each response starts a fresh provider session with transcript replay. Replay compacts tool calls in every Conversation-family profile:
   - A `ToolCall` block replays as the tool name, the paths or command, the exit status, and the first 1 KiB of output with a truncation marker.
   - A changes card replays as its file list and commit IDs.
   - The saved transcript keeps the full blocks.
   - A provider session for each agent, with native resume, is a later slice.
7. Tool set for write sessions:
   - Claude: `Read`, `Glob`, `Grep`, `Edit`, `Write`, `Bash`, and the scoped Discard MCP tools.
   - Codex: shell, `apply_patch`, and the scoped Discard MCP tools.
   - Off: web fetch and web search, subagents and delegation, skills, slash commands, user MCP servers, hooks, provider plugins, code mode, and the node REPL.
8. Repository instructions: Discard reads `AGENTS.md` and `CLAUDE.md` from the session root and adds them to the system prompt, labelled as repository instructions. Each file is limited to 32 KiB, and a longer file gets a truncation notice. System instructions and the current user message take precedence. Provider discovery of instruction files stays off (Claude `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`; Codex project-doc discovery off). The user's global provider instructions stay off where the provider allows it. Codex's stated limit for global `AGENTS.md` and administrator policy still applies.

### Approvals

9. Defaults in a write session:
   - **File edits** by provider edit tools: when every target path resolves inside the session root, Discard approves automatically and audits the decision with policy `workspace_edit`. Otherwise Discard shows a card. Adapters do not use `acceptEdits`, because it also approves some shell commands.
   - **Shell commands** that the provider asks about get an approval card with Approve, Deny, and Always allow. Read-only commands that the provider runs without asking stay as they are.
   - **Writes outside the session root and network access** fail inside the sandbox. Requests to escape follow decision 12.
   - **Destructive Git commands** cannot run inside the sandbox, because `.git` is not writable (decision 15). They need an escape approval.
10. Always allow (slice B): Discard keeps its own rule table keyed by agent, project, and command prefix.
    - Rules match the parsed command words, for example `cargo test`.
    - Commands with `&&`, `;`, `|`, redirects, subshells, command substitution, inline environment assignments, or `sudo` never match a rule.
    - When a rule matches, Discard answers the provider prompt and the audit records the rule ID.
    - Project settings list the rules, and the user can revoke each one.
    - In slice A, Always allow in write sessions is visible and disabled, and it shows `Soon`. `Coding` sessions keep their current session-only behavior.
11. An OS sandbox is required, and sessions fail closed without it.
    - Codex: `sandbox: "workspace-write"`, network off, and the session root as the only writable workspace root (system temp folders stay writable).
    - Claude: session settings `sandbox.enabled: true`, `sandbox.allowUnsandboxedCommands: false`, and `sandbox.autoAllowBashIfSandboxed: false`.
    - Before it sends the prompt, the adapter checks the effective controls, as Conversation setup does today. If a control is missing or overridden, the response uses the read-only profile and shows a notice.
12. Escape requests get a separate card: "Runs outside the sandbox: full access as you". This card has no Always allow. Rules and plugins never decide it. If Claude's sandbox asks for each network domain, the card is "Allow network access to `<domain>`", and slice B adds Always allow for each domain in each project.
13. Plugins with `approvals.decide` can decide shell-command approvals in write sessions. Rust refuses plugin decisions for these approval kinds: sandbox escape, network domain, main-checkout write, worktree creation, conversation move, branch creation, and `workspace_write` changes. Every plugin decision is audited.
14. Each approval has a kind: `ProviderTool`, `SandboxEscape`, `NetworkDomain`, `MainCheckoutWrite`, `WorktreeCreate`, `ConversationMove`, or `BranchCreate`. The app renders all of these cards, and plugins can theme them but not replace or hide them.

### Git

15. Agents commit through a new scoped MCP tool, `worktree_commit(message, paths?)`.
    - Discard runs `git add` and `git commit` outside the sandbox, in the session root only.
    - Hooks are off. Interactive prompts are off. Each Git command has a time limit.
    - The commit uses the user's Git identity and signing configuration. If signing fails, the tool returns an error.
    - Without `paths`, the commit stages all changes in the session root, including untracked files that are not ignored.
    - The tool refuses on a detached HEAD, in the main checkout without an active grant, and while another session holds the write lock.
    - The commit has no agent trailer. Discard records the commit ID, agent, conversation, and response in a new `agent_commits` table.
    - Write sessions set `GIT_OPTIONAL_LOCKS=0`, so read-only Git commands such as `status`, `diff`, and `log` work inside the sandbox.
16. Agents never push, merge, rebase, or delete branches unless the user asks. No tool does these actions. The sandbox blocks them from the shell, and an escape needs the user's approval.
17. Detached worktrees: agents can edit, but `worktree_commit` refuses. In slice C, the agent requests "create a branch here". When the user approves, Discard creates the branch at HEAD and moves the conversation to that branch's channel.

### Getting a write session (slice B)

18. A branch channel without a worktree runs read-only. The agent can call `worktree_request_creation`. When the user approves the card, Discard creates the worktree and starts a write-capable follow-up response for the same request.
19. The main-checkout grant (decision 2) uses the tool `main_checkout_request_write` and works the same way. When the user approves, the follow-up response runs with the main checkout as its session root.

### Slice B details (approved 2026-11-09, after slice A)

20a. **Prefix choice.** The Always-allow control shows the parsed command words with a default cut: the program plus its first word that is not a flag. For `cargo test -p discard-core workspace`, the default is `cargo test`. The user can widen or narrow the cut before saving. The rule saves the words up to the cut.
20b. **Codex.** Slice B spikes once for a reliable marker that separates Codex command approvals inside the sandbox from escalation requests. If the spike finds one, Codex command approvals inside the sandbox get the `ProviderTool` kind and can use Always allow. If it finds none, the slice A fallback stays: every Codex command approval in a write session is `SandboxEscape`, and the card says "Always allow is unavailable for Codex write sessions."
20c. **Network rules.** A network-domain rule matches the exact host only, with no wildcards. It is kept for each agent in each project.
20d. **Main checkout with Codex.** The main checkout contains `.git` inside the session root. If a spike shows that Codex's `workspace-write` sandbox does not keep `.git` read-only, Codex agents cannot get main-checkout grants. The request fails with that reason.
20e. **Main checkout with Claude (owner decision, 2026-11-09, after the slice B spike).** The spike found that approved Codex commands and patches change `.git`, so 20d applies and Codex agents get no main-checkout grants. Codex agents never see `main_checkout_request_write` or its prompt line. Claude agents do get main-checkout grants:
   - Claude's sandbox `denyWrite` on `<root>/.git` blocks shell writes even after approval.
   - Claude file-tool edits to `.git` are classified `SandboxEscape`, so they need the full-access card. Linked worktrees have the same protection.

### Moving a conversation (slice C)

20. Only the project's Primary/Moderator agent can call `conversation_request_move`. Other agents can suggest a move in text.
21. The move card shows the target and a handoff written by the moderator: goal, constraints, decisions, and open questions. When the user approves:
    - Discard posts the handoff and the user's original message, with its attachments, in the target channel.
    - Discard links both channels to each other and starts a response in the target.
    - The source transcript is not copied.
    - A new channel gets the handoff as its topic. An existing channel keeps its topic.
22. A move can go to an existing channel or to a new branch. A new branch starts from the default branch, or from the branch the work depends on. The card shows the branch name and the base commit, and Discard records both. For a new branch, one approval covers the branch, the worktree, and the first response.
23. The user's view stays in the source channel. The source gets a "Moved to #channel" link card, and the target gets a "Moved from" link.

### Review and interruptions

24. Each write response ends with a changes card. Plugins cannot override or hide it, as with approval cards.
    - Before the session starts, Discard records a snapshot with read-only Git: HEAD, `git status --porcelain=v2`, and content hashes of dirty files.
    - The card shows commits from the start HEAD to the end HEAD, and the uncommitted paths whose content changed since the snapshot.
    - Each file has an expandable diff. Diffs are size-limited, and a large diff shows a truncation notice.
    - Discard builds the card after the response completes, fails, or is stopped.
25. Stop keeps all edits and commits. The changes card shows a "Stopped" state.
26. Background approvals use the existing counts for each channel and project. They have no timeout. After a restart they are stale (existing reconcile behavior), and the response ends with a notice that tells the user to send again.
27. Write lock:
    - The lock is keyed by the canonical session-root path.
    - A write response holds it from session start until the changes card is saved. `worktree_commit` holds it while it runs.
    - A second write response for the same path waits in a queue and shows "Waiting for #channel". The user can cancel it while it waits.
    - The lock works only inside Discard. External processes are not controlled.

### Prompt

28. The write-session prompt says:
    - which directory is writable
    - that file edits are approved automatically and shell commands ask
    - that commits go through `worktree_commit`
    - that the agent must not push or merge unless the user asks
    - what the sandbox blocks
    - that results go in the reply, and the changes card carries the details

## Security limits

- A write session runs as the user's OS user. The sandbox limits writes and network access, not reads. An agent can read other sessions' MCP tokens. Discard does not present MCP scoping as isolation for write sessions.
- Repository files and instruction files are untrusted data. They rank below system instructions and the user's messages.
- Managed or remote provider policy stays authoritative. Discard does not claim isolation from the provider's administrator.

## UI honesty

Follow `AGENTS.md`. Planned controls stay visible, disabled, and labelled `Soon`, with a `TODO` comment beside them in the code. Examples: Always allow in write sessions until slice B, and move and branch actions until slice C. A control that is disabled because of current state needs no `TODO`.

## Delivery

- **Slice A, write sessions:**
  - the spike (below)
  - `workspace_write`, `SessionMode::Workspace`, eligibility, adapters, sandbox checks, the tool set, and repository instructions
  - automatic file-edit approval, approval kinds, escape cards, and plugin-decision limits
  - `worktree_commit` and `agent_commits`, the changes card, the write lock and queue, replay compaction, and the prompt
- **Slice B, getting a write session:** worktree creation requests, the main-checkout grant, the Always-allow rule table and its settings list, and network-domain Always allow.
- **Slice C, moves:** the move tool and card, new-branch targets, links, and "create a branch here".

Each slice needs unit tests, process-fixture tests with the fake providers, opt-in live probes, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm --dir web check`, `pnpm --dir web test`, and `pnpm --dir web build`.

### Spike (start of slice A)

Record each answer in the capability tables in `docs/provider-claude.md` and `docs/provider-codex.md`.

1. Codex 0.166.0 `app-server` with `approvalPolicy: "untrusted"`, `sandbox: "workspace-write"`, and a real working directory:
   - Does `apply_patch` inside the root send `item/fileChange/requestApproval`?
   - How is an escalation request marked?
   - Which `availableDecisions` arrive?
   - Which config turns off project-doc discovery and sets the writable roots?
2. Claude 2.1.318 with `--tools "Read,Glob,Grep,Edit,Write,Bash"`, `--permission-mode manual`, `--permission-prompt-tool stdio`, and the sandbox settings:
   - Does `Bash` run inside the sandbox?
   - Which `can_use_tool` requests arrive for `Bash` and `Edit`?
   - How does a network-domain request arrive?
   - Does `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` still apply with a real working directory?
