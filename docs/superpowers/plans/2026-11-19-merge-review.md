# Merge Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The `Merge review` tab of a branch channel shows what the branch adds to the default branch and lets the user record Approve or Request changes. It never changes Git.

**Architecture:** `discard-core` reads the review snapshot with read-only, bounded Git calls (`merge_review.rs`), keeps the append-only decision rows (`store/review.rs`), and derives the state from the newest row and the live head (`review_state.rs`). `discard-app` joins those into four Tauri commands. A new `ReviewView.vue` renders the snapshot and reuses the Changes tab components for files and diffs.

**Tech Stack:** Rust (Tauri 2, rusqlite), Vue 3 + TypeScript, Vitest through `vp test`, pnpm.

**Spec:** `docs/superpowers/specs/2026-11-19-merge-review-design.md`. Read it before each task. Decision numbers below (`D3`) refer to the "Decisions I made" list in it.

## Global Constraints

- All Git calls in this feature are read-only, bounded, and pass `--no-ext-diff --no-textconv` (same helpers as `workspace_diff.rs`: `worktree_access::{git, git_bounded}`).
- Target branch is `project_context::git::default_branch` (D1). Source is `Store::channel_branch`.
- `head` is the commit of `refs/heads/<branch>`, not the worktree's checked-out commit (D10). The review compares from the merge base (three-dot).
- State names, exactly: `Not reviewed`, `Approved`, `Approval out of date`, `Changes requested`, `Changes requested · new commits since`, `Merged`, `Nothing to review`.
- Exact copy: `The branch changed. Reload the review.`, `Nothing to review: no commits beyond <target>`, `This channel is archived.`, `Ask for a summary`, note prefix `Review of <short commit>: `.
- `merged` is true only when `ahead` is 0 and the newest stored decision's `head` is an ancestor of the target (D8). `ahead == 0` otherwise gives `Nothing to review`.
- The busy check runs before any row is written. Request changes and the summary button use the same busy rule as the branch actions: `AppCore::channel_busy_reason`.
- Schema is additive and idempotent (`CREATE TABLE IF NOT EXISTS`).
- `AGENTS.md`: remove the `TODO` and the `Soon` label from the tab. A channel that cannot be reviewed shows an empty state with its reason, not a disabled tab.
- Web tests go in a `__tests__` directory beside the code they test.
- No Claude attribution in commits. No `Co-Authored-By` trailer. Commit on `kyle/merge-review`, not on `main`.

## Review Focus

1. **Source branch deleted while the tab is open.** A refresh gives a `reason` ("The branch no longer exists."), not an error toast.
2. **Branch names that look like options.** A branch named `-x` or containing spaces or non-ASCII text must not become a Git option. Resolve through `refs/heads/<name>` and pass `--end-of-options` where Git supports it.
3. **Repository with no commits (unborn `HEAD`) or no default branch.** Gives a `reason`, never a panic.
4. **Whitespace-only or oversized note on Request changes.** Refused before the busy check writes anything; the cap equals the composer's message cap.
5. **Double click on Approve or Request changes.** Two identical requests (same decision, head, note) in a row create one row.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/discard-core/src/merge_review.rs` (create) | `ReviewSnapshot` and `snapshot()`. Read-only Git. |
| `crates/discard-core/src/review_state.rs` (create) | `Decision`, `ReviewDecision`, `ReviewState`, `review_state()`. Pure. |
| `crates/discard-core/src/store/review.rs` (create) | `review_decisions` table and its methods. |
| `crates/discard-core/src/lib.rs`, `store.rs` (modify) | `pub mod merge_review; pub mod review_state;`, `mod review;` and the schema call. |
| `crates/discard-app/src/desktop/review.rs` (create) | `ReviewDto`, the join of snapshot, rows and busy/archived state, and the decide/retry/summary logic. |
| `crates/discard-app/src/desktop/commands.rs`, `mod.rs` (modify) | Four Tauri commands and their registration. |
| `web/src/bridge/types.ts`, `api.ts` (modify) | `ReviewDto` types and four wrappers. |
| `web/src/components/review/ReviewView.vue` (create) | The tab body. |
| `web/src/components/review/ReviewActions.vue`, `ReviewCommits.vue` (create) | Decision controls and the commit list. |
| `web/src/App.vue` (modify) | Enable the tab, remove the `TODO`/`Soon`, badge, mount `ReviewView`. |
| `docs/` (modify) | Plugin-dogfooding row, worktrees doc note. |

---

### Task 1: Review snapshot (core, read-only Git)

**Files:**
- Create: `crates/discard-core/src/merge_review.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod merge_review;`)
- Test: unit tests in `merge_review.rs`, using temporary repositories like `project_context/git_tests.rs` (copy its `git()` helper that sets author, `commit.gpgsign=false`, and `core.hooksPath=/dev/null`).

**Interfaces:**
- Consumes: `crate::workspace_diff::ChangeFile` (reuse the type and its bounded per-file diff code; extract a shared helper from `workspace_diff.rs` instead of copying it), `crate::project_context::git::default_branch(root: &Path) -> Option<String>`, `crate::worktree_access::{git, git_bounded}`.
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommit { pub id: String, pub subject: String, pub author: String, pub time: i64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub source: Option<String>, pub target: Option<String>,
    pub head: Option<String>,          // full id of refs/heads/<source>
    pub head_short: Option<String>,
    pub base: Option<String>,          // full merge-base id
    pub base_short: Option<String>,
    pub ahead: u64, pub behind: u64,
    pub nothing_to_review: bool,       // ahead == 0
    pub merged: bool,                  // ahead == 0 and last_decided_head is an ancestor of target
    pub dirty: usize,                  // uncommitted + untracked paths in `folder`
    pub commits: Vec<ReviewCommit>,    // newest first, bounded
    pub files: Vec<ChangeFile>,        // base...head, bounded
    pub truncated: bool,
    pub reason: Option<String>,        // set: no review possible; commits and files empty
}

/// `folder` is the channel folder (worktree or project path). `branch` is the channel's bound branch, if any.
/// `last_decided_head` is the `head` of the newest stored decision.
pub fn snapshot(folder: &Path, branch: Option<&str>, last_decided_head: Option<&str>) -> ReviewSnapshot;
```

- [ ] **Step 1: Write the failing tests**

Test names and assertions (one temporary repo per test; `snapshot` never panics and never returns `Err`):

```rust
#[test] fn a_branch_with_commits_lists_them_newest_first_with_files_and_counts()
// main has "initial"; branch `feature` adds two commits touching a.txt and b.txt.
// assert ahead == 2, behind == 0, commits.len() == 2, commits[0].subject is the newest,
// files paths == ["a.txt","b.txt"], head == full id of refs/heads/feature, nothing_to_review == false, reason == None.
#[test] fn a_branch_with_no_commits_ahead_has_nothing_to_review_and_is_not_merged()
// branch created at main's tip. assert ahead == 0, nothing_to_review, !merged, commits/files empty.
#[test] fn a_branch_is_merged_only_when_a_stored_decision_head_is_on_the_target()
// feature merged into main with --no-ff. snapshot(.., Some(feature_head)) -> merged == true.
// snapshot(.., None) and snapshot(.., Some("0000...")) -> merged == false, nothing_to_review == true.
#[test] fn a_branch_behind_the_target_reports_behind_and_keeps_its_own_commits_only()
#[test] fn the_default_branch_gives_a_reason()           // branch == Some("main") -> reason == Some("This channel is on the default branch.")
#[test] fn no_bound_branch_gives_a_reason()              // None -> Some("This channel has no branch.")
#[test] fn a_deleted_source_branch_gives_a_reason()      // Some("The branch no longer exists.")
#[test] fn a_repository_without_commits_gives_a_reason() // git init only -> reason set, no panic
#[test] fn a_missing_default_branch_gives_a_reason()     // reason == Some("The target branch cannot be determined.")
#[test] fn unrelated_histories_give_a_no_merge_base_reason()
#[test] fn a_branch_name_that_starts_with_a_dash_is_not_read_as_an_option()
#[test] fn dirty_counts_uncommitted_and_untracked_paths_and_is_not_part_of_the_diff()
#[test] fn a_large_diff_is_cut_and_marked_truncated()    // reuse the byte limit that workspace_diff uses
#[test] fn the_folder_not_being_a_repository_gives_a_reason()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p discard-core merge_review`
Expected: FAIL (module does not exist).

- [ ] **Step 3: Implement `snapshot` in `merge_review.rs`**

Resolve the source through `refs/heads/<branch>` with `rev-parse --verify --end-of-options`. Get the target from `default_branch(folder)`. Use `merge-base`, `rev-list --left-right --count` for ahead/behind, `log` for commits, and the per-file diff helper from `workspace_diff.rs` (extract it into a shared `pub(crate)` function that takes a base and a head commit; keep the existing `channel_changes` output byte-identical, and keep its tests green). Order of checks decides the `reason` (listed above): no branch, not a repository, unborn `HEAD`, default branch equals source, missing source ref, no target, no merge base. Every Git error becomes a `reason`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p discard-core merge_review workspace_diff`
Expected: PASS, including all existing `workspace_diff` tests.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-core
git commit -m "feat: read the merge review snapshot of a branch"
```

---

### Task 2: Decisions and state rules (core, store)

**Files:**
- Create: `crates/discard-core/src/review_state.rs`, `crates/discard-core/src/store/review.rs`
- Modify: `crates/discard-core/src/lib.rs` (`pub mod review_state;`), `crates/discard-core/src/store.rs` (`mod review;` and run the new `CREATE TABLE IF NOT EXISTS` where the other idempotent tables run, near `store.rs:632`)
- Test: unit tests in both new files.

**Interfaces:**
- Consumes: `crate::merge_review::ReviewSnapshot`, `crate::model::ChannelId`.
- Produces:

```rust
// review_state.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision { Approved, ChangesRequested }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub id: String, pub channel_id: String, pub decision: Decision,
    pub head: String, pub base: String, pub target: String,
    pub note: Option<String>, pub decided_at: i64, pub posted_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    NotReviewed, Approved, ApprovalOutOfDate, ChangesRequested, ChangesRequestedNewCommits, Merged, NothingToReview,
}

/// Merged and NothingToReview win. Then the newest row against the live head.
pub fn review_state(snapshot: &ReviewSnapshot, newest: Option<&ReviewDecision>) -> ReviewState;
pub const NOTE_LIMIT: usize; // equals the composer message cap; find it and reuse the constant

// store/review.rs
impl Store {
    pub fn add_review_decision(&self, row: &ReviewDecision) -> Result<ReviewDecision>; // returns the existing newest row when it is identical (decision, head, note): double-click rule
    pub fn review_decisions(&self, channel: &ChannelId) -> Result<Vec<ReviewDecision>>; // newest first
    pub fn mark_review_posted(&self, id: &str, at: i64) -> Result<()>;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
// review_state.rs
#[test] fn no_row_is_not_reviewed()
#[test] fn an_approval_at_the_live_head_is_approved_and_at_an_older_head_is_out_of_date()
#[test] fn changes_requested_at_the_live_head_and_at_an_older_head_differ()
#[test] fn merged_and_nothing_to_review_win_over_any_row()
// store/review.rs
#[test] fn decisions_append_and_list_newest_first()
#[test] fn an_identical_newest_decision_is_returned_instead_of_a_second_row()
#[test] fn a_changed_note_or_head_or_decision_makes_a_new_row()
#[test] fn mark_review_posted_sets_posted_at_once_and_an_unknown_id_errors()
#[test] fn the_table_survives_opening_the_store_twice()   // idempotent schema
#[test] fn rows_of_one_channel_do_not_show_in_another()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p discard-core review_state store::review`
Expected: FAIL.

- [ ] **Step 3: Implement the types, `review_state`, and the store methods**

Table is exactly the one in the spec (`review_decisions`, with `posted_at INTEGER`). `id` is a new UUID like other ids in the store. `decided_at` is unix seconds.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p discard-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-core
git commit -m "feat: store merge review decisions and derive the review state"
```

---

### Task 3: Backend commands (app)

**Files:**
- Create: `crates/discard-app/src/desktop/review.rs`
- Modify: `crates/discard-app/src/desktop/mod.rs` (add `mod review;`, register four commands near `commands::channel_changes`, `mod.rs:296`), `crates/discard-app/src/desktop/commands.rs` (four thin commands next to `channel_changes`, `commands.rs:842`), `crates/discard-app/src/desktop/dto.rs` if the DTO belongs there.
- Test: unit tests in `review.rs`; integration tests in `crates/discard-app/tests/` following `agents_conversations.rs`.

**Interfaces:**
- Consumes: Task 1 `snapshot`, Task 2 store methods and `review_state`, `terminal::cwd::for_channel(&store, &ChannelId) -> Result<(PathBuf, String), String>` (folder), `Store::channel_branch`, `AppCore::channel_busy_reason(&self, id: &str) -> Option<&'static str>`, `AppCore::send_to_agent(&self, channel: Option<&str>, text: String, recipient: Option<MessageAuthor>) -> Result<(), String>`.
- Produces:

```rust
#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct ReviewDto {
    pub snapshot: ReviewSnapshot,
    pub state: ReviewState,
    pub decisions: Vec<ReviewDecision>,      // newest first
    pub blocked: Option<String>,             // busy reason or "This channel is archived."; actions disabled when set
    pub unposted: Option<String>,            // id of a changes_requested row whose posted_at is NULL
}

// commands.rs (all async, work on spawn_blocking, read Git only)
#[tauri::command] pub async fn channel_review(channel_id: String) -> Result<ReviewDto, String>;
#[tauri::command] pub async fn review_decide(channel_id: String, decision: String, head: String, note: Option<String>) -> Result<ReviewDto, String>;
#[tauri::command] pub async fn review_retry_post(channel_id: String, decision_id: String) -> Result<ReviewDto, String>;
#[tauri::command] pub async fn review_request_summary(channel_id: String) -> Result<(), String>;
```

Behavior (spec "Decisions", "Request changes", "Approve", "Summary button"):

- `review_decide` order: parse `decision` ("approved" | "changes_requested") → trim note; Request changes needs a non-empty note within `NOTE_LIMIT` → refuse an archived channel and a busy channel (Request changes only; Approve is allowed while busy, because it posts nothing) → read the live snapshot and compare its `head` with the sent `head`; on a mismatch return `The branch changed. Reload the review.` → refuse when the state is `Merged`, `NothingToReview`, or the snapshot has a `reason` → write the row (`add_review_decision`) → for Request changes only, `send_to_agent(Some(channel), "Review of <head_short>: <note>")`, then `mark_review_posted`. A post failure leaves `posted_at` empty and returns the DTO with `unposted` set, plus the error text.
- `review_retry_post` posts the stored note with its original prefix (short id from the row's `head`), then sets `posted_at`. It refuses a row that is already posted or is not `changes_requested`, and it applies the busy rule.
- `review_request_summary` posts a fixed request (ask the agent to summarize the branch against the target with its git tools). It applies the busy and archived rules.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn review_dto_joins_snapshot_rows_state_and_busy_reason()
#[test] fn approve_writes_one_row_posts_nothing_and_is_allowed_while_busy()
#[test] fn request_changes_needs_a_note_and_a_free_channel_and_writes_no_row_when_refused()   // whitespace-only, over NOTE_LIMIT, busy
#[test] fn request_changes_posts_the_prefixed_note_once_and_sets_posted_at()
#[test] fn a_failed_post_keeps_the_row_unposted_and_retry_posts_the_stored_text_after_the_head_moved()
#[test] fn a_stale_head_is_refused_with_the_exact_copy_and_writes_no_row()
#[test] fn merged_nothing_to_review_and_reason_states_refuse_decisions()
#[test] fn an_archived_channel_refuses_every_action_with_its_reason()
#[test] fn two_identical_requests_leave_one_row()
#[test] fn the_summary_request_posts_once_and_obeys_the_busy_rule()
```

Test the logic as plain functions over `Store` plus a send closure (the Tauri commands stay thin and untested directly, as with `run_command`). The integration tests drive `Channels`/`AppCore` the way `agents_conversations.rs` does for the post.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p discard-app review`
Expected: FAIL.

- [ ] **Step 3: Implement `review.rs` and the four commands**

Pure functions take `&Store`, the folder, the busy reason, and a `send: impl FnOnce(String) -> Result<(), String>`. The commands resolve the folder with `terminal::cwd::for_channel`, run on `tauri::async_runtime::spawn_blocking`, and emit the existing conversation-changed event after a post.

- [ ] **Step 4: Run the tests and clippy**

Run: `cargo test -p discard-core -p discard-app && cargo clippy --workspace --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-app crates/discard-core
git commit -m "feat: add the merge review commands"
```

---

### Task 4: Web bridge and the tab body

**Files:**
- Create: `web/src/components/review/ReviewView.vue`, `ReviewActions.vue`, `ReviewCommits.vue`
- Modify: `web/src/bridge/types.ts` (`ReviewDto`, `ReviewSnapshotDto`, `ReviewDecisionDto`, `ReviewStateDto`), `web/src/bridge/api.ts` (`channelReview`, `reviewDecide`, `reviewRetryPost`, `reviewRequestSummary`)
- Test: `web/src/components/review/__tests__/ReviewView.test.ts` (mock `invoke` like `changes/__tests__/ChangesView.test.ts`)

**Interfaces:**
- Consumes: Task 3 command names and DTO shape (camelCase as above); `ChangesTree.vue`, `FileDiff.vue`, `SegmentedControl.vue`, `treeOrder`, `readChoice/saveChoice` (same preference keys pattern, separate key for the review layout).
- Produces:

```ts
export type ReviewStateDto = 'not_reviewed' | 'approved' | 'approval_out_of_date' | 'changes_requested'
  | 'changes_requested_new_commits' | 'merged' | 'nothing_to_review'
export const channelReview = (channelId: string) => invoke<ReviewDto>('channel_review', { channelId })
export const reviewDecide = (channelId: string, decision: 'approved' | 'changes_requested', head: string, note: string | null) =>
  invoke<ReviewDto>('review_decide', { channelId, decision, head, note })
export const reviewRetryPost = (channelId: string, decisionId: string) => invoke<ReviewDto>('review_retry_post', { channelId, decisionId })
export const reviewRequestSummary = (channelId: string) => invoke<void>('review_request_summary', { channelId })
// ReviewView props: { channelId: string; turnRunning: boolean }   emits: { state: [ReviewStateDto | null] } (for the tab badge)
```

- [ ] **Step 1: Write the failing tests**

```ts
test('loads the review when it mounts and shows source → target, counts, commits, files')
test('every state shows its label: not reviewed (no label), approved, approval out of date with both short ids, changes requested, changes requested · new commits since, merged')
test('nothing to review shows "Nothing to review: no commits beyond main" and no actions')
test('a reason shows an empty state with the reason and no actions')
test('the uncommitted notice shows the count and says those files are not reviewed')
test('approve calls reviewDecide with the pinned head and a null note')
test('request changes needs a non-empty note and sends it with the pinned head')
test('actions are disabled with the blocked reason shown, not hidden')
test('a head-changed error reloads the review and shows the message')
test('an unposted request shows its error and a Retry that calls reviewRetryPost')
test('the summary button is labelled "Ask for a summary" and calls reviewRequestSummary')
test('earlier decisions show in a short history list, newest first, with decision, short commit, and note')
test('refresh reloads, and a turn ending (turnRunning true → false) reloads')
test('emits the state for the tab badge')
test('files render through the same tree and diff components as the Changes tab')
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd web && vp test run src/components/review`
Expected: FAIL.

- [ ] **Step 3: Implement the DTO types, the wrappers, and the three components**

Layout top to bottom is in the spec "Surface": header, state line with controls, uncommitted notice, commits list, files and diffs. Reuse the large-diff constants and the split/inline choice from `ChangesView.vue`; export them from a shared module instead of copying them. Controls: Approve button, Request changes button that reveals a note field, Ask for a summary button, refresh button.

- [ ] **Step 4: Run web tests and typecheck**

Run: `cd web && vp test run && pnpm run typecheck && pnpm run lint`
Expected: PASS (the existing `TerminalPanel.test.ts` lint warning may remain).

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat: add the merge review view"
```

---

### Task 5: Enable the tab

**Files:**
- Modify: `web/src/App.vue` (the tab at `App.vue:1314-1315`, the content area next to `ChangesView` at `App.vue:1357`)
- Test: `web/src/components/__tests__/` or the existing `App` tab tests; add `MergeReviewTab.test.ts` beside the nearest existing App-level test.

**Interfaces:**
- Consumes: `ReviewView` (Task 4) with `:channel-id`, `:turn-running`, and its `state` event.
- Produces: tab value `review`, enabled, labelled `Merge review`, with a badge only for `approved`, `changes_requested`, `changes_requested_new_commits`, `approval_out_of_date`, `merged`.

- [ ] **Step 1: Write the failing tests**

```ts
test('the Merge review tab is enabled and has no Soon label and no TODO text')
test('opening the tab mounts ReviewView for the active channel')
test('the badge shows for approved, changes requested, out of date, and merged, and not for not reviewed')
test('threads and direct messages do not show the tab')
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd web && vp test run MergeReviewTab`
Expected: FAIL.

- [ ] **Step 3: Edit `App.vue`**

Remove `<!-- TODO: Implement merge review. -->` and `disabled` and ` · Soon`. Mount `<ReviewView v-else-if="conversationTab === 'review'" ...>`. Keep the tab inside the existing `v-if="store.snapshot.activeProjectId"` block. Keep the selected tab when the channel changes the way the other tabs do.

- [ ] **Step 4: Run the tests**

Run: `cd web && vp test run && pnpm run typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat: enable the merge review tab"
```

---

### Task 6: Docs and live check

**Files:**
- Modify: `docs/superpowers/specs/2026-11-07-plugin-dogfooding-design.md` (rows at lines 48 and 117: "Rust-rendered, no plugin surface"), `docs/worktrees.md` (a short "Merge review" section: read-only, approve is a record, no merge), `docs/ui-design-reference/README.md` (the Merge review row: implemented, with its production condition kept).

- [ ] **Step 1: Update the three docs**

State what is built and what is not (no merge, no gating).

- [ ] **Step 2: Run everything**

Run: `cargo test -p discard-core -p discard-app -p discard-providers -p discard-lua && cargo clippy --workspace --all-targets && cd web && vp test run && pnpm run typecheck`
Expected: PASS, no clippy warnings.

- [ ] **Step 3: Live check**

Follow the memory notes `feedback_app-check-clean-env.md` and `feedback_tauri-dev-hot-reload.md`: build `web/` (`pnpm run build`) and `cargo build -p discard-app`; copy the database to a temp `DISCARD_HOME` with `sqlite3 .backup`; launch with every `CLAUDE*`, `ANTHROPIC*`, `HEADROOM*` variable unset; drive the window with `cua-driver run_script`. Never send a plain message to an agent while driving. Check on a branch channel with commits: header and counts, files and diff, Approve then a new commit (out of date), Request changes (note appears in Conversation; Retry state after a forced failure if possible), the empty states for the default branch and a branchless channel. Report what was and was not exercised.

- [ ] **Step 4: Commit**

```bash
git add docs
git commit -m "docs: describe the merge review tab"
```
