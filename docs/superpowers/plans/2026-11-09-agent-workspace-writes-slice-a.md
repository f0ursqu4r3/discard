# Agent Workspace Writes, Slice A — Implementation Plan

> **For agentic workers:** This plan is built with `codex-build`. The executor does not commit. The reviewer commits after reading the diff. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An opted-in agent in a branch or worktree channel edits files, runs approved shell commands inside an OS sandbox, and commits through Discard, and each write response ends with a changes card.

**Architecture:** The engine chooses a new `SessionMode::Workspace` for each response when the channel, agent, and provider allow it. Otherwise the response uses the existing read-only `Conversation` profile. `Workspace` keeps every Conversation feature (replay, memory, topic, mentions, summaries, authors). It sets the provider working directory to the channel's bound directory and turns on a small native tool set inside the provider's OS sandbox. Discard approves in-root file edits automatically, classifies approvals by kind, commits through a scoped MCP tool outside the sandbox, holds a lock for each worktree path, and builds a protected changes card after each write response.

**Tech Stack:** Rust (tokio, rusqlite, serde), Claude Code 2.1.318 (stream-json, `--permission-prompt-tool stdio`), Codex 0.166.0 (`app-server`), Vue 3 with shadcn-vue, Vite+.

**Spec:** `docs/superpowers/specs/2026-11-09-agent-workspace-writes-design.md`. Read it before you start. Slice A covers decisions 1, 3–9, 11–16, 24–28, and the slice A parts of decisions 10 and 17. Slices B and C are out of scope.

## Global Constraints

- Supported provider versions stay pinned: Claude Code `2.1.318`, Codex `codex-cli 0.166.0`.
- Write sessions fail closed. Any missing control, flag, sandbox, or binding gives a read-only response with a stated reason. A write session never falls back to looser settings.
- The `--builder` `Coding` path keeps its current behavior. Do not change `Coding` tests except to add new fields.
- Agents never write in the main checkout in slice A. The main-checkout grant is slice B.
- Hooks are always off for Discard's Git commands: `-c core.hooksPath=/dev/null -c core.fsmonitor=false`, `GIT_TERMINAL_PROMPT=0`.
- UI honesty (`AGENTS.md`): a planned control is visible and disabled, it shows `Soon`, and a `TODO` comment beside it in the code names the missing behavior. `TODO` never appears in user-facing text.
- User-facing text uses short, direct sentences and active voice.
- New schema changes go in migration `user_version` 8 in `crates/discard-core/src/store.rs`, after the existing migrations. Existing rows keep working.
- Every `AgentConfig { .. }` literal in tests gets `workspace_write: false`.
- Proof command (run from the repo root):
  `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`

## Review Focus

1. **Path escapes in automatic edit approval.** An edit target can use `..`, an absolute path outside the root, a symlink inside the root that points outside it, or a file that does not exist yet under a symlinked folder. Every such case must show a card and never be auto-approved. Task 4 has the tests.
2. **Replay boundary after compaction.** Existing conversations save a summary boundary `<message-id>:part:<n>`. Compaction changes the fragment count. A saved boundary must not resolve to "replay everything plus the old summary". Task 9 has the test.
3. **Lock release on every exit path.** Stop, provider crash, setup failure, and an error while the changes card is built must all release the worktree lock. A second channel waiting on the path must then proceed. Task 7 has the tests.
4. **Pre-existing uncommitted changes in borrowed worktrees.** The user may have dirty files before the response. The changes card must not present those files as agent changes unless their content changed during the response. Task 8 has the test.
5. **Hooks reachable from the worktree.** A tracked `core.hooksPath` folder (for example `.husky`) that the agent edited must not run when `worktree_commit` commits. Task 7 has the test.

---

### Task 1: Provider spike — workspace capability facts

Confirm how both CLIs behave in a write profile before the adapters change. Use the synthetic local endpoints and temporary provider homes that the existing probes use. Use no real credentials.

**Files:**
- Create: `crates/discard-providers/tests/probes/claude_workspace.py`
- Create: `crates/discard-providers/tests/probes/codex_workspace.py`
- Create: `crates/discard-providers/fixtures/claude-workspace-turn.jsonl`, `crates/discard-providers/fixtures/codex-workspace-turn.jsonl` (scrubbed recorded transcripts)
- Modify: `docs/provider-claude.md`, `docs/provider-codex.md` (new section "Workspace sessions (spike results)")
- Read first: `crates/discard-providers/tests/probes/claude_conversation.py`, `crates/discard-providers/tests/probes/codex_conversation.py`

**Interfaces:**
- Produces: written answers that Tasks 4–6 use. Each answer is one table row with the observed value and the probe command.

- [ ] **Step 1: Write `claude_workspace.py`.** It starts Claude 2.1.318 in a temporary Git repository (cwd = repo root) with `--permission-mode manual --permission-prompts host --permission-prompt-tool stdio --tools "Read,Glob,Grep,Edit,Write,Bash" --strict-mcp-config --disable-slash-commands --system-prompt-snapshot off`. It uses settings `{"disableAllHooks":true,"autoMemoryEnabled":false,"sandbox":{"enabled":true,"allowUnsandboxedCommands":false,"autoAllowBashIfSandboxed":false},"env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"1","CLAUDE_CODE_DISABLE_CLAUDE_MDS":"1","CLAUDE_CODE_PLUGIN_DIRS":"","GIT_OPTIONAL_LOCKS":"0"}}`. The synthetic model requests, in order:
  1. `Edit` on a file in the root
  2. `Write` to `../outside.txt`
  3. `Bash` `touch inside.txt`
  4. `Bash` `touch "$HOME/outside-marker"`
  5. `Bash` `curl -sI https://example.com`
  6. `Bash` `git status`
  7. `Bash` `git add -A`

  The probe approves every `can_use_tool` request and records it, with its `input` and any sandbox or network fields. It records which commands succeeded and which files exist afterward. It runs once with `--restricted` and once without.
- [ ] **Step 2: Write `codex_workspace.py`.** It starts `codex app-server` 0.166.0 with cwd = a temporary Git repository, `approvalPolicy: "untrusted"`, `sandbox: "workspace-write"`, `ephemeral: true`, and the catalog with shell and `apply_patch` kept. It runs the same seven actions (`apply_patch` for the edits) and records:
  - every server request (method, params, `availableDecisions`)
  - how an escalation request is marked
  - the effective `sandbox_workspace_write` values
  - whether `project_doc_max_bytes=0` stops `AGENTS.md` discovery
- [ ] **Step 3: Run both probes.** Commands: `python3 crates/discard-providers/tests/probes/claude_workspace.py` and `python3 crates/discard-providers/tests/probes/codex_workspace.py`. Expected: each prints a JSON report and exits 0.
- [ ] **Step 4: Record the results.** For each provider, add a table to its doc that answers each spike question from spec "Spike". Save the scrubbed transcripts as the fixtures.

---

### Task 2: Agent opt-in `workspace_write`

**Files:**
- Modify: `crates/discard-core/src/agent.rs` (field, `AgentChange`, `needs_user_approval`, tests)
- Modify: `crates/discard-core/src/agent_edit.rs` (`AgentDraft`, review, save)
- Modify: `crates/discard-app/src/desktop/settings/agents.rs` (`Draft`, `settings_agent_get` JSON)
- Modify: `web/src/bridge/api.ts`, `web/src/components/agents/AgentProfileEditor.vue`, related tests in `web/src/components/agents/`

**Interfaces:**
- Produces: `AgentConfig.workspace_write: bool` (`#[serde(default)]`), `AgentChange::SetWorkspaceWrite { agent: AgentId, enabled: bool }`, `AgentDraft.workspace_write: bool`, JSON field `workspaceWrite`.

- [ ] **Step 1: Write failing tests in `agent.rs`.**

```rust
#[test]
fn enabling_workspace_writes_needs_user_approval() {
    let change = AgentChange::SetWorkspaceWrite { agent: AgentId("a".into()), enabled: true };
    assert!(change.needs_user_approval(&Initiator::User));
    let off = AgentChange::SetWorkspaceWrite { agent: AgentId("a".into()), enabled: false };
    assert!(!off.needs_user_approval(&Initiator::User));
    assert!(off.needs_user_approval(&Initiator::Plugin("p".into())));
}

#[test]
fn old_agent_toml_reads_workspace_write_as_false() {
    let toml = "name = \"A\"\nprovider = \"claude\"\npolicy = \"ask\"\n";
    assert!(!AgentConfig::from_toml(toml).unwrap().workspace_write);
}
```

- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core agent::tests`. Expected: compile error, because `workspace_write` and `SetWorkspaceWrite` do not exist.
- [ ] **Step 3: Implement the field and the change kind.** Add an `agent_edit` review row with field label `"Workspace writes"` and values `"on"`/`"off"`. Turning it on sets `needs_confirm`. The settings draft reads and writes `workspaceWrite`. In `AgentProfileEditor.vue`, add a switch next to the `pluginWrite` switch with label "Edit files in branch channels" and help text "Lets this agent edit files and run approved commands in branch and worktree channels."
- [ ] **Step 4: Run the tests.** Command: `cargo test -p discard-core agent && cargo test -p discard-app settings && pnpm --dir web test -- agents`. Expected: PASS.

---

### Task 3: Write eligibility, session profile, and prompt

**Files:**
- Create: `crates/discard-core/src/workspace_session.rs` (register it in `lib.rs`)
- Modify: `crates/discard-core/src/provider.rs` (`SessionMode::Workspace`, `ProviderError::SandboxUnavailable`)
- Modify: `crates/discard-core/src/engine.rs` (choose the mode for each response, prompt sections)
- Modify: `crates/discard-core/src/ui_state.rs` (`Update::SessionProfile`, stored profile)
- Test: `crates/discard-core/src/workspace_session.rs` (unit tests), `crates/discard-providers/tests/engine.rs` (fake-provider engine tests)

**Interfaces:**
- Consumes: `AgentConfig.workspace_write` (Task 2), `Store::worktree_for`, `worktree_access::git`.
- Produces:
  - `SessionMode::Workspace` and `impl SessionMode { pub fn replays_transcript(self) -> bool }` (true for `Conversation` and `Workspace`).
  - `ProviderError::SandboxUnavailable(String)`.
  - `pub struct WorkspaceGrant { pub root: PathBuf, pub channel: ChannelId, pub branch: Option<String> }`. `branch` is `None` for a detached HEAD.
  - `pub enum ReadOnlyReason { NotProjectChannel, NoWorktree, WorktreeMissing, MainCheckout, AgentNotOptedIn, PolicyDenyAll, SandboxUnavailable(String) }` with `pub fn note(&self) -> String`.
  - `pub fn eligibility(store: &Store, channel: &ChannelId, agent: &AgentConfig) -> Result<WorkspaceGrant, ReadOnlyReason>`.
  - `pub fn workspace_prompt(grant: &WorkspaceGrant) -> String`.
  - `pub fn repository_instructions(root: &Path) -> Option<String>`.
  - `pub enum SessionProfile { ReadOnly { reason: Option<ReadOnlyReason> }, Workspace { root: PathBuf } }` and `Update::SessionProfile(SessionProfile)`.

- [ ] **Step 1: Write failing unit tests in `workspace_session.rs`.** Use temporary Git repositories and a test `Store`:
  - `general_channel_is_read_only` → `Err(NotProjectChannel)` for a direct chat, `#general`, and a `legacy` channel. Only `branch` and detached-worktree channel kinds are eligible. A branch channel with no binding gives `Err(NoWorktree)`.
  - `bound_linked_worktree_is_eligible` → `Ok(grant)`, with `grant.root` equal to the canonical worktree path and `branch == Some("feature")`.
  - `borrowed_worktree_is_eligible` → a worktree with `owned=false` gives `Ok`.
  - `main_checkout_binding_is_read_only` → a binding whose path is the main checkout gives `Err(MainCheckout)`.
  - `missing_directory_is_read_only` → `Err(WorktreeMissing)`.
  - `agent_without_opt_in_is_read_only` → `Err(AgentNotOptedIn)`. A `DenyAll` agent with the opt-in gives `Err(PolicyDenyAll)`.
  - `detached_worktree_grant_has_no_branch` → `branch == None`.
  - `repository_instructions_reads_root_files_with_limit`: `AGENTS.md` of 40 KiB gives 32 KiB of text plus the notice "File truncated at 32 KiB.", labelled with `<repository-instructions file="AGENTS.md">`. No files gives `None`.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core workspace_session`. Expected: compile error.
- [ ] **Step 3: Implement `workspace_session.rs`.** Detect the main checkout the way `WorktreeAccess::inspect_bound` does: `git rev-parse --absolute-git-dir` equals `--git-common-dir`. Use these exact texts:
  - `ReadOnlyReason::note()` returns one sentence that starts "File edits are unavailable in this response:" and ends with the reason. Reasons:
    - "this is not a project branch channel"
    - "this channel has no worktree"
    - "the bound worktree folder is missing"
    - "agents do not write in the main checkout"
    - "this agent is not allowed to edit files"
    - "this agent's policy denies tool use"
    - "the provider sandbox is unavailable (<detail>)"
  - `workspace_prompt` states every item of spec decision 28. It names `grant.root`, and it states that a detached HEAD blocks commits when `branch` is `None`.
- [ ] **Step 4: Write a failing engine test** in `crates/discard-providers/tests/engine.rs`: `eligible_branch_channel_starts_a_workspace_session`. It uses the fake provider. It asserts that the recorded `SessionSpec` has `mode == SessionMode::Workspace`, `cwd == grant.root`, and `resume_handle == None`, and that the system prompt contains the workspace prompt. Add `ineligible_channel_states_the_reason`: the spec mode is `Conversation`, and the prompt contains the `note()` text.
- [ ] **Step 5: Implement the engine changes.**
  - Write sessions exist only when the process mode (`Engine.mode`) is `Conversation`. The `--builder` `Coding` path never computes eligibility.
  - In `snapshot()`, compute `eligibility` for the recipient agent and store it in a new field `Engine.write_access: Result<WorkspaceGrant, ReadOnlyReason>`. Its default is `Err(ReadOnlyReason::NotProjectChannel)`, and both types derive `Clone`.
  - In `turn_snapshot`, use `SessionMode::Workspace` and `cwd = grant.root` when a grant exists. Replace each `self.mode == SessionMode::Conversation` check that guards replay, context, or `ResponseCompleted` with `session_mode.replays_transcript()`.
  - Append `workspace_prompt` and `repository_instructions` to the system prompt for write sessions. Append the reason `note()` once to every read-only response. The exception is a direct chat whose agent has `workspace_write` off.
  - Send `Update::SessionProfile` after `Update::Responding`.
  - If `start_session` returns `ProviderError::SandboxUnavailable(detail)`, start again with `SessionMode::Conversation`, the read-only reason `SandboxUnavailable(detail)`, and `Update::Notice` with the same sentence.
- [ ] **Step 6: Run the tests.** Command: `cargo test -p discard-core workspace_session && cargo test -p discard-providers --test engine`. Expected: PASS.

---

### Task 4: Approval kinds, automatic edit approval, and Always-allow limits

**Files:**
- Modify: `crates/discard-core/src/approval.rs` (`ApprovalKind`, `Approval.kind`, decision rules)
- Modify: `crates/discard-core/src/event.rs` (`ApprovalRequest` fields)
- Modify: `crates/discard-core/src/store.rs` (migration 8: `approvals.kind`)
- Modify: `crates/discard-core/src/timeline.rs` (`ApprovalPrompt.kind`)
- Modify: `crates/discard-core/src/engine.rs` (automatic approval, refusing Always allow)
- Modify: `crates/discard-core/src/workspace_session.rs` (`paths_inside`)
- Modify: `crates/discard-app/src/desktop/dto.rs`, `crates/discard-app/src/desktop/core.rs:454` (`PromptDto`)
- Modify: `web/src/bridge/types.ts`, `web/src/components/ApprovalCard.vue`, plus a new test `web/src/components/ApprovalCard.test.ts`

**Interfaces:**
- Produces:
  - `#[serde(rename_all = "snake_case")] pub enum ApprovalKind { #[default] ProviderTool, SandboxEscape, NetworkDomain, MainCheckoutWrite, WorktreeCreate, ConversationMove, BranchCreate }`.
  - `EventKind::ApprovalRequest { approval_id, call_id, tool, input, #[serde(default)] kind: ApprovalKind, #[serde(default)] paths: Vec<String> }`.
  - `DecisionError::NotPermitted(&'static str)`.
  - `pub fn paths_inside(root: &Path, paths: &[String]) -> bool`.
  - `pub fn is_edit_tool(tool: &str) -> bool` (Claude `Edit`, `Write`, `NotebookEdit`; the Codex file-change tool name from Task 1).
  - `PromptDto { approval_id, tool, summary, view, kind: String, always_allow: String }`. `always_allow` is one of `"available"`, `"planned"`, or `"unavailable"`.

- [ ] **Step 1: Write failing tests.**

```rust
#[test]
fn plugins_decide_only_provider_tool_approvals() {
    let mut escape = Approval::new(turn(), "Bash".into(), json!({})); escape.kind = ApprovalKind::SandboxEscape;
    assert!(matches!(escape.decide(Decision::Approve, Decider::Plugin("p".into()), "ask"), Err(DecisionError::NotPermitted(_))));
    let mut tool = Approval::new(turn(), "Bash".into(), json!({}));
    assert!(tool.decide(Decision::Approve, Decider::Plugin("p".into()), "ask").is_ok());
}

#[test]
fn always_allow_is_refused_for_escapes_and_app_grants() {
    for kind in [ApprovalKind::SandboxEscape, ApprovalKind::MainCheckoutWrite, ApprovalKind::WorktreeCreate, ApprovalKind::ConversationMove, ApprovalKind::BranchCreate] {
        let mut a = Approval::new(turn(), "x".into(), json!({})); a.kind = kind;
        assert!(matches!(a.decide(Decision::AlwaysAllow, Decider::User, "ask"), Err(DecisionError::NotPermitted(_))));
    }
}
```

  In `workspace_session.rs`:
  - `paths_inside_accepts_relative_and_absolute_inner_paths`
  - `paths_inside_rejects_parent_traversal` (`"../x"`, `"a/../../x"`)
  - `paths_inside_rejects_outer_absolute_paths`
  - `paths_inside_rejects_symlink_escape`: a symlink `link → /tmp/outside`, and the path `link/new.txt`, which does not exist yet.
  - `paths_inside_rejects_empty_list`

  Engine tests in `crates/discard-providers/tests/engine.rs`:
  - `in_root_edit_is_auto_approved_in_workspace_sessions`: the audit row has decider `Policy("workspace_edit")`.
  - `out_of_root_edit_waits_for_the_user`
  - `always_allow_in_a_workspace_session_is_refused_and_not_forwarded`
  - `existing_approval_rows_read_as_provider_tool_after_migration`
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core approval workspace_session && cargo test -p discard-providers --test engine`. Expected: FAIL.
- [ ] **Step 3: Implement the core changes.**
  - `paths_inside`: resolve each path against `root`. Canonicalize the longest existing ancestor, then require that the result starts with the canonical root. Reject any component that is `..`. An empty list gives `false`.
  - Migration 8 adds `kind TEXT NOT NULL DEFAULT 'provider_tool'` to `approvals`. `save_approval` and the loaders read and write it.
  - Engine, `Workspace` sessions only: auto-approve when `is_edit_tool(tool) && kind == ProviderTool && paths_inside(root, &paths)`, the same way memory tools are approved today. A user `AlwaysAllow` sends `Update::Error("Always allow is not available in write sessions yet. Approve this request once.")`, and the decision is not forwarded.
- [ ] **Step 4: Implement the card.** `core.rs` builds `always_allow` like this:
  - `"unavailable"` when the kind is not `ProviderTool` or `NetworkDomain`
  - `"planned"` when the current `SessionProfile` is `Workspace`
  - `"available"` otherwise

  In `ApprovalCard.vue`:
  - `provider_tool`: title "{tool} wants to run".
  - `sandbox_escape`: title "Runs outside the sandbox: full access as you", plus the line "This command can read and change any file you can, and use the network."
  - `network_domain`: title "Allow network access to {summary}".
  - `"planned"`: render a disabled "Always allow" button with a `Soon` badge, and put `<!-- TODO: Durable Always-allow rules for write sessions (slice B). -->` beside it.
  - `"unavailable"`: render no Always allow button.

  In `ApprovalCard.test.ts`, cover all three `always_allow` values and the escape title.
- [ ] **Step 5: Run the tests.** Command: `cargo test -p discard-core && cargo test -p discard-providers --test engine && pnpm --dir web test -- ApprovalCard`. Expected: PASS.

---

### Task 5: Claude adapter — workspace profile

**Files:**
- Modify: `crates/discard-providers/src/claude/provider.rs` (`build_args`, `start_session`, tests)
- Modify: `crates/discard-providers/src/conversation.rs` (`WORKSPACE_TOOLS`, `check_claude_managed_files`)
- Test: `crates/discard-providers/tests/claude_process.rs` (with `fixtures/fake-claude.sh`)

**Interfaces:**
- Consumes: `SessionMode::Workspace`, `ProviderError::SandboxUnavailable` (Task 3), spike results (Task 1).
- Produces: `pub(crate) const WORKSPACE_TOOLS: &[&str] = &["worktree_commit"];`

- [ ] **Step 1: Write failing tests in `claude/provider.rs`.**

```rust
#[test]
fn workspace_args_enable_only_the_write_tools_and_the_sandbox() {
    let args = build_args(&workspace_spec());
    assert_eq!(value_after(&args, "--tools"), "Read,Glob,Grep,Edit,Write,Bash");
    assert!(value_after(&args, "--allowedTools").contains("mcp__discard-memory__worktree_commit"));
    let settings: serde_json::Value = serde_json::from_str(&value_after(&args, "--settings")).unwrap();
    assert_eq!(settings["sandbox"], serde_json::json!({"enabled":true,"allowUnsandboxedCommands":false,"autoAllowBashIfSandboxed":false}));
    assert_eq!(settings["disableAllHooks"], true);
    assert_eq!(settings["env"]["CLAUDE_CODE_DISABLE_CLAUDE_MDS"], "1");
    assert_eq!(settings["env"]["GIT_OPTIONAL_LOCKS"], "0");
    assert_eq!(value_after(&args, "--permission-mode"), "manual");
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert!(!args.iter().any(|a| a == "acceptEdits" || a == "--resume"));
}

#[test]
fn workspace_mode_requires_host_prompts() { /* ProviderDenies + Workspace → Err(ProviderError::SandboxUnavailable(_)) from start_session */ }
```

- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-providers claude::provider`. Expected: FAIL.
- [ ] **Step 3: Implement the profile.** `Workspace` uses the Conversation flags with these changes:
  - `--tools` is set to the list above.
  - `--allowedTools` is `MEMORY_TOOLS` plus `WORKSPACE_TOOLS`.
  - Settings add the `sandbox` object, and `GIT_OPTIONAL_LOCKS=0` goes in both settings `env` and the process environment.
  - Keep or drop `--restricted` as Task 1 found.
  - `cwd = spec.cwd`: no `ConversationFiles`. Require an absolute, existing directory.

  `start_session` keeps the version, help-flag, and managed-file checks. Return `SandboxUnavailable` in these cases:
  - the OS is not macOS
  - the enforcement is not `HostPrompts`
  - managed settings set `sandbox.enabled` to false or `sandbox.allowUnsandboxedCommands` to true. Extend `check_claude_managed_files` to read those keys.
- [ ] **Step 4: Add a process test** `workspace_session_runs_in_the_granted_directory` in `claude_process.rs`. It asserts the child cwd and environment through `fake-claude.sh`.
- [ ] **Step 5: Parse edit paths and the approval kind.** In `claude/parse.rs`, fill `paths` from `file_path` (`Edit`, `Write`) and `notebook_path`. Set `kind` as Task 1 recorded. If network-domain requests arrive and can be identified, classify them `NetworkDomain`. Use `ApprovalKind` and the `ApprovalRequest` fields from Task 4. Add a parser test against `fixtures/claude-workspace-turn.jsonl`.
- [ ] **Step 6: Update `docs/provider-claude.md`.** Add a "Workspace sessions" section with the flags and the fail-closed rules.
- [ ] **Step 7: Run the tests.** Command: `cargo test -p discard-providers --test claude_process && cargo test -p discard-providers claude`. Expected: PASS.

---

### Task 6: Codex adapter — workspace profile

**Files:**
- Modify: `crates/discard-providers/src/codex/provider.rs` (`thread_params`, `start_session`, tests)
- Modify: `crates/discard-providers/src/conversation.rs` (`sanitize_catalog`, `codex_controls`)
- Modify: `crates/discard-providers/src/codex/parse.rs` (kind and paths)
- Test: `crates/discard-providers/tests/codex_process.rs` (with `fixtures/fake-codex.py`)

**Interfaces:**
- Consumes: Task 1 results, Task 3 types, `ApprovalKind` and the `ApprovalRequest` fields (Task 4).
- Produces: `pub(crate) enum CatalogTools { None, Workspace }`, `sanitize_catalog(cache: Value, selected: &str, tools: CatalogTools)`, and `codex_controls(effective: &Value, tools: CatalogTools)`.

- [ ] **Step 1: Write failing tests.**
  - `workspace_thread_uses_untrusted_and_workspace_write`: `thread_params(&workspace_spec())` → `approvalPolicy == "untrusted"`, `sandbox == "workspace-write"`, `ephemeral == true`, `allowProviderModelFallback == false`, `cwd == spec.cwd`.
  - `workspace_catalog_keeps_shell_and_patch`: `sanitize_catalog(cache, "m", CatalogTools::Workspace)` keeps the cached `shell_type` and `apply_patch_tool_type`. It still sets `experimental_supported_tools == []`, `multi_agent_version == "disabled"`, and `node_repl_disabled == true`.
  - `workspace_controls_keep_sandbox_and_disable_network`: `codex_controls(&json!({}), CatalogTools::Workspace)` sets `features.shell_tool` to the value Task 1 found working. It sets `sandbox_workspace_write.network_access == false`, `sandbox_workspace_write.writable_roots == []`, `project_doc_max_bytes == 0`, and `web_search == "disabled"`, and it keeps every other Conversation feature off.
  - `conversation_profile_is_unchanged`: the existing Conversation catalog and controls tests pass with `CatalogTools::None`.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-providers codex conversation`. Expected: FAIL.
- [ ] **Step 3: Implement the profile.**
  - `Workspace` with `HostPrompts` → `("untrusted", "workspace-write")`. `Workspace` with any other enforcement → `ProviderError::SandboxUnavailable`.
  - `cwd = spec.cwd`, with no temporary cwd.
  - The scoped MCP `enabled_tools` is `MEMORY_TOOLS` plus `WORKSPACE_TOOLS`.
  - The handshake `validate_controls` also checks the sandbox keys. A mismatch returns `SandboxUnavailable`.
- [ ] **Step 4: Classify Codex approvals in `codex/parse.rs`.** The session passes its mode to the parser.
  - `fileChange` requests fill `paths` from the change map.
  - Escalation requests are marked as Task 1 recorded, and they get `kind = SandboxEscape`.
  - **Fail-closed fallback:** if Task 1 found no reliable marker, every `commandExecution` approval in a `Workspace` session gets `kind = SandboxEscape`.

  Add parser tests against `fixtures/codex-workspace-turn.jsonl`.
- [ ] **Step 5: Add a process test** `workspace_thread_starts_in_the_granted_directory` in `codex_process.rs`.
- [ ] **Step 6: Update `docs/provider-codex.md`.** Add a "Workspace sessions" section.
- [ ] **Step 7: Run the tests.** Command: `cargo test -p discard-providers --test codex_process && cargo test -p discard-providers codex`. Expected: PASS.

---

### Task 7: `worktree_commit`, `agent_commits`, and the worktree write lock

**Files:**
- Create: `crates/discard-core/src/workspace_commit.rs`
- Create: `crates/discard-core/src/workspace_lock.rs`
- Modify: `crates/discard-core/src/worktree_access.rs` (`with_locks`, `commit`)
- Modify: `crates/discard-core/src/store.rs` (migration 8: `agent_commits`)
- Modify: `crates/discard-core/src/engine.rs` (`MemoryService::prepare_workspace_turn`, `Engine.workspace_locks`, acquiring and holding the lock)
- Modify: `crates/discard-mcp/src/identity.rs` (`Identity.workspace_root`), `crates/discard-mcp/src/tools.rs`, `crates/discard-mcp/src/service.rs`
- Modify: `crates/discard-app/src/main.rs` and the desktop wiring that builds `Engine`, `WorktreeAccess`, and `MemoryRuntime` (one shared `Arc<WorkspaceLocks>`)

**Interfaces:**
- Produces:
  - `pub struct CommitRequest { pub message: String, pub paths: Option<Vec<String>> }`
  - `pub struct CommitResult { pub commit: String, pub branch: String, pub files: Vec<String> }`
  - `pub fn commit(root: &Path, request: &CommitRequest) -> Result<CommitResult, String>`
  - `pub struct WorkspaceLocks`, with:
    - `pub async fn acquire(&self, root: &Path, holder: ChannelId) -> WorkspaceLockGuard` (the guard releases on drop)
    - `pub fn holder(&self, root: &Path) -> Option<ChannelId>`
  - `WorktreeAccess::with_locks(self, locks: Arc<WorkspaceLocks>) -> Self`
  - `WorktreeAccess::commit(&self, agent: &AgentId, channel: &ChannelId, root: &Path, request: &CommitRequest) -> Result<CommitResult, String>`
  - `Store::record_agent_commit(&self, commit: &str, repo: &str, channel: &ChannelId, agent: &AgentId) -> Result<()>`
  - `Store::link_agent_commits(&self, channel: &ChannelId, response: &MessageId) -> Result<usize>` (fills rows whose response is NULL)
  - `MemoryService::prepare_workspace_turn(&self, agent, project, source, base_prompt, grant: &WorkspaceGrant, query: &str) -> Result<PreparedMemory, String>`. The default implementation returns `Err("Write sessions require scoped workspace tools.")`.
  - `Identity.workspace_root: Option<PathBuf>`

- [ ] **Step 1: Write failing tests in `workspace_commit.rs`.** Use temporary Git repositories:
  - `commits_all_changes_with_the_users_identity`: the author is the repo's `user.name` and `user.email`, and the message has no trailer.
  - `commits_only_listed_paths`
  - `refuses_detached_head`: the error is "Create a branch before committing. This worktree has a detached HEAD."
  - `refuses_empty_and_oversized_messages`: an empty message or one over 4096 bytes fails.
  - `refuses_paths_outside_the_root`
  - `nothing_to_commit_is_an_error`
  - `does_not_run_hooks`: set `core.hooksPath=.husky` and a tracked `.husky/pre-commit` that writes a marker file, then assert that the marker does not exist.
- [ ] **Step 2: Write failing tests in `workspace_lock.rs`.**
  - `second_acquire_waits_until_the_guard_drops`
  - `holder_reports_the_owning_channel`
  - `dropping_a_waiting_future_does_not_leak_the_lock`
- [ ] **Step 3: Run them to see them fail.** Command: `cargo test -p discard-core workspace_commit workspace_lock`. Expected: compile error.
- [ ] **Step 4: Implement both modules.** `commit` runs `git add -A -- <paths or .>` and then `git commit -m <message>` through the same hardened runner as `worktree_access::git`, with a 30-second limit for each command. It fails if the root is the main checkout. `WorktreeAccess::commit` requires `locks.holder(root) == Some(channel)`. On success it records `agent_commits`.
- [ ] **Step 5: Wire the MCP tool.** `worktree_commit` is listed only when `identity.workspace_root` is `Some`. Input schema: `{"message": string (required), "paths": string[] (optional)}`, with no other keys. Description: "Commit changes in this conversation's worktree. Discard stages and commits outside the sandbox with hooks off and the user's Git identity. Refuses on a detached HEAD." `MemoryRuntime::prepare_workspace_turn` issues the identity with `workspace_root = Some(grant.root)`.
- [ ] **Step 6: Wire the engine.**
  - Write sessions call `prepare_workspace_turn`.
  - On the `SandboxUnavailable` fallback from Task 3, drop the lock guard, revoke the workspace token, and prepare memory again with `prepare_turn`. The read-only session must not list `worktree_commit`. Add the engine test `sandbox_fallback_drops_the_lock_and_the_commit_tool`.
  - Before `start_session`, they acquire the lock. While the lock is held by another channel, send `Update::Notice("Waiting for #<holder channel name> to finish using this worktree.")` and wait for either the lock or `Command::Cancel`. A cancel ends the response as `Cancelled`.
  - Hold the guard until the changes card (Task 8) is saved.
  - After the response is stored, call `link_agent_commits`.
- [ ] **Step 7: Write the engine tests** in `crates/discard-providers/tests/engine.rs`:
  - `lock_is_released_after_cancel_provider_crash_and_setup_failure`
  - `second_channel_on_the_same_path_waits_and_then_runs`
  - `commits_made_during_a_response_link_to_that_response`
- [ ] **Step 8: Run the tests.** Command: `cargo test -p discard-core && cargo test -p discard-mcp && cargo test -p discard-providers --test engine`. Expected: PASS.

---

### Task 8: Changes card

**Files:**
- Create: `crates/discard-core/src/workspace_changes.rs`
- Modify: `crates/discard-core/src/model.rs` (`Block::Changes`)
- Modify: `crates/discard-core/src/engine.rs` (snapshot before the session, card after it)
- Create: `web/src/components/ChangesCard.vue`, `web/src/components/ChangesCard.test.ts`
- Modify: `web/src/components/BlockView.vue`, `web/src/bridge/types.ts`, `web/src/components/protected.test.ts`

**Interfaces:**
- Produces:
  - `pub struct StartSnapshot { pub head: Option<String>, pub dirty: BTreeMap<String, Option<String>> }`. Each value is the content hash, or `None` when the file was deleted.
  - `pub fn snapshot(root: &Path) -> Result<StartSnapshot, String>`
  - `#[serde(rename_all = "snake_case")] pub enum ChangesState { Completed, Stopped, Failed }`
  - `pub fn changes(root: &Path, start: &StartSnapshot, state: ChangesState) -> Result<Block, String>`
  - `Block::Changes { root: String, state: ChangesState, commits: Vec<ChangedCommit>, files: Vec<ChangedFile>, truncated: bool }`, where:
    - `ChangedCommit { id: String, subject: String }`
    - `ChangedFile { path: String, status: String, diff: String, truncated: bool, preexisting: bool }`

- [ ] **Step 1: Write failing tests in `workspace_changes.rs`:**
  - `reports_new_commits_and_uncommitted_edits`
  - `ignores_files_dirty_before_the_response_when_unchanged`: the file was dirty at the snapshot, has the same content at the end, and is absent from the card.
  - `marks_preexisting_dirty_files_that_changed`: `preexisting == true`.
  - `limits_diff_size`: a diff over 16 KiB per file is cut and `truncated == true`. The total is limited to 128 KiB and 200 files, and the card's `truncated == true`.
  - `reports_untracked_new_files`
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core workspace_changes`. Expected: compile error.
- [ ] **Step 3: Implement the module.** Use the hardened Git runner with `--no-optional-locks`.
  - **Changed files:** names from `git diff --name-status <start.head>..HEAD`, plus current dirty paths whose content hash differs from `start.dirty`, plus paths that were dirty at the start and are now different from that content.
  - **Per-file diff:** `git diff <start.head> -- <path>`. Untracked files use `--no-index /dev/null <path>`.
  - **Commits:** `git log --format=%H%x00%s <start.head>..HEAD`.
- [ ] **Step 4: Wire the engine.** In write sessions, call `snapshot` after the lock is acquired and before `start_session`. After the driver ends, call `changes` with:
  - `Completed` when the outcome is completed
  - `Stopped` when it is cancelled
  - `Failed` otherwise

  Append the block to the response blocks before `store_message`, and store the response even when there is no other block. If building the card fails, append a Markdown block "Discard could not list this response's changes: <error>".
- [ ] **Step 5: Build `ChangesCard.vue`.**
  - Use `data-ui-id="changes-card"` and `data-protected="true"`, with the protected tokens that `ApprovalCard.vue` uses.
  - Header: "Changes · N files · M commits". Add a "Stopped" or "Failed" badge for those states.
  - Commit list: short ID and subject.
  - File list: status and path, each expandable to a `DiffBlock`.
  - A `preexisting` file shows "Had uncommitted changes before this response."
  - When the card is truncated, show "Some changes are not shown."
  - Add the card to `protected.test.ts`.
  - Test the header text, the state badge, the expand action, and the truncation text.
- [ ] **Step 6: Run the tests.** Command: `cargo test -p discard-core && cargo test -p discard-providers --test engine && pnpm --dir web test -- ChangesCard protected`. Expected: PASS.

---

### Task 9: Replay compaction

**Files:**
- Modify: `crates/discard-core/src/conversation_context.rs`
- Modify: `docs/runtime/conversation-context.md`

**Interfaces:**
- Produces: `pub fn compact_blocks(blocks: &[Block]) -> serde_json::Value`, which `replay_with_authors` uses in place of `serde_json::to_string(&m.blocks)`.

- [ ] **Step 1: Write failing tests.**
  - `tool_call_replays_name_target_status_and_first_kib`: a `ToolCall` with `input = {"file_path":"a.rs","old_string":<9 KiB>,"new_string":<9 KiB>}` and a 10 KiB output replays as `name`, `input == {"file_path":"a.rs"}`, and `is_error`. The output is the first 1024 bytes on a UTF-8 boundary plus `"…[truncated 9216 bytes]"`. The input keeps only `command`, `cmd`, `file_path`, `path`, `paths`, `notebook_path`, and `pattern`.
  - `changes_card_replays_files_and_commit_ids`: the replay is `{"type":"changes","state":…,"files":[{"path","status"}],"commits":["<id>"]}`.
  - `other_blocks_replay_unchanged`
  - `saved_boundary_survives_fewer_fragments`: the saved `through` is `<id>:part:5`, and after compaction that message has 2 parts. `after_boundary` returns only messages after `<id>`.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core conversation_context`. Expected: FAIL.
- [ ] **Step 3: Implement the compaction.** Change `after_boundary` like this: when the exact ID is missing, find the last fragment whose message prefix (`<id>` before `:part:`) matches, and return everything after it. With no match, keep the current behavior.
- [ ] **Step 4: Update `docs/runtime/conversation-context.md`.** Describe the compacted replay and the boundary fallback.
- [ ] **Step 5: Run the tests.** Command: `cargo test -p discard-core conversation_context`. Expected: PASS.

---

### Task 10: Documentation and the full proof

**Files:**
- Modify: `docs/worktrees.md` (write sessions in channels, the lock, `worktree_commit`)
- Modify: `docs/project-channel-status.md` (implemented: slice A; remaining: slices B and C)
- Modify: `README.md` (one line under Docs for the new spec)

- [ ] **Step 1: Update the docs.** Each doc states what slice A does and what is still planned. Slice A does not claim worktree creation, the main-checkout grant, durable Always-allow rules, or moves.
- [ ] **Step 2: Run the proof command** from Global Constraints. Expected: every command exits 0.
