# Mark Project and Group as Read Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make **Mark Project as Read** and **Mark Group as Read** work: one click marks every channel of the project (or of every member project of the group) and every thread under those channels as read.

**Architecture:** `StateStore` gets one batched marker write. `AppCore` reports the last message of each loaded channel of a set of projects (the command also covers channels that are not loaded and archived ones, using the last stored message) and applies a batch of markers (clearing divider anchors and waking the pump). Two Tauri commands compute the thread markers from storage, resolve a group to its projects, and call `AppCore`. The two menu items become real actions that call the commands.

**Tech Stack:** Rust (discard-core, discard-app), Tauri 2, Vue 3 + TypeScript, Vite+ tests.

**Spec:** `docs/superpowers/specs/2026-11-23-mark-as-read-design.md`

## Global Constraints

- A channel's or thread's marker becomes the ID of its last message. A channel counts every role. A thread counts user and agent messages only (the `ThreadRecord.rows` rule: no `System` messages).
- A marker that already equals the last message does not change. A channel or thread with no message changes nothing.
- One write: `StateStore::set_last_read_many(&mut self, items: &[(String, String)]) -> Result<(), String>`. On error the file and the in-memory state stay as they were.
- Commands: `mark_project_read(project_id: String)` and `mark_group_read(group_id: String)`. An unknown group ID is an error. A project with no channels, or an empty group, is `Ok` with no change.
- Pending approval badges are not touched. Muted channels are marked too. Direct messages are not touched (they belong to no project).
- Menu actions: the project item becomes the action `mark-read`; the group item becomes the action `group-mark-read`. Both are always enabled. Remove the `planned(...)` calls and their `TODO` comments. No `Soon` text for these items (AGENTS.md).
- Web tests go in a `__tests__` directory beside the code they test.
- Commit messages: no attribution lines.

## Review Focus

- A turn is running in a channel when the user clicks: only messages saved before the click become read; a reply saved after the click stays unread.
- The state file cannot be written: the command returns an error, no marker changes in memory, and the web shows the error toast.
- A group ID that does not exist, and a group with no member projects: an error for the first, `Ok` with no change for the second.
- A question thread that holds only the user's message (the agent has not answered): marking it read must not fail or hide a later answer.
- Marking twice in a row: the second call changes nothing and does not bump the revision or the conversations revision.

---

### Task 1: Batched read-marker write

**Files:**
- Modify: `crates/discard-core/src/window_state.rs` (next to `set_last_read`, ~line 144; tests in the same file)

**Interfaces:**
- Produces: `StateStore::set_last_read_many(&mut self, items: &[(String, String)]) -> Result<(), String>`. Each item is `(channel_or_thread_id, message_id)`.

- [ ] **Step 1: Write failing tests** in the existing test module of `window_state.rs`: `set_last_read_many_saves_every_marker_in_one_write` (after the call, `last_read("a")` and `last_read("b")` return the new IDs and a reopened store reads both), `set_last_read_many_with_no_items_writes_nothing` (the file's modified time or existence is unchanged), and `a_failed_set_last_read_many_changes_nothing` (make the target path unwritable the same way the existing failed-save test for `set_last_read` does; both `last_read` values stay as before and the call returns `Err`).
- [ ] **Step 2: Run** `cargo test -p discard-core window_state`. Expected: compile error, then FAIL.
- [ ] **Step 3: Implement `set_last_read_many`.** Clone the state, insert every pair, then call `self.save(next)`. Return `Ok(())` without saving when `items` is empty.
- [ ] **Step 4: Run** `cargo test -p discard-core window_state`. Expected: PASS.
- [ ] **Step 5: Commit** `git add crates/discard-core && git commit -m "feat: save several read markers in one write"`.

### Task 2: AppCore reads and applies read markers

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs` (new methods near `mark_read_in`, ~line 1173; tests in the existing test modules)

**Interfaces:**
- Consumes: `StateStore::set_last_read_many` (Task 1).
- Produces:
  - `AppCore::loaded_channel_tails(&self, project_ids: &[String]) -> Vec<(String, Option<String>)>`: for every loaded non-thread channel whose `project_id` is in `project_ids`, `(channel_id, id of the last message of its history, or None when the history is empty)`.
  - `AppCore::apply_read_markers(&self, markers: &[(String, String)]) -> Result<bool, String>`: drops every pair whose marker already equals the stored one, saves the rest with `set_last_read_many`, clears `new_anchor` of each changed loaded channel, bumps `rev` (and `history_rev` when a changed channel is shown by a view), bumps `conversations_rev` when at least one marker changed, wakes the pump, and returns whether anything changed. On a save error it returns `Err` and changes nothing in memory.

- [ ] **Step 1: Write failing tests** named for the rules:
  - `loaded_channel_tails_lists_only_channels_of_the_given_projects` (two projects; a thread channel and a channel of the other project are absent; an empty-history channel gives `None`)
  - `apply_read_markers_marks_a_channel_read_and_clears_its_divider_anchor` (unread before, `channel_indicators` read after)
  - `apply_read_markers_skips_a_marker_that_already_matches_and_reports_no_change` (second call returns `Ok(false)` and `rev` and `conversations_rev` do not move)
  - `a_reply_saved_after_the_tails_were_read_stays_unread` (read tails, apply, then apply a later agent message update: the channel is unread again)
  - `apply_read_markers_changes_nothing_when_the_write_fails` (make the state file unwritable like other core tests do; `Err`, markers unchanged, `rev` unchanged)
  - `apply_read_markers_leaves_pending_approvals_alone` (a pending approval stays counted)
- [ ] **Step 2: Run** `cargo test -p discard-app core::`. Expected: FAIL (methods missing).
- [ ] **Step 3: Implement** both methods. Read channel data under the `state` lock and drop it before any other lock. Reuse the anchor-clearing and `history_rev` logic of `mark_read_in`.
- [ ] **Step 4: Run** `cargo test -p discard-app core::` then `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `git add crates/discard-app && git commit -m "feat: apply read markers for loaded channels in one step"`.

### Task 3: The mark-as-read commands

**Files:**
- Create: `crates/discard-app/src/desktop/mark_read.rs` (register with `pub mod mark_read;` in `desktop/mod.rs`)
- Modify: `crates/discard-app/src/desktop/commands.rs` (two commands near `mark_read`, ~line 327)
- Modify: `crates/discard-app/src/desktop/mod.rs` (add both commands to `generate_handler!` near `commands::mark_read`, ~line 353)

**Interfaces:**
- Consumes: `AppCore::loaded_channel_tails`, `AppCore::apply_read_markers` (Task 2); `Store::rail_layout()` (returns `RailLayout { items: Vec<RailItem> }`; `RailItem::Group(g)` has `id` and `project_ids`); `Store::threads_for_channel(&ChannelId) -> Result<Vec<ThreadInfo>>`; `Store::messages_for_channel(&ChannelId) -> Result<Vec<Message>>`.
- Produces:
  - `pub fn group_project_ids(layout: &discard_core::store::RailLayout, group_id: &str) -> Result<Vec<String>, String>`: the member project IDs, or `Err("unknown group")` for an unknown ID.
  - `pub fn thread_markers(store: &discard_core::store::Store, channel_ids: &[String]) -> Result<Vec<(String, String)>, String>`: for every thread of each channel (followed or not), `(thread_id, id of its last user or agent message)`; a thread with no such message is left out.
  - `pub fn mark_projects_read(core: &AppCore, store: &Mutex<Store>, project_ids: &[String]) -> Result<bool, String>`: reads the tails, builds the thread markers while holding the store lock only for that step, then calls `apply_read_markers` with channel and thread markers together.
  - Tauri commands `mark_project_read(context: State<SettingsContext>, project_id: String) -> Result<(), String>` and `mark_group_read(context: State<SettingsContext>, group_id: String) -> Result<(), String>`.

- [ ] **Step 1: Write failing tests** in `mark_read.rs`: `group_project_ids_returns_the_members_in_order`, `group_project_ids_rejects_an_unknown_group`, `an_empty_group_gives_no_projects`, `thread_markers_use_the_last_user_or_agent_message_and_skip_system_messages`, `a_thread_with_only_a_user_message_still_gets_a_marker`, `a_thread_with_no_message_gets_no_marker`, and `mark_projects_read_marks_channels_and_threads_in_one_write` (a store fixture with one project, two channels, one thread under the first: all three markers set; a second call returns `Ok(false)`). Build store fixtures the way `threads.rs` and `commands.rs` tests do.
- [ ] **Step 2: Run** `cargo test -p discard-app mark_read`. Expected: FAIL.
- [ ] **Step 3: Implement** the three functions and the two commands. The commands map `Ok(_)` to `Ok(())`. `mark_group_read` resolves the group with `group_project_ids` from `store.rail_layout()`, then calls `mark_projects_read`. Register both commands.
- [ ] **Step 4: Run** `cargo test -p discard-app && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git add crates/discard-app && git commit -m "feat: add the mark project and group as read commands"`.

### Task 4: Menu items, web calls, and docs

**Files:**
- Modify: `web/src/bridge/api.ts` (two functions near `ungroupRailGroup`, ~line 283)
- Modify: `web/src/components/project-menu.ts` (items at ~line 60 and ~line 174; action types)
- Modify: `web/src/components/ServerRail.vue` (`onGroupAction`, ~line 170) and `web/src/App.vue` (`projectAction`, ~line 639, and the handler of the rail `railChange` event)
- Test: `web/src/components/__tests__/project-menu.test.ts` (~lines 41 and 250), `web/src/components/__tests__/ServerRail.groups.test.ts` (~line 296)
- Modify: `docs/settings.md` (project and group menu paragraph near line 492), `docs/project-channel-status.md`

**Interfaces:**
- Consumes: Tauri commands `mark_project_read` and `mark_group_read` (Task 3).
- Produces: `markProjectRead(projectId: string): Promise<void>` and `markGroupRead(groupId: string): Promise<void>` in `api.ts`; menu action ids `mark-read` (project) and `group-mark-read` (group).

- [ ] **Step 1: Update the failing tests.** In `project-menu.test.ts`: `Mark Project as Read` is `{ kind: 'action', label: 'Mark Project as Read', action: 'mark-read' }` and `Mark Group as Read` is `{ kind: 'action', label: 'Mark Group as Read', action: 'group-mark-read' }`. In `ServerRail.groups.test.ts`: the `Mark Group as Read` item is not `aria-disabled` and its text has no `Soon`; clicking it emits the group read request for that group's ID (mirror how the test checks `Ungroup`). Add a test that choosing `Mark Project as Read` in the project menu calls `markProjectRead` with the project ID (mock `api.ts` as the neighboring tests do).
- [ ] **Step 2: Run** `pnpm --dir web test project-menu ServerRail`. Expected: FAIL.
- [ ] **Step 3: Implement.** `api.ts`: `invoke<void>('mark_project_read', { projectId })` and `invoke<void>('mark_group_read', { groupId })`. `project-menu.ts`: add `'mark-read'` to `ProjectMenuAction` and `'group-mark-read'` to `GroupMenuAction`; turn the two planned items into actions; delete their `TODO` comments. Wire the project action in `App.vue` `projectAction` (call `markProjectRead`; the existing `catch` shows the error toast) and the group action through the same path `group-ungroup` takes (`ServerRail.vue` emits, `App.vue` calls `markGroupRead` and shows an error toast on failure). The state snapshot already refreshes through the pump, so call no extra refresh.
- [ ] **Step 4: Update the docs** in short, direct sentences: both items now work; what they clear (channels and threads of the project, or of every project in the group); what they leave (pending approval badges, direct messages); the planned list that named them (`Mark Group as Read` Soon) loses them.
- [ ] **Step 5: Run** `pnpm --dir web test` then `pnpm --dir web check`. Expected: tests PASS. `check` may fail on `MoveRequestDetails.vue` formatting, which is already on `main`; report that and nothing else.
- [ ] **Step 6: Commit** `git add web docs && git commit -m "feat: enable mark project and group as read"`.

## Self-review notes

- Spec coverage: decisions 1 and 2 in Tasks 2 and 3 (scope, marker rule); decision 3 in Task 1; decision 4 in Task 3; decision 5 in Task 2; decision 6 in Task 4; decision 7 in Task 4.
- Calls the spec left open, made here: threads are found from storage (`threads_for_channel`), not only from loaded records, because the web reads chips from storage; the group's project list comes from the saved rail layout; a project with no channels is `Ok` (the spec's "unknown project is an error" was narrowed to groups, and the spec was amended to match).
