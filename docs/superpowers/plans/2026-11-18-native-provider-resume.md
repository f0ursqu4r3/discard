# Native provider resume per agent — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep one provider session per (channel, agent) and continue it with only new information, falling back to full replay whenever Discard state and provider session drift.

**Architecture:** A pure `native_resume` module decides "resume or replay" from a saved `agent_sessions` row and a session fingerprint. The engine sends a delta (new messages, a context update, volatile session state, the current message) on resume and the existing replay otherwise. Adapters accept a resume handle in Conversation and Workspace when the engine passes one. Replay stays the reference behavior.

**Tech Stack:** Rust workspace (`discard-core`, `discard-mcp`, `discard-providers`, `discard-app`), SQLite store (`rusqlite`), Vue 3 + Vitest in `web/`.

**Spec:** [2026-11-18-native-provider-resume-design.md](../specs/2026-11-18-native-provider-resume-design.md). The spec is the authority. Conflicts in this plan resolve against it.

## Global Constraints

- Build only after the spike (Task 1) records its answers. A spike result can shrink scope: if a provider cannot re-apply a control on resume, that adapter refuses resume (spec "Spike").
- Isolation stays: no user MCP servers, hooks, provider memory, or plugins. A resumed process gets the same flags, settings, and a **new** per-session MCP config file as a fresh one. Each response still issues a new MCP token.
- `native_resume` is `false` by default. It needs no policy-loosening confirmation. The settings note reads: "Keeps this agent's provider session between responses. The provider stores the conversation on disk."
- Replay path, saved transcript, and the `sessions` table (Coding) stay unchanged. The one summary change is the lower target of Task 10 (about 50% of the input budget, only for agents with `native_resume` on).
- No resume for summary sessions, sub-agent threads (`ReadOnlyReason::Thread` turns), or the `--builder` Coding path.
- Anti-goal: a resumed agent that answers from stale context. When in doubt, replay.
- Web tests go in a `__tests__` directory beside the code. UI text states implemented behavior only. Any planned control stays visible, disabled, labelled `Soon`, with a `TODO` in code (AGENTS.md).
- User-facing prose uses short, direct sentences and active voice.
- Commit nothing unless the owner asks. Never add Claude attribution to commits, PRs, or comments.
- Proof command (run after each task that touches Rust or web, and in full at the end):
  `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`
- Plan rulings (differ in detail from the spec; the behavior is the same):
  - The fingerprint is stored as canonical JSON, not a hash, so a notice can name the field that changed.
  - "Own replies" (spec call 5) is implemented as a contiguous-prefix rule on `delivered_through`, not as a recorded ID list. See Task 8.
  - A row keeps `dropped_reason` so the notice shows on the replay that follows a drop. Update the spec's session-record table and call 5 in Task 12.

## Review Focus

1. **Two agents in one channel.** Agent B replies between two turns of agent A. A's next delta must include B's reply and still skip A's own reply. (Task 8)
2. **History edits.** Deleting or editing a message at or before `delivered_through` must force a replay, never a doubtful resume. (Tasks 3, 4)
3. **Unsafe turns.** A cancelled turn, a failed turn, and `ResponseSaveFailed` each drop the handle, and the next response replays with the right notice. A steer does not. (Task 9)
4. **Rejected handle.** A rejected resume replays once in the same response. If that replay fails too, the turn fails. It never loops. (Task 9)
5. **Cleanup and opt-out.** Archiving or deleting a channel, deleting an agent, turning `native_resume` off, or changing provider or model leaves no usable row. (Tasks 3, 8)

---

## File Structure

- Create `crates/discard-core/src/native_resume.rs`: fingerprint, drop reasons, decision, delta composition, byte estimate, history digest. No I/O.
- Modify `crates/discard-core/src/agent.rs`, `agent_edit.rs`: the setting.
- Modify `crates/discard-core/src/store.rs`: schema v16, `agent_sessions` API, cleanup.
- Modify `crates/discard-core/src/engine.rs`: resume path, bookkeeping, drop rules.
- Modify `crates/discard-core/src/conversation_context.rs`: summary target parameter.
- Modify `crates/discard-core/src/provider.rs`: `SessionSpec.resumable`, `Provider::resume_epoch`.
- Modify `crates/discard-core/src/ui_state.rs`, `workspace_session.rs`: resume state for the UI.
- Modify `crates/discard-mcp/src/service.rs`: structured memory parts, `resume_dir`.
- Modify `crates/discard-providers/src/{fake.rs,conversation.rs,claude/provider.rs,codex/provider.rs}` and their tests.
- Modify `web/src/bridge/{api.ts,types.ts}`, `web/src/components/agents/AgentProfileEditor.vue`, `web/src/components/workspace/ContextPanel.vue` and tests.
- Docs: `docs/provider-claude.md`, `docs/provider-codex.md`, `docs/runtime/conversation-context.md`, `docs/project-channel-status.md`, `docs/settings.md`, the spec.

---

## Slice 1

### Task 1: Spike

**Files:** Modify `docs/provider-claude.md`, `docs/provider-codex.md`. Scratch work in the scratchpad directory only.

**Interfaces:** Produces five recorded answers that later tasks read: (a) Claude resumes from a stable empty directory; (b) which controls re-apply on resume, per provider; (c) the "handle rejected" marker per provider; (d) where sessions live and whether `HOME`/`CODEX_HOME` moves them; (e) whether each CLI compacts a long session on its own, with a flag or a stream signal.

- [ ] **Step 1:** Claude 2.1.318, Conversation isolation flags, a stable empty directory. Start a session with a token-A MCP config and a system prompt P1. Resume with `--resume <id>`, a token-B MCP config, `--tools`, `--permission-mode`, `--append-system-prompt P2`, and the sandbox settings. Record which values take effect and whether `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` still applies.
- [ ] **Step 2:** Codex 0.166.0. Start a thread with `ephemeral:false`, then `thread/resume` with a changed per-session MCP config and the Conversation controls (`codex_controls`, tools off). Record whether the new config, model, and `developerInstructions` hold, and whether the original ones persist.
- [ ] **Step 3:** Resume an unknown and an expired ID on both CLIs. Record the exact error text, exit code or JSON-RPC code. Choose the marker the engine and adapters read as "handle rejected".
- [ ] **Step 4:** Record the session store path for each CLI, with and without a changed `HOME`/`CODEX_HOME`.
- [ ] **Step 5:** Drive a long session past the context window on each CLI. Record whether it compacts, whether a flag disables it, and what the stream shows.
- [ ] **Step 6:** Add the answers to the capability tables of both provider docs. For each control that does not re-apply, write "adapter refuses resume" in the doc and in the ledger. Tasks 7 and 11 build only the passing providers.

Verification: both provider docs have all five answers with the CLI version. No product code changes.

### Task 2: The `native_resume` setting

**Files:** Modify `crates/discard-core/src/agent.rs`, `agent_edit.rs`, `store.rs` (struct literals), `web/src/bridge/api.ts`, `web/src/components/agents/AgentProfileEditor.vue`. Test: `agent.rs` tests, `agent_edit.rs` tests, `web/src/components/agents/__tests__/profiles.test.ts`.

**Interfaces:**
- Produces: `AgentConfig.native_resume: bool` (`#[serde(default)]`), `AgentDraft.native_resume: bool`. Web `AgentProfile.native_resume: boolean`.

- [ ] **Step 1: Write failing tests.** `old_agent_toml_reads_native_resume_as_false` (as the `sandbox_autorun` test at `agent.rs:164`). `saving_a_draft_with_native_resume_persists_it` in `agent_edit.rs` and asserts `needs_confirm` is false for the change. `AgentProfileEditor` renders a "Keep provider session" checkbox bound to `native_resume` with the disk note beside it.
- [ ] **Step 2: Run** `cargo test -p discard-core agent` and `pnpm --dir web test profiles`. Expect FAIL.
- [ ] **Step 3: Implement.** Add the field to `AgentConfig`, `AgentDraft`, its `From<&AgentConfig>`, and the diff list in `agent_edit.rs` (label "Native resume"). No confirmation path. Fix every struct literal the compiler reports (about 6 in `store.rs`, `agent.rs`, tests). Mirror the field in `api.ts` and the editor.
- [ ] **Step 4: Run** the two commands again. Expect PASS. Run `cargo build --workspace`.

### Task 3: `agent_sessions` table and store API

**Files:** Modify `crates/discard-core/src/store.rs`. Test: `store.rs` tests.

**Interfaces:**
- Produces, in `store.rs` (or a `store/agent_sessions.rs` module beside `rail`):
  ```rust
  pub struct AgentSession {
      pub channel_id: ChannelId, pub agent_id: AgentId,
      pub handle: String, pub fingerprint: String, // canonical JSON
      pub delivered_through: Option<MessageId>,
      pub delivered_memory: BTreeMap<String, u64>,
      pub delivered_topic_version: Option<u64>,
      pub delivered_capture_paused: bool,
      pub bytes_in_session: u64, pub cwd: String,
      pub dropped_reason: Option<String>, // set: handle is unusable; next response replays and shows it
  }
  impl Store {
      pub fn agent_session(&self, channel: &ChannelId, agent: &AgentId) -> Result<Option<AgentSession>>;
      pub fn put_agent_session(&self, row: &AgentSession) -> Result<()>;            // upsert
      pub fn drop_agent_session(&self, channel: &ChannelId, agent: &AgentId, reason: &str) -> Result<()>; // sets dropped_reason, keeps the row
      pub fn delete_agent_session(&self, channel: &ChannelId, agent: &AgentId) -> Result<()>;
  }
  ```

- [ ] **Step 1: Write failing tests.** `migration_16_adds_agent_sessions` (copy the pattern at `store.rs:5507`: drop the table, set `user_version=15`, reopen, assert `schema_version()==16`; also update the existing assertions of `15`). `agent_session_round_trips_with_memory_map`. `drop_then_put_clears_the_reason`. `deleting_a_channel_removes_its_rows`, `archiving_a_channel_removes_its_rows`, `deleting_an_agent_removes_its_rows`.
- [ ] **Step 2: Run** `cargo test -p discard-core store`. Expect FAIL.
- [ ] **Step 3: Implement.** Migration block `if self.schema_version()? < 16` in `migrate()` creating `agent_sessions(channel_id, agent_id, handle, fingerprint, delivered_through, delivered_memory, delivered_topic_version, delivered_capture_paused, bytes_in_session, cwd, dropped_reason, PRIMARY KEY(channel_id, agent_id))`, foreign keys as the neighbouring tables do. Call `delete_agent_session`-style cleanup inside `archive_channel` (`store.rs:2088`), the channel-delete path, and `delete_agent` (`store.rs:1619`), in the same transaction.
- [ ] **Step 4: Run** the same command. Expect PASS.

### Task 4: `native_resume` pure module

**Files:** Create `crates/discard-core/src/native_resume.rs`; register in `lib.rs`. Test: unit tests in the module.

**Interfaces:**
- Consumes: `AgentSession` (Task 3), `conversation_context::{replay_with_posted, input_budget}`.
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
  pub struct SessionFingerprint {
      pub provider: String, pub model: String, pub effort: Option<String>, pub mode: String,
      pub session_dir: String, pub fixed_prompt_hash: String, pub tools: Vec<String>,
      pub delegation: bool, pub sandbox: bool, pub autorun: bool,
      pub cli_version: Option<String>, pub summary_through: Option<String>, pub history_digest: String,
  }
  impl SessionFingerprint {
      pub fn canonical(&self) -> String;                          // stable JSON for storage
      pub fn first_difference(&self, saved: &str) -> Option<&'static str>; // field label; unparsable saved => Some("session record")
  }
  pub fn history_digest(messages: &[(MessageId, String)], through: Option<&MessageId>) -> String; // sha256 over replayed text up to and including `through`
  pub enum DropReason { FingerprintChanged(&'static str), PreviousTurnUnsafe(String), HandleRejected, OverBudget, ProviderCompacted }
  impl DropReason { pub fn notice(&self) -> String }            // "Fresh replay: the model changed." and so on
  pub enum ResumeDecision { Fresh, Resume(AgentSession), Replay(DropReason) }
  pub fn decide(saved: Option<&AgentSession>, current: &SessionFingerprint, incoming_bytes: usize, budget: usize) -> ResumeDecision;
  pub fn compose_resume(context_update: Option<&str>, session_state: &str, messages: &[(MessageId, String)], current: &str) -> String;
  pub fn context_update(delivered: &BTreeMap<String, u64>, selected: &[SelectedMemory], live_ids: &[String], topic: Option<&str>, capture_paused: Option<bool>) -> Option<String>;
  pub fn undelivered(messages: &[(MessageId, String)], through: Option<&MessageId>) -> &[(MessageId, String)]; // reuse after_boundary semantics
  pub fn bytes_after_turn(prev: u64, input_len: usize, reply_len: usize) -> u64; // never below input_len + reply_len + prev
  ```
  `SelectedMemory { id: String, version: u64, scope: String, kind: String, text: String }` lives here and is re-used by Task 5.

- [ ] **Step 1: Write failing tests.**
  - `fingerprint_names_each_changed_field` (one case per field: model, effort, mode, session_dir, fixed prompt, tools, delegation, sandbox, autorun, cli_version, summary boundary, history digest).
  - `decide_returns_fresh_without_a_row`, `decide_replays_when_dropped_reason_is_set` (reason carried into the notice), `decide_replays_over_budget` (`bytes_in_session + incoming_bytes > budget`), `decide_resumes_when_everything_matches`.
  - `history_digest_changes_when_an_earlier_message_changes_or_disappears` and `history_digest_ignores_later_messages`.
  - `context_update_lists_new_and_changed_items_and_retractions` (item at new version is delivered; unchanged is not; delivered item absent from `live_ids` is retracted; delivered item that merely stopped matching but is live stays); `context_update_is_none_when_nothing_changed`.
  - `compose_resume_orders_update_state_messages_then_current` and labels the update as context data (follow the wording of the topic block at `engine.rs:300`).
- [ ] **Step 2: Run** `cargo test -p discard-core native_resume`. Expect FAIL.
- [ ] **Step 3: Implement** the signatures above. `sha2` is a workspace dependency; if absent, use the hasher the crate already uses for `committed_body_hash` in `memory_items.rs`. `notice()` texts are the builder's wording; each starts "Fresh replay:" and names the reason.
- [ ] **Step 4: Run** the same command. Expect PASS.

### Task 5: Structured memory parts from the memory service

**Files:** Modify `crates/discard-core/src/engine.rs` (`PreparedMemory`, `MemoryService`), `crates/discard-mcp/src/service.rs` (`prepare_scoped`). Test: `crates/discard-mcp/src/service.rs` tests and `crates/discard-mcp/tests/`.

**Interfaces:**
- Consumes: `SelectedMemory` (Task 4).
- Produces, on `PreparedMemory` (new field `parts: Option<ResumableParts>`, `None` for services that do not provide it):
  ```rust
  pub struct ResumableParts {
      pub fixed_prompt: String,          // base prompt + mandatory (instruction/preference) blocks + fixed lines
      pub session_state: String,         // volatile lines; empty string when none
      pub core_hash: String,             // hash of the mandatory blocks; goes in the fixed prompt hash input
      pub selected: Vec<SelectedMemory>, // non-mandatory items selected for this turn
      pub live_ids: Vec<String>,         // IDs of every live applicable item
      pub capture_paused: bool,
  }
  ```
  and on `MemoryService`: `fn resume_dir(&self, channel: &ChannelId, agent: &AgentId) -> Option<PathBuf> { None }` (default). `MemoryRuntime` returns `<runtime parent>/sessions/<channel>/<agent>`, created with mode 0700.

- [ ] **Step 1: Write failing tests.** `system_prompt_is_unchanged_by_the_split` (golden: same string as before for a fixture with memory, worktree, move line, delegation). `parts_separate_fixed_and_volatile_lines`: fixed holds personality, mandatory blocks, tool-naming lines (worktree access, project access) and the delegation line; volatile holds "Recent worktree approval outcomes", "This branch channel has no worktree", main-checkout availability and the Primary / Moderator move line. `capture_pause_is_not_in_the_fixed_prompt`. `selected_excludes_instruction_and_preference_items`. `changing_an_instruction_changes_core_hash`. `resume_dir_is_stable_per_channel_and_agent`.
- [ ] **Step 2: Run** `cargo test -p discard-mcp`. Expect FAIL.
- [ ] **Step 3: Implement.** In `prepare_scoped` collect the prompt as ordered segments tagged fixed or volatile, then build `system_prompt` by joining all in the original order (so replay is byte-identical) and `fixed_prompt` / `session_state` from the tagged segments. The capture-pause sentence goes in neither fixed nor volatile text of `parts`; its flag travels as `capture_paused`. Return `parts: Some(..)` from `prepare_scoped` for every surface; summary callers ignore it.
- [ ] **Step 4: Run** `cargo test -p discard-mcp && cargo test -p discard-core`. Expect PASS.

### Task 6: Provider plumbing and the fake provider

**Files:** Modify `crates/discard-core/src/provider.rs`, `crates/discard-providers/src/fake.rs`, every `SessionSpec { .. }` literal the compiler flags (`engine.rs:361,818`, `conversation.rs:431`, tests). Test: `fake.rs` tests.

**Interfaces:**
- Produces: `SessionSpec.resumable: bool` (the session must persist; `spec.cwd` is the stable directory the engine chose). `fn resume_epoch(&self) -> Option<String> { None }` on `Provider` (the CLI version as `check_setup` last read it). `fn compacted(&self) -> bool { false }` on `ProviderSession` (true when the CLI compacted the session during the turn). `ProviderError::ResumeRejected` already exists and stays the only "handle rejected" signal.
- Fake: records `started_with` handles (exists) and `resumable`; `with_rejecting_resume()` makes `start_session` return `ResumeRejected` when a handle is given.

- [ ] **Step 1: Write failing tests** in `fake.rs`: `records_resumable_flag`, `a_rejecting_fake_refuses_a_handle_and_accepts_none`, `the_fake_can_report_a_compaction`.
- [ ] **Step 2: Run** `cargo test -p discard-providers fake`. Expect FAIL.
- [ ] **Step 3: Implement.** Add the field and trait method. Set `resumable: false` at every existing literal.
- [ ] **Step 4: Run** `cargo test --workspace --no-run` then `cargo test -p discard-providers fake`. Expect PASS.

### Task 7: Claude adapter

**Files:** Modify `crates/discard-providers/src/claude/provider.rs`, `conversation.rs`. Test: `crates/discard-providers/tests/claude_process.rs`.

**Interfaces:**
- Consumes: Task 1 answers (a), (b), (c); `SessionSpec.resumable`, `Provider::resume_epoch`.
- Behavior: with `spec.resumable` the adapter (1) accepts `resume_handle` in Conversation and Workspace, (2) uses `spec.cwd` as the working directory instead of a fresh temp directory (it must be an absolute, existing directory), (3) keeps session persistence on, (4) maps the Task 1 "unknown session" marker to `ProviderError::ResumeRejected`, (5) runs every fail-closed check of a fresh session, (6) passes every flag, setting, MCP config and the system prompt again on resume (the spike shows `--append-system-prompt` is not stored), (7) sets `DISABLE_AUTO_COMPACT=1` in the process environment so Discard owns the size guard (spike answer e), and (8) reports `compacted()` true if the stream shows a `compact_boundary` event. Without `resumable`, a handle in Conversation or Workspace still fails with `setup(..)` as today (`claude/provider.rs:257`).

- [ ] **Step 1: Write failing tests** with the fixture process in `claude_process.rs`: `resumable_session_passes_resume_and_the_same_isolation_flags_as_a_fresh_one` (compare the two arg lists; only `--resume` differs), `resumable_session_runs_in_the_given_directory`, `a_handle_without_resumable_is_still_refused_in_conversation`, `the_unknown_session_error_maps_to_resume_rejected` (marker: exit 1, no `init`, `result` with `subtype: "error_during_execution"` and an `errors[]` entry starting `No conversation found with session ID`), `resumable_sessions_set_disable_auto_compact`, `a_compact_boundary_event_sets_compacted`.
- [ ] **Step 2: Run** `cargo test -p discard-providers --test claude_process`. Expect FAIL.
- [ ] **Step 3: Implement.** Cache the `--version` output that `check_setup` and `start_session` already read in a `Mutex<Option<String>>` and return it from `resume_epoch`. If Task 1 recorded that a control does not re-apply on resume, return `setup("... cannot resume")` for `resumable` and skip steps 1 and 2.
- [ ] **Step 4: Run** the same command. Expect PASS.

### Task 8: Engine — the resume path

**Files:** Modify `crates/discard-core/src/engine.rs` (`turn_snapshot`, `conversation_input`). Test: `crates/discard-providers/tests/conversation_runtime.rs` (fake-provider engine tests).

**Interfaces:**
- Consumes: Tasks 2–6. The agent flag comes from `load_agent(&self.agent_id).native_resume`. Resume applies when `self.mode == Conversation`, the turn is not a thread turn, the summary path is not running, and `native_resume` is on and the provider reports a version or does not need one.
- Produces: `fn native_resume_enabled(&self, thread_turn: bool) -> bool` on `Engine`, and a private `struct ResumeTurn { decision: ResumeDecision, fingerprint: SessionFingerprint, delivered_ids: Vec<MessageId> }`.
- Behavior:
  - Build the fingerprint before `SessionSpec`: provider id, model, effort, mode, session dir (Workspace root, or `memory.resume_dir`), fixed prompt hash (`parts.fixed_prompt` + workspace prompt + repository instructions + `core_hash`), tool list and delegation flag (`conversation_tools` equivalent), sandbox/autorun, `provider.resume_epoch()`, `state.through` from `conversation_context`, `history_digest` over replayed messages up to the saved `delivered_through`.
  - `decide` runs with `incoming_bytes` = size of the delta input. A `Fresh` or `Replay` outcome follows the existing replay path with the fixed system prompt (selected items and session state go in the first context block, not the system prompt). A `Replay` outcome sends `Update::Notice(reason.notice())` and saves a marker like `conversation_input` does.
  - A `Resume` outcome sets `spec.resume_handle`, `spec.resumable = true`, `spec.system_prompt = fixed system` (adapters ignore it if the CLI keeps the old one; Task 1 decides), and sends `compose_resume(update, session_state, undelivered, current)` where `current` carries topic, worktree, and project snapshots exactly as `conversation_input` builds them today.
  - After a **completed** turn save the row: new handle, fingerprint, `delivered_memory` (adds the delivered items), topic version, capture flag, `bytes_in_session` via `bytes_after_turn`, `cwd`, and `delivered_through` = the last message of the **contiguous prefix** of non-system messages that the session saw or produced: everything delivered in this turn's input, the current user message, and this turn's saved reply. A message that arrived mid-turn stops the prefix, so the next delta delivers it. Only turns that produced this session's reply advance past it, so another agent's reply is always delivered and the agent's own reply is never repeated.
  - A turn with `native_resume` off deletes any row for (channel, agent) at the start of the turn.

- [ ] **Step 1: Write failing tests** (fake provider, in-memory store, a `FakeMemory` that returns `parts`):
  - `second_response_resumes_and_sends_only_new_messages` (`started_with` shows `[None, Some(handle)]`; second input excludes the first exchange; includes the new message).
  - `a_changed_non_mandatory_memory_item_goes_in_the_context_update_and_keeps_the_handle`.
  - `a_changed_instruction_item_replays_with_a_notice` (core hash).
  - `topic_and_capture_pause_changes_are_delivered_as_updates`.
  - `two_agents_in_one_channel_keep_separate_rows` and `agent_a_delta_includes_agent_b_reply_but_not_its_own` (Review Focus 1).
  - `a_message_posted_during_the_turn_is_delivered_next_turn`.
  - `deleting_an_earlier_message_forces_a_replay` (Review Focus 2).
  - `changing_model_provider_or_turning_the_setting_off_leaves_no_row` (Review Focus 5).
  - `a_thread_turn_and_a_summary_never_resume`.
- [ ] **Step 2: Run** `cargo test -p discard-providers --test conversation_runtime resume`. Expect FAIL.
- [ ] **Step 3: Implement** as described. Extract the per-turn context assembly of `conversation_input` (topic, worktree, project snapshots) into a helper that both paths call, so replay output stays byte-identical (the existing replay tests stay green).
- [ ] **Step 4: Run** `cargo test -p discard-providers --test conversation_runtime && cargo test -p discard-core`. Expect PASS.

### Task 9: Engine — drop rules, rejected handle, steers

**Files:** Modify `crates/discard-core/src/engine.rs` (the end of `turn_snapshot`, the `started` error arm). Test: `conversation_runtime.rs`.

**Interfaces:** Consumes Task 3 `drop_agent_session`, Task 4 `DropReason`.

- Rules:
  - Turn outcome `Cancelled` or `Failed`, or `ResponseSaveFailed`, calls `drop_agent_session(.., reason)` instead of saving state. A turn that a steer stopped (`steer.is_some()` and outcome not `Failed`) saves nothing and drops nothing; the steer turn continues the same row.
  - A rejected handle surfaces as `ProviderError::ResumeRejected` from `start_session` (Codex) **or from the first `send_user_message` of the resumed session (Claude; its init event may come only after the first message, so the adapter cannot wait in `start_session`)**. Handle both: clear the handle, rebuild the full replay input, start once more with no handle, and send the `HandleRejected` notice. A second failure ends the turn as failed with the provider's error. No loop.
  - A provider context-limit error during a resumed turn drops the handle (`OverBudget`) and fails this turn with the existing error; the next response replays.
  - A resumed process that cannot resume after an app restart is the `ResumeRejected` case.
  - A turn after which `session.compacted()` is true saves the row with `dropped_reason` = `ProviderCompacted` (the provider's copy no longer matches the transcript). The next response replays.

- [ ] **Step 1: Write failing tests:** `a_cancelled_turn_drops_the_handle_and_the_next_response_replays_with_a_notice`, `a_failed_turn_drops_the_handle`, `a_response_that_fails_to_save_drops_the_handle`, `a_steer_keeps_the_handle_and_the_continuation_resumes`, `a_rejected_handle_replays_once_in_the_same_response` (started_with `[Some, None]`; full replay input; notice), `a_rejected_handle_whose_replay_also_fails_ends_the_turn` (no third start), `over_budget_bytes_replay_on_the_next_response`, `a_compacted_session_replays_on_the_next_response`.
- [ ] **Step 2: Run** `cargo test -p discard-providers --test conversation_runtime drop`. Expect FAIL.
- [ ] **Step 3: Implement.** Keep the existing Coding path (`resume_handle` of `sessions`) untouched.
- [ ] **Step 4: Run** `cargo test -p discard-providers --test conversation_runtime`. Expect PASS.

### Task 10: Summary headroom

**Files:** Modify `crates/discard-core/src/engine.rs` (`conversation_input`), `conversation_context.rs`. Test: `conversation_context.rs` tests, `conversation_runtime.rs`.

**Interfaces:**
- Produces: `pub fn summary_target(budget: usize, resumable: bool) -> usize` in `conversation_context.rs`: `budget` when not resumable, `budget / 2` when resumable. `conversation_input` takes `resumable: bool`.
- Behavior: the loop still enters only when the input exceeds `available`. For a resumable agent it continues summarizing until the input is at most `summary_target(available, true)` or no messages remain. A non-resumable agent behaves as today.

- [ ] **Step 1: Write failing tests:** `summary_target_halves_only_for_resumable_agents`; engine test `a_channel_that_overflowed_once_resumes_on_the_next_turn` (fake provider with a small window; after the summary replay, the next response's `started_with` shows a handle); `a_non_resumable_agent_keeps_todays_summary_target`.
- [ ] **Step 2: Run** `cargo test -p discard-core conversation_context && cargo test -p discard-providers --test conversation_runtime headroom`. Expect FAIL.
- [ ] **Step 3: Implement.** Pass `native_resume_enabled` into `conversation_input`.
- [ ] **Step 4: Run** the same commands. Expect PASS.

### Task 11: Slice 1 gate

- [ ] **Step 1:** Run the proof command. Fix failures at their cause.
- [ ] **Step 2:** In the running app (`cargo tauri dev`), turn the setting on for a Claude agent. Send three messages with a memory change after the second. Confirm: the second and third responses resume, the memory change arrives, a model change shows "Fresh replay" in the transcript. Record the result in the ledger.

---

## Slice 2

### Task 12: Codex adapter

**Files:** Modify `crates/discard-providers/src/codex/provider.rs` (`thread_params`, `start_session` at the resume check near line 302, `check_setup`). Test: `crates/discard-providers/tests/codex_process.rs` and the `thread_params` unit tests.

**Interfaces:** Consumes Task 1 answers (b), (c). Behavior mirrors Task 7: `spec.resumable` with a handle in Conversation or Workspace selects `thread/resume` with the same controls as `thread/start`; a resumable fresh start sets `ephemeral:false`; a non-resumable start stays ephemeral; `Failure::Rejected` on resume (`codex/provider.rs:405`) and the Task 1 marker map to `ResumeRejected`; `resume_epoch` returns the CLI version. Spike results: Codex does not re-apply `developerInstructions` on resume. The engine's fixed-prompt hash already equals the prompt at thread start whenever the fingerprint matches, so no adapter check is needed beyond passing the same prompt. A resumable start and resume pass `-c model_auto_compact_token_limit=1000000000` (no off switch exists; the spike shows this delays compaction), and an `item/completed` with `item.type == "contextCompaction"` sets `compacted()`. The MCP config and the `-c` Conversation controls go on every spawn.

- [ ] **Step 1: Write failing tests:** `resumable_thread_passes_a_high_auto_compact_limit`, `a_context_compaction_item_sets_compacted`, `resumable_conversation_thread_is_not_ephemeral`, `non_resumable_conversation_thread_stays_ephemeral`, `resumable_handle_selects_thread_resume_with_conversation_controls`, `a_handle_without_resumable_is_still_refused_in_conversation`, `a_rejected_resume_maps_to_resume_rejected`.
- [ ] **Step 2: Run** `cargo test -p discard-providers codex`. Expect FAIL.
- [ ] **Step 3: Implement.** If Task 1 says Codex cannot re-apply a control, refuse resume for Codex and record it in the ledger; skip the resume parts of the tests.
- [ ] **Step 4: Run** the same command. Expect PASS.

### Task 13: UI indicator

**Files:** Modify `crates/discard-core/src/workspace_session.rs` or `ui_state.rs` (new `Update::ResumeState(ResumeState)`), `crates/discard-app/src/desktop` (event bridge), `web/src/bridge/types.ts`, `web/src/components/workspace/ContextPanel.vue`. Test: `web/src/components/workspace/__tests__/` and a core `ui_state` test.

**Interfaces:**
- Produces: `pub enum ResumeState { Resumed, Replay { reason: String } }`, sent once per response for an agent with `native_resume` on. Web type `ResumeState = { kind: 'resumed' } | { kind: 'replay'; reason: string }`.

- [ ] **Step 1: Write failing tests:** core test that the engine sends `ResumeState::Resumed` on a resumed turn and `Replay{reason}` after a drop; web test that `ContextPanel` shows "Resumed session" or "Fresh replay: <reason>" per agent beside the Autorun badge, and nothing when the setting is off.
- [ ] **Step 2: Run** `cargo test -p discard-core ui_state` and `pnpm --dir web test ContextPanel`. Expect FAIL.
- [ ] **Step 3: Implement** the update, its serialization, and the panel row.
- [ ] **Step 4: Run** both commands. Expect PASS.

### Task 14: Live probes and docs

**Files:** Add opt-in tests `crates/discard-providers/tests/claude_live.rs`, `codex_live.rs` (a three-turn conversation with a memory change between turns, ignored by default like the existing live tests). Modify `docs/runtime/conversation-context.md`, `docs/project-channel-status.md`, `docs/settings.md`, `docs/provider-claude.md`, `docs/provider-codex.md`, and the spec.

- [ ] **Step 1:** Add the two live probes next to the existing live tests, with the same opt-in gate.
- [ ] **Step 2:** Run each live probe once with its CLI installed. Record the result in the provider docs.
- [ ] **Step 3:** Document the feature: the setting and disk note in `docs/settings.md`; resume, the delta, drop rules, and headroom in `docs/runtime/conversation-context.md`; in `docs/project-channel-status.md` move "native provider resume per agent" from planned to implemented and keep the other planned items.
- [ ] **Step 4:** Update the spec: status "Approved. Implemented.", the `dropped_reason` column in the session-record table, and call 5 (contiguous-prefix rule).
- [ ] **Step 5:** Run the proof command in full. Expect all green.
