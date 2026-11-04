# Plan Review Log: Discard
Act 1 (grill) complete — plan locked with the user (28 decisions, 4 rounds). MAX_ROUNDS=5.

## Round 1 — Codex

The plan needs revision before implementation. It leaves security boundaries, provider behavior, and recovery rules undefined.

1. **Critical — Plugin edits may load without review.** [PLAN.md:99](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:99) requires approval only for first load and capability changes. An agent could change plugin behavior without changing its capabilities. **Fix:** Require approval for every code change unless the user has enabled the stated trust option.

2. **Critical — MCP access has no caller boundary.** [PLAN.md:55](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:55) and [PLAN.md:97](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:97) expose memory and plugin tools without defining how calls are tied to an agent, project, or policy. **Fix:** Bind each MCP connection to a host-issued session identity and authorize every tool call and scope.

3. **Critical — Plugin capabilities do not define effective limits.** [PLAN.md:85](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:85) permits scoped filesystem, network, and process access, but the manifest lists only broad capability names. An approved process could also bypass host-level filesystem or network restrictions. **Fix:** Define enforceable path, host, executable, and subprocess limits; treat unrestricted execution as full local access.

4. **High — Plugins can undermine approvals.** [PLAN.md:51](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:51) permits automatic decisions, while [PLAN.md:82](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:82) lets hooks modify or block approval events. Themes can also make protected prompts unreadable. **Fix:** Make Rust enforce policy and approval presentation, require explicit authority for auto-approval, and fail closed on hook errors or timeouts.

5. **High — Provider support is assumed rather than specified.** [PLAN.md:36](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:36) does not select a protocol or prove that each provider supports interactive approvals, tool interception, MCP configuration, and resume. Codex documents app-server as an integration for interactive applications, with a distinct lifecycle and approval protocol. [Codex platform guidance](https://developers.openai.com/blog/codex-as-a-platform) **Fix:** Define and test a versioned capability matrix and integration contract for each provider before committing to the shared UI.

6. **High — SQLite cannot by itself restore the provider’s context.** [PLAN.md:44](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:44) calls SQLite the source of truth but treats the CLI session ID as a resume handle; the provider may retain context or state that the normalized event stream cannot reconstruct. **Fix:** Define which provider state is authoritative, detect missing or invalid resume state, and specify an explicit recovery flow.

7. **High — Lazy worktree creation can change a session’s workspace mid-run.** [PLAN.md:45](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:45) does not define how the app identifies a write-capable turn, moves an existing session, or handles uncommitted files. Concurrent channels can also race during creation. **Fix:** Select and create the workspace before starting the provider session, serialize worktree creation, and define dirty-tree, failure, and cleanup behavior.

8. **High — Plugin review has a time-of-check/time-of-use gap.** [PLAN.md:98](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:98) does not bind the reviewed diff to the bytes that load; concurrent edits, symlinks, or reloads could change them. **Fix:** Review an immutable snapshot and atomically install and load that exact content hash.

9. **High — Runtime limits are incomplete.** [PLAN.md:68](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:68) promises to kill over-budget calls but does not cover memory exhaustion, blocking native calls, queued events, or a blocked shared VM thread. **Fix:** Specify enforceable CPU, memory, queue, and I/O limits, and isolate untrusted plugins so one cannot stall the host or other plugins.

10. **High — Memory scope and updates lack conflict rules.** [PLAN.md:58](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:58) defines files and a rebuildable index, but not path validation, concurrent edits, atomic writes, or index recovery after a crash. **Fix:** Define canonical scope IDs, reject path escapes, serialize or version writes, use atomic replacement, and reconcile the index on startup.

11. **Medium — Approval and event recovery are unspecified.** [PLAN.md:38](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:38) and [PLAN.md:49](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:49) omit event IDs, ordering, duplicate decisions, process exits, and stale pending approvals. **Fix:** Define a durable turn and approval state machine with idempotent decisions, restart reconciliation, and an audit record.

12. **Medium — Failure handling needs acceptance tests and diagnostics.** [PLAN.md:22](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:22) specifies mostly unit tests, while the design depends on child processes, worktrees, MCP, and crash recovery. **Fix:** Add end-to-end failure tests and structured, redacted logs for provider, plugin, approval, storage, and workspace operations.

VERDICT: REVISE
### Claude's response (Round 1)
Accepted all 12 findings. Changes to PLAN.md:
1. Plugin edits: review now required on every code change. Trust toggle (UI-only) still skips code review but never capability changes. (PLAN.md had drifted from the grilled decision.)
2. MCP: per-session token bound to agent, project, policy. Local-only transport. Every call authorized.
3. Capabilities: host-enforced path, host, and executable scopes. Broad `exec` shown as full local access.
4. Approvals: Rust enforces policy and presentation. Auto-decide needs `approvals.decide`. Hooks fail closed. Contrast floor on themes.
5. Providers: M1 spike and versioned capability matrix before shared UI. Degraded modes explicit.
6. Resume: validate handle, explicit recovery flow with transcript-seeded new session.
7. Worktrees: create before provider session start, serialized per repo, dirty and failure rules. **Reverses the user's earlier "lazy on first write-capable turn" decision. Flagged for sign-off.**
8. TOCTOU: immutable snapshot, content hash, atomic install.
9. Limits: one Luau state per plugin, pooled threads, CPU, memory, queue, host-call timeouts.
10. Memory: canonical scope IDs, path validation, atomic writes, versioned edits, startup index reconcile.
11. State machine: event IDs, sequence numbers, turn states, idempotent approvals, stale handling, audit table.
12. Tests and diagnostics: E2E failure test list, redacted structured logs.
No findings rejected.

## Round 2 — Codex

The revision addresses most prior findings. It adds provider capability checks, memory write rules, plugin review snapshots, approval audit records, runtime limits, and failure tests. Several security and lifecycle gaps remain.

1. **Critical — Lua can create agents with stronger policies.** [PLAN.md:36](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:36) allows Lua to create agents, and the agent config contains the permission policy. The plan does not require user approval for that policy. **Fix:** Require Rust-enforced user approval for agent creation and policy changes, including changes initiated by plugins.

2. **Critical — Hooks may not intercept provider tool calls.** [PLAN.md:96](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:96) promises hooks that modify or block tool calls, but [PLAN.md:44](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:44) permits providers that lack the needed capability to run in degraded mode. A tool event received after execution is too late to block it. **Fix:** Define which hooks each provider can enforce before execution, and disable unsupported blocking hooks rather than presenting them as active.

3. **High — Tool degradation can bypass approval policy.** [PLAN.md:44](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:44) permits a degraded mode when interactive approvals are unavailable, but does not say whether tools can still run. **Fix:** Fail closed when a provider cannot enforce the configured policy, and require an explicit policy change before enabling a less restrictive mode.

4. **High — MCP identity can be exposed to the agent.** [PLAN.md:67](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:67) does not define how the CLI receives its token or protects it from tools and child processes. A token visible in process arguments, environment, or provider-accessible configuration can be copied and reused. **Fix:** Specify token delivery, lifetime, revocation, and the local attacker boundary; use separate narrowly scoped connections and prevent disclosure where the provider supports it.

5. **High — Executable allowlists overstate isolation.** [PLAN.md:102](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:102) labels only broad execution grants as full local access, but an allowed executable can read files, access the network, or launch other programs. **Fix:** Treat every unsandboxed `exec` grant as full local access, or run it inside an enforced OS sandbox.

6. **High — A clean worktree can contain valuable commits or ignored files.** [PLAN.md:53](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:53) removes clean worktrees on archive without defining checks for unmerged commits or ignored files. **Fix:** Preserve branches and check for unmerged commits, untracked files, and ignored files before removal; ask before deleting any user data.

7. **High — Protected approval rendering conflicts with plugin overrides.** [PLAN.md:90](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:90) allows plugins to override approval blocks, while [PLAN.md:121](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:121) says approval prompts cannot be replaced or hidden. **Fix:** Exclude approval blocks and their containers from plugin overrides, and define limits for plugin-controlled theme values.

8. **Medium — The shared thread pool does not ensure plugin isolation.** [PLAN.md:80](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:80) says one plugin cannot block another, but a blocked native operation or enough occupied workers can exhaust the pool. A timeout also does not necessarily stop underlying work. **Fix:** Define cancellation and worker isolation for host calls, bound concurrent work per plugin, and test pool exhaustion.

9. **Medium — Worktree rollout lacks a migration rule.** [PLAN.md:129](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:129) creates sessions in M1, but worktrees arrive in M6. Existing sessions already have provider context tied to a directory. **Fix:** Keep existing sessions in their original workspace and define an explicit new-session migration flow.

10. **Medium — Resume recovery can change the task context materially.** [PLAN.md:45](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:45) offers a summary-seeded session but does not define who creates the summary or whether the user can inspect it. **Fix:** Show the proposed summary and require user confirmation before starting the replacement session.

VERDICT: REVISE
### Claude's response (Round 2)
Accepted all 10. Finding 4 accepted with a scope note: the agent process holds its own token by design, so the protected boundary is between sessions, not agent versus itself.
1. Agent creation and policy changes need Rust-rendered user approval, including plugin-initiated ones.
2. Hook enforcement matrix per provider. Pre-execution hooks active only where enforceable. Others shown as observe-only.
3. Fail closed when a provider cannot enforce policy. No silent loosening.
4. Token handling: per-session token, 0600 config file outside the project, no CLI args, expiry, revocation, peer check, separate memory and plugin connections. Same-user hostile process declared out of scope.
5. Every `exec` grant shown as full local access. Allowlist is a record of intent. OS sandbox post-v1.
6. Worktree removal checks unmerged commits, untracked and ignored files. Branch always kept. Ask before any data loss.
7. Approval, review, and error blocks and their containers are not overridable. Theme values clamped.
8. Host calls cancellable, blocking pool with per-plugin cap, pool exhaustion test.
9. Workspace mode recorded per channel. Pre-M6 channels stay put. Move is explicit and starts a new session.
10. Resume summary shown to the user and confirmed before the new session starts.
No findings rejected.

## Round 3 — Codex

The revision addresses most prior findings. It adds policy approval, provider enforcement checks, protected approval surfaces, worktree migration rules, and summary review. Three material gaps remain.

1. **High — Session tokens do not enforce the stated boundary.** [PLAN.md:71](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:71) stores all session tokens in files readable by the same OS user. An agent with shell or filesystem access can act as that user and read another session’s token. Excluding hostile same-user processes on line 72 also excludes that case, so the claim that one agent cannot use another’s identity is not supported. **Fix:** Enforce per-session isolation and test cross-session access, or state that session tokens do not protect against agents with local filesystem access.

2. **High — Direct plugin changes can bypass review.** [PLAN.md:121](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:121) limits the `plugin_*` tools to drafts, but an agent with filesystem access can modify the live plugin directory directly. The plan does not require a hash check at startup or reload. **Fix:** Load only plugin content whose hash matches an approved snapshot, and reject unapproved disk changes on every load and reload.

3. **High — Host-call cancellation is not guaranteed.** [PLAN.md:86](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:86) says timeouts abort underlying work, but dropping an async task does not necessarily stop blocking work or reverse side effects. A capped shared pool can still become unavailable if several plugins exhaust it. **Fix:** Define which operations can be cancelled, isolate non-cancellable work in killable processes or reserved workers, and test timeout and pool-exhaustion behavior.

VERDICT: REVISE
### Claude's response (Round 3)
Accepted all 3.
1. Session tokens: plan now states they are routing, scoping, and audit, not a defense against a same-user agent with shell or filesystem access. Real defense is the provider sandbox and approval policy. UI shows each agent's effective sandbox and approval mode. OS isolation post-v1.
2. Load-time hash verification on startup, reload, and file-watcher events against approved hashes stored in the app DB with a Keychain-held HMAC key. Mismatch disables the plugin and shows "modified outside review". Applies to all plugins.
3. Cancellation specified per operation type. Per-plugin reserved workers, no shared plugin pool. FS ops bounded and non-reversible, documented. Tests added.
No findings rejected.

## Round 4 — Codex

The revision addresses the three prior findings in part. It now states the token limit, checks plugin hashes, and defines cancellation by operation type. The plugin load design still has gaps.

1. **High — Hash verification does not bind the loaded bytes.** [PLAN.md:131](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:131) hashes the live directory, but a process can change a file after the check and before the loader reads it. File-watcher events do not close that race. **Fix:** Load from a captured snapshot and verify the hash of the exact bytes passed to the runtime.

2. **High — Trusted UI edits conflict with load-time verification.** [PLAN.md:129](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:129) permits later edits without review, but line 131 rejects any content that does not match an approved hash. **Fix:** Define an atomic path that accepts and records a new hash for a trusted UI-only plugin after verifying that its capabilities remain unchanged.

3. **Medium — Linux has no defined key storage.** [PLAN.md:131](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:131) requires macOS Keychain, but Linux is in the platform plan. **Fix:** Define a Linux key-storage mechanism and fail-closed behavior when secure storage is unavailable.

4. **Medium — Plugin worker ownership is contradictory.** [PLAN.md:85](/Users/la.kyle.dougan/git/personal/discard/PLAN.md:85) puts Luau states on a shared thread pool, while line 90 says there is no shared pool for plugin work. The isolation guarantee depends on which design applies. **Fix:** Specify separate execution and I/O worker ownership, including how a stalled plugin cannot exhaust shared execution capacity.

VERDICT: REVISE
### Claude's response (Round 4)
Accepted all 4.
1. Load from a captured in-memory snapshot. Hash the exact bytes given to the runtime. `require` resolves against the snapshot.
2. Trusted UI-only path: verify capabilities unchanged, then atomically record the new hash and commit.
3. Key storage per OS (Keychain, Credential Manager, Secret Service). Passphrase fallback. Fail closed with no user plugins.
4. Fixed a Round 1 contradiction: one OS thread per plugin plus reserved I/O workers. No shared plugin pool. Cap of 64 active plugins.
No findings rejected.

## Round 5 — Codex

The revision addresses all four prior findings. It binds plugin execution to captured bytes, defines how trusted UI-only edits receive a new hash, adds Linux key storage with a fallback, and assigns each plugin its own execution and I/O workers.

I found no new material issue that blocks implementation. The provider capability spikes remain an important early validation step.

VERDICT: APPROVED
### Resolution
Converged. VERDICT: APPROVED at round 5 of MAX_ROUNDS=5. Awaiting user sign-off.

## Act 3 — Build (M8)

### Round 1 — Codex build
Spec: `docs/superpowers/plans/2026-11-03-m8-context-menus-providers-agents.md`. Codex ran with `--sandbox workspace-write` (`--full-auto` no longer exists in Codex 0.166). It implemented Tasks 1 to 8 in one run and made no commits. It reported clippy clean and all tests passing, with the MCP loopback test run outside the sandbox. It marked the GUI checks of Tasks 4 to 7 as NOT RUN. It merged some planned test cases and added the manager tests after the code.

### Claude's verdict
Passed: `cargo clippy --workspace --all-targets -- -D warnings` is clean and `cargo test --workspace` shows 841 passed, 0 failed in a fresh run. I read the full diff. The agent confirm and block rules reuse `AgentChange::needs_user_approval` and `CapabilityMatrix::enforcement_for`. Archive uses `GitWorkspace::archive` and its existing data-loss confirmation. The plugin menu callback receives only `{ kind, id }`.
Open: no GUI check ran. Some plan tests are merged or missing (a separate `ui` capability test for `menu_item`, a test that a plugin item cannot displace a Rust item). Reusing an agent across projects shares agent-private memory between projects.

### Round 2 — Claude fix
Problem found in review: `find_agent` reused an agent across projects, so two projects shared agent-private memory. Claude replaced it with `Store::find_project_agent(project, provider, plugin_write)`, which looks only at the project's own channels (archived ones included). Tests added first and seen failing: `a_new_project_never_reuses_another_projects_agent`, `archiving_in_one_project_does_not_hand_its_agent_to_another`. After the fix: clippy clean, 844 tests pass, 0 fail. The plan text for Task 2 is updated to match.
