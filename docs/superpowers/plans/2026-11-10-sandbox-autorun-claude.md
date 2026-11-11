# Sandbox Autorun (Claude) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an opted-in Claude agent run sandboxed shell commands in a worktree write session without an approval card.

**Architecture:** A new per-agent flag `sandbox_autorun` flows from `AgentConfig` through engine eligibility into `SessionSpec`. The Claude adapter then sets `autoAllowBashIfSandboxed: true` and adds read denials for credential paths. The engine reports the autorun state in `SessionProfile::Workspace`. The app shows it in the agent editor, the context panel, tool-call blocks, and plugin slot state. Codex reports autorun as unsupported until its spike runs (spec delivery part 2, not in this plan).

**Tech Stack:** Rust (discard-core, discard-providers, discard-app, discard-lua), SQLite via rusqlite, Tauri 2, Vue 3 with shadcn-vue, Vite+ tests.

**Spec:** `docs/superpowers/specs/2026-11-10-sandbox-autorun-design.md`

## Global Constraints

- `sandbox_autorun` defaults to false. Old agent TOML reads it as false.
- Turning autorun on needs the existing user confirmation (`AgentChange::needs_user_approval`).
- Autorun applies only when `workspace_write` is on, the policy is `Ask`, the session is a write session, and the grant is not the main checkout.
- Claude autorun settings keep `sandbox.enabled: true`, `allowUnsandboxedCommands: false`, and the `.git` `denyWrite` paths unchanged.
- The managed-settings check (`crates/discard-providers/src/conversation.rs:251-254`) keeps rejecting local managed settings that set `autoAllowBashIfSandboxed`. Do not relax it.
- Sandbox escape, network domain, main-checkout write, and protected-path edit approvals never auto-approve.
- Planned controls follow `AGENTS.md`: visible, disabled, labelled `Soon`, with a `TODO` in code. Codex autorun is planned.
- Commit messages: Conventional Commits. No Claude attribution trailers or lines.
- Every task ends with `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` passing. Web tasks also run `pnpm --dir web check` and `pnpm --dir web test`.

## Review Focus

1. **Write access turned off while autorun is on.** Expect autorun to stop applying, and the saved flag to clear. Test: Task 1, `turning_off_workspace_writes_clears_autorun`.
2. **An autorun agent gets a main-checkout grant.** Expect per-command approval and the indicator "Autorun off: main-checkout writes ask for each command". Test: Task 3, `main_checkout_grant_disables_autorun`.
3. **Claude's native `Read` tool reads `~/.ssh/id_ed25519`.** The sandbox covers Bash only. Expect a permission deny rule to block it. Test: Task 2, `autorun_denies_native_reads_of_credentials`; live probe in Task 7.
4. **The session root sits under `DISCARD_HOME/worktrees`.** Expect the root to stay readable while the rest of `DISCARD_HOME` is denied. Test: Task 2, `read_denials_keep_the_session_root_readable`.
5. **Claude setup fails with `SandboxUnavailable` for an autorun response.** Expect the read-only fallback with no autorun indicator and no autorun audit row. Test: Task 4, `sandbox_fallback_records_no_autorun_audit`.

---

### Task 1: Agent setting

**Files:**
- Modify: `crates/discard-core/src/agent.rs` (struct at :8-26, `AgentChange` at :87-93, `needs_user_approval` at :106-113, tests)
- Modify: `crates/discard-core/src/agent_edit.rs` (`AgentDraft` :8-11, `review` :37-60, save function)
- Modify: every `AgentConfig { ... }` literal in the workspace. Find them with `grep -rn "workspace_write: false," crates`. Add `sandbox_autorun: false`.

**Interfaces:**
- Produces: `AgentConfig.sandbox_autorun: bool` (`#[serde(default)]`), `AgentDraft.sandbox_autorun: bool`, `AgentChange::SetSandboxAutorun { agent: AgentId, enabled: bool }`.

- [ ] **Step 1: Write the failing tests** in `agent.rs` and `agent_edit.rs`:

```rust
#[test]
fn enabling_sandbox_autorun_needs_user_approval() {
    let on = AgentChange::SetSandboxAutorun { agent: AgentId("a".into()), enabled: true };
    assert!(on.needs_user_approval(&Initiator::User));
    let off = AgentChange::SetSandboxAutorun { agent: AgentId("a".into()), enabled: false };
    assert!(!off.needs_user_approval(&Initiator::User));
    assert!(off.needs_user_approval(&Initiator::Plugin("p".into())));
}
#[test]
fn old_agent_toml_reads_sandbox_autorun_as_false() {
    assert!(!AgentConfig::from_toml("name = 'A'\nprovider = 'claude'\npolicy = 'ask'\n").unwrap().sandbox_autorun);
}
// agent_edit.rs
#[test]
fn enabling_autorun_needs_confirm_and_lists_the_change() {
    // current: workspace_write true, sandbox_autorun false; draft: sandbox_autorun true
    let r = review(&current, &draft, &matrix);
    assert!(r.needs_confirm);
    assert!(r.changes.iter().any(|c| c.field == "Sandbox autorun" && c.after == "on"));
}
#[test]
fn enabling_autorun_without_workspace_writes_is_blocked() {
    // draft: workspace_write false, sandbox_autorun true
    assert_eq!(review(&current, &draft, &matrix).blocked.as_deref(), Some("Turn on file edits before autorun."));
}
#[test]
fn turning_off_workspace_writes_clears_autorun() {
    // saved: workspace_write true, sandbox_autorun true; draft: workspace_write false, sandbox_autorun true
    // after save: load_agent(..).sandbox_autorun == false
}
```

- [ ] **Step 2: Run** `cargo test -p discard-core agent` — expect compile failure on the missing field and variant.

- [ ] **Step 3: Implement.** Add the field, the draft field (and `From<&AgentConfig>`), the variant (enable needs user approval, same arm as `SetWorkspaceWrite`), the `"Sandbox autorun"` review row, the `needs_confirm` term, and the blocked message. The blocked check applies only when `draft.sandbox_autorun && !draft.workspace_write && !current.sandbox_autorun` (enabling). In the save path, write `sandbox_autorun = draft.sandbox_autorun && draft.workspace_write`.

- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS. Fix all `AgentConfig` literals the compiler reports.

- [ ] **Step 5: Commit** `feat: add the sandbox autorun agent setting`

---

### Task 2: Provider capability and Claude adapter settings

**Files:**
- Modify: `crates/discard-core/src/provider.rs` (`SessionSpec` :50-61, `Provider` trait :78-100)
- Modify: `crates/discard-providers/src/claude/provider.rs` (settings block :93-104, tests near :464)
- Modify: `crates/discard-providers/src/codex/provider.rs` (trait impl; test)
- Create: `crates/discard-core/src/sandbox_reads.rs` (register in `lib.rs`)
- Modify: every `SessionSpec { ... }` literal. Add `sandbox_autorun: false`.

**Interfaces:**
- Produces:
  - `pub enum SandboxAutorun { Supported, Unsupported(String) }` in `provider.rs`.
  - `Provider::sandbox_autorun(&self) -> SandboxAutorun`, default `Unsupported("this provider has not been verified for autorun".into())`.
  - `SessionSpec.sandbox_autorun: bool`.
  - `sandbox_reads::denied_read_paths(home: &Path, discard_home: &Path, root: &Path) -> Vec<PathBuf>`.

- [ ] **Step 1: Write the failing tests.**

```rust
// sandbox_reads.rs
#[test]
fn denied_reads_cover_credential_paths() {
    let paths = denied_read_paths(home, discard_home, root);
    for p in [".ssh", ".aws", ".config/gh", ".gnupg", ".netrc", ".docker/config.json", ".claude", ".claude.json", ".codex"] {
        assert!(paths.contains(&home.join(p)), "{p}");
    }
}
#[test]
fn read_denials_keep_the_session_root_readable() {
    // discard_home contains discard.db, worktrees/<id>/ (the root), and other/; root = discard_home/worktrees/<id>
    let paths = denied_read_paths(home, discard_home, root);
    assert!(paths.contains(&discard_home.join("discard.db")));
    assert!(paths.contains(&discard_home.join("other")));
    assert!(!paths.iter().any(|p| root.starts_with(p)));
}
// claude/provider.rs
#[test]
fn autorun_session_auto_allows_sandboxed_bash() {
    // Workspace spec with sandbox_autorun: true
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], true);
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert!(settings["sandbox"]["filesystem"]["denyWrite"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().ends_with(".git")));
    assert!(!settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap().is_empty());
}
#[test]
fn autorun_denies_native_reads_of_credentials() {
    let deny = settings["permissions"]["deny"].as_array().unwrap();
    assert!(deny.iter().any(|r| r.as_str().unwrap().starts_with("Read(") && r.as_str().unwrap().contains(".ssh")));
}
#[test]
fn workspace_without_autorun_keeps_bash_prompts() {
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
    assert!(settings["sandbox"]["filesystem"].get("denyRead").is_none());
}
#[test]
fn claude_supports_sandbox_autorun() { assert_eq!(provider.sandbox_autorun(), SandboxAutorun::Supported); }
// codex/provider.rs
#[test]
fn codex_reports_autorun_unsupported() { assert!(matches!(provider.sandbox_autorun(), SandboxAutorun::Unsupported(_))); }
```

- [ ] **Step 2: Run** `cargo test -p discard-core sandbox_reads` and `cargo test -p discard-providers claude::provider` — expect failures.

- [ ] **Step 3: Implement.**
  - `denied_read_paths`: the home entries listed in the test, plus each direct child of `discard_home` that is not an ancestor of `root`. Do not check existence; the sandbox accepts missing paths. *(If a live run shows Claude rejects missing paths, filter to existing ones.)*
  - Claude adapter, only when `spec.mode == Workspace && spec.sandbox_autorun`: set `autoAllowBashIfSandboxed: true`, `sandbox.filesystem.denyRead` to the paths, and `permissions.deny` to `Read(<path>/**)` for directories and `Read(<path>)` for files, for each path. Use `std::env::home_dir()` and the Discard home the adapter already receives (follow how the MCP config path reaches the adapter; add a `discard_home` field to the Claude provider constructor if none exists).
  - Claude: `sandbox_autorun()` returns `Supported`.
  - Codex: implement `sandbox_autorun()` returning `Unsupported("Codex autorun needs a sandbox check first.".into())`, with `// TODO: Run the Codex on-request sandbox spike (spec decision 12) and support autorun if it passes.` If `start_session` receives `sandbox_autorun: true`, return `ProviderError::SandboxUnavailable("Codex does not support autorun")`.

- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS.

- [ ] **Step 5: Commit** `feat: configure Claude sandbox autorun with credential read denials`

---

### Task 3: Engine eligibility, profile, and prompt

**Files:**
- Modify: `crates/discard-core/src/workspace_session.rs` (`SessionProfile` :30, `workspace_prompt` :75-80, tests)
- Modify: `crates/discard-core/src/engine.rs` (snapshot build :195-218, `session_profile` :388-398, system prompt :525-528, `SessionSpec` build :555-562)
- Test: `crates/discard-providers/tests/engine.rs` (fake-provider engine tests)

**Interfaces:**
- Consumes: `AgentConfig.sandbox_autorun`, `Provider::sandbox_autorun()`, `SessionSpec.sandbox_autorun`.
- Produces:
  - `pub enum AutorunState { Off, On, Unavailable(String) }` in `workspace_session.rs`. `Off` means the agent did not opt in.
  - `SessionProfile::Workspace { root: PathBuf, autorun: AutorunState }`.
  - `pub fn autorun_state(agent_opted_in: bool, grant: &WorkspaceGrant, provider: SandboxAutorun) -> AutorunState`.
  - `workspace_prompt(grant: &WorkspaceGrant, autorun: &AutorunState) -> String`.

- [ ] **Step 1: Write the failing tests.**

```rust
// workspace_session.rs
#[test]
fn autorun_is_on_for_an_opted_in_linked_worktree() {
    assert_eq!(autorun_state(true, &linked_grant, SandboxAutorun::Supported), AutorunState::On);
}
#[test]
fn main_checkout_grant_disables_autorun() {
    assert_eq!(autorun_state(true, &main_grant, SandboxAutorun::Supported),
        AutorunState::Unavailable("main-checkout writes ask for each command".into()));
}
#[test]
fn unsupported_provider_reports_its_reason() {
    assert_eq!(autorun_state(true, &linked_grant, SandboxAutorun::Unsupported("x".into())), AutorunState::Unavailable("x".into()));
}
#[test]
fn agent_without_autorun_is_off() {
    assert_eq!(autorun_state(false, &linked_grant, SandboxAutorun::Supported), AutorunState::Off);
}
#[test]
fn autorun_prompt_says_commands_run_without_asking() {
    let p = workspace_prompt(&linked_grant, &AutorunState::On);
    assert!(p.contains("Shell commands inside this directory run without asking."));
    assert!(p.contains("Run the project's checks yourself before you report the work done."));
    assert!(!p.contains("Shell commands ask for approval."));
}
// tests/engine.rs (fake provider records the SessionSpec it receives)
#[test] fn autorun_agent_starts_session_with_autorun_flag() { /* spec.sandbox_autorun == true; profile update is Workspace { autorun: On } */ }
#[test] fn autorun_unavailable_starts_session_without_flag() { /* fake provider returns Unsupported; spec.sandbox_autorun == false; notice states the reason once */ }
```

- [ ] **Step 2: Run** `cargo test -p discard-core workspace_session` and `cargo test -p discard-providers --test engine autorun` — expect failures.

- [ ] **Step 3: Implement.** Store `AutorunState` on the turn snapshot next to `write_access`, computed after `verify_binding`. `session_profile()` returns it inside `Workspace`. Set `spec.sandbox_autorun = matches!(state, AutorunState::On)`. For `Unavailable(reason)`, append `" Autorun is off for this response: {reason}."` to the prompt. Prompt text for `On` replaces "Shell commands ask for approval." with "Shell commands inside this directory run without asking. Network access and writes outside it still need approval. Run the project's checks yourself before you report the work done."

- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS. Update existing `SessionProfile::Workspace { root }` matches.

- [ ] **Step 5: Commit** `feat: choose sandbox autorun for each write response`

---

### Task 4: Audit row and "ran in sandbox" marking

**Files:**
- Modify: `crates/discard-core/src/store.rs` (migrations after :427, new insert function, tests)
- Modify: `crates/discard-core/src/model.rs` (`Block::ToolCall` :148-155)
- Modify: `crates/discard-core/src/timeline.rs` (tool call push :76, result :86, tests)
- Modify: `crates/discard-core/src/engine.rs` (response save near :765)
- Modify: every `Block::ToolCall { ... }` literal. Add `sandboxed: None`.

**Interfaces:**
- Consumes: `AutorunState::On` from Task 3.
- Produces:
  - Schema version 10: `CREATE TABLE autorun_responses (response_id TEXT PRIMARY KEY REFERENCES messages(id), channel_id TEXT NOT NULL REFERENCES channels(id), agent_id TEXT NOT NULL REFERENCES agents(id), provider TEXT NOT NULL, root TEXT NOT NULL, created_at INTEGER NOT NULL)`.
  - `Store::record_autorun_response(&self, response: &MessageId, channel: &ChannelId, agent: &AgentId, provider: &str, root: &Path) -> Result<()>`.
  - `Block::ToolCall.sandboxed: Option<bool>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`).
  - `Timeline::set_autorun(&mut self, on: bool)`.

- [ ] **Step 1: Write the failing tests.**

```rust
// store.rs
#[test] fn migration_creates_autorun_responses() { /* open at user_version 9, reopen; table exists; schema_version() == 10 */ }
// timeline.rs
#[test]
fn autorun_marks_shell_calls_without_an_approval() {
    // set_autorun(true); ToolCall Bash call "c1" + result; ToolCall Bash "c2" + ApprovalRequest(call_id "c2") + result
    assert!(matches!(&t.blocks()[0], Block::ToolCall { sandboxed: Some(true), .. }));
    assert!(matches!(&t.blocks()[1], Block::ToolCall { sandboxed: None, .. }));
}
#[test]
fn edits_are_not_marked_sandboxed() { /* set_autorun(true); Edit call + result -> sandboxed: None */ }
// tests/engine.rs
#[test] fn autorun_response_records_audit_row() { /* row with the response id, agent, provider "fake", root */ }
#[test] fn sandbox_fallback_records_no_autorun_audit() { /* fake provider returns SandboxUnavailable for Workspace; no row; profile ReadOnly */ }
```

- [ ] **Step 2: Run** `cargo test -p discard-core store::tests::migration_creates_autorun_responses timeline` — expect failures.

- [ ] **Step 3: Implement.** Shell tools are `Bash` (Claude) and `command` (Codex). Mark on the tool result: `sandboxed = Some(true)` when autorun is on, the name is a shell tool, and no `ApprovalRequest` with that `call_id` arrived. The engine calls `set_autorun` after the session starts (not before the fallback), and records the audit row when it saves the response, beside `link_agent_commits`. Delete `autorun_responses` rows wherever `agent_commits` rows are deleted for a channel (store.rs:1431).

- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS.

- [ ] **Step 5: Commit** `feat: audit autorun responses and mark sandboxed shell calls`

---

### Task 5: App bridge, settings, and plugin slot state

**Files:**
- Modify: `crates/discard-app/src/desktop/settings/agents.rs` (`Draft` :72, `into_core` :80, list JSON :91, create guard :160)
- Modify: `crates/discard-app/src/desktop/core.rs` (snapshot DTO near :450)
- Modify: `crates/discard-core/src/ui_state.rs` (`slot_state`)
- Modify: `crates/discard-lua/src/api_spec.rs` (`SLOT_STATE_DOC`)

**Interfaces:**
- Consumes: `AgentDraft.sandbox_autorun`, `Provider::sandbox_autorun()`, `SessionProfile::Workspace { autorun }`.
- Produces (JSON, camelCase):
  - Agent list rows: `sandboxAutorun: bool`, `sandboxAutorunUnavailable: string | null` (the provider's `Unsupported` reason).
  - Draft input: `sandboxAutorun?: bool` (missing keeps the saved value).
  - Snapshot DTO: `autorun: { state: "on" | "unavailable", reason: string | null } | null`. `null` for `Off` and for non-Workspace profiles.
  - Plugin slot state: `autorun: boolean` (true only for `On` while a turn runs).

- [ ] **Step 1: Write the failing tests.**

```rust
// settings/agents.rs
#[test] fn agent_rows_report_autorun_support() { /* fake provider Unsupported("x") -> row["sandboxAutorunUnavailable"] == "x" */ }
#[test] fn create_rejects_autorun() { /* draft sandboxAutorun true -> Err("Create the agent first. Then review Sandbox autorun permission.") */ }
// core.rs
#[test] fn snapshot_reports_autorun_state() { /* profile Workspace{autorun: Unavailable("r")} -> dto.autorun == {state:"unavailable", reason:"r"} */ }
// ui_state.rs
#[test] fn slot_state_reports_autorun_while_running() { /* Responding + SessionProfile Workspace{On} -> slot_state["autorun"] == true; after finish == false */ }
```

- [ ] **Step 2: Run** `cargo test -p discard-app settings::agents` and `cargo test -p discard-core ui_state` — expect failures.

- [ ] **Step 3: Implement** the fields above. Add `autorun: boolean` to `SLOT_STATE_DOC`: "`state.autorun` is true while the running response runs shell commands without approval." Plugins get read access only; add no plugin action that changes the setting.

- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS.

- [ ] **Step 5: Commit** `feat: expose sandbox autorun to settings, snapshots, and plugins`

---

### Task 6: Web UI

**Files:**
- Modify: `web/src/bridge/api.ts` (agent type near :137, snapshot type)
- Modify: `web/src/components/agents/AgentProfileEditor.vue` (after the workspace-write field, :438-450)
- Modify: `web/src/components/workspace/ContextPanel.vue` (beside the agent status, :75 and :131)
- Modify: `web/src/components/ToolCallBlock.vue`
- Test: `web/src/components/agents/profiles.test.ts`, `web/src/components/workspace/ContextPanel.test.ts`, `web/src/components/ToolCallBlock.test.ts`

**Interfaces:**
- Consumes: the JSON fields from Task 5.

- [ ] **Step 1: Write the failing tests.**

```ts
it('disables autorun until file edits are on', () => { /* workspaceWrite false -> #agent-sandbox-autorun disabled, description says "Turn on file edits first." */ })
it('labels autorun Soon for providers without support', () => { /* sandboxAutorunUnavailable "x" -> switch disabled, text "Soon", reason "x" shown */ })
it('shows Autorun beside the agent status', () => { /* snapshot.autorun {state:'on'} -> text "Autorun" */ })
it('shows why autorun is off', () => { /* {state:'unavailable', reason:'r'} -> text "Autorun off: r" */ })
it('labels sandboxed tool calls', () => { /* block.sandboxed true -> text "Ran in sandbox" */ })
```

- [ ] **Step 2: Run** `pnpm --dir web test` — expect the new tests to fail.

- [ ] **Step 3: Implement.**
  - Editor switch id `agent-sandbox-autorun`, label "Run sandboxed commands without asking", description "Shell commands in the worktree run without approval. The sandbox blocks writes outside the worktree and network access. It does not block reads." Disabled with "Turn on file edits first." when `workspaceWrite` is off (state block, no `TODO`). When `sandboxAutorunUnavailable` is set: disabled, `Soon` badge, the reason, and `<!-- TODO: Enable when the provider passes the autorun sandbox check. -->`.
  - Turning the switch off in the editor when `workspaceWrite` goes off is not needed; the backend clears it on save.
  - Context panel: a small "Autorun" badge, or "Autorun off: {reason}" as muted text.
  - Tool-call block: a "Ran in sandbox" label in the header when `sandboxed` is true.

- [ ] **Step 4: Run** `pnpm --dir web check`, `pnpm --dir web test`, `pnpm --dir web build` — expect PASS. Check the editor and a running autorun response visually with `cargo tauri dev`.

- [ ] **Step 5: Commit** `feat: add sandbox autorun controls and indicators`

---

### Task 7: Live probe and docs

**Files:**
- Modify: `crates/discard-providers/tests/probes/claude_workspace.py` (add an autorun run) or `crates/discard-providers/tests/claude_live.rs` (follow where Workspace live checks live today)
- Modify: `docs/provider-claude.md` (Workspace sessions section), `docs/worktrees.md` (agent workspace writes section), `docs/project-channel-status.md` (Agent workspace writes section)

- [ ] **Step 1: Add the live autorun probe** (ignored by default). With `sandbox_autorun: true` in a temporary linked worktree, assert:
  - `touch inside.txt` runs with no `can_use_tool` request.
  - `touch .git/marker` fails with Operation not permitted.
  - `cat ~/.ssh/<probe file>` via Bash fails, and native `Read` of the same file is denied. Use a probe file under a temporary HOME.
  - `curl -m 3 https://example.com` sends a `SandboxNetworkAccess` request.

- [ ] **Step 2: Run** `cargo test -p discard-providers --test claude_live -- --ignored autorun` — expect PASS. If `denyRead` or `permissions.deny` does not hold, stop: Claude must report `Unsupported` with that reason, and the spec owner decides next steps.

- [ ] **Step 3: Update the docs.** Record the probe results in the capability table in `docs/provider-claude.md`. Describe the setting, its limits (reads, Codex unsupported), and the indicators in `docs/worktrees.md`. Add one line to `docs/project-channel-status.md`.

- [ ] **Step 4: Run** the full check list: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm --dir web check`, `pnpm --dir web test`, `pnpm --dir web build`.

- [ ] **Step 5: Commit** `docs: record Claude sandbox autorun behavior and limits`
