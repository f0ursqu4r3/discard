# Branch Tree in the Channel List Implementation Plan (Phase 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The user chose Codex for the build (`codex-build`), with Claude reviewing the diff after.

**Goal:** Replace the flat BRANCHES list with a two-level tree that shows nesting, which rows have a worktree, and which row is the current channel.

**Architecture:** A pure function in `discard-core` turns branch entries plus ancestry distances into ordered tree rows. A git-backed function gathers the distances. The UI renders the rows and keeps its existing off-render-path loading.

**Tech Stack:** Rust 2024, GPUI (`discard-ui`), plain `git` through the `git()` helper in `worktree.rs`.

**Spec:** `docs/superpowers/specs/2026-11-04-branch-tree-channel-list-design.md` (Phase 1 only).

## Global Constraints

- Phase 1 only. Rows start no actions. The only click collapses a parent row.
- No visual groups or headers inside the tree. It is one list under the existing "BRANCHES" label.
- Tree building is a pure function in `discard-core` with unit tests. The UI only renders it.
- Branch listing runs off the render path, at startup and when a turn ends. Keep `refresh_branches` as it is.
- Two levels only. Depth-0 rows: the main checkout, then worktrees. Depth-1 rows: branches without a worktree.
- A branch nests under a worktree branch W only if W's tip is an ancestor of it. Closest W = fewest commits between W and the branch. Ties go to the main checkout, then to the worktree branch name alphabetically. No qualifying W: the main checkout.
- A detached HEAD worktree shows its short SHA (7 characters) as the row name. A bare worktree is not listed. A prunable worktree counts as a branch with no worktree.
- Git runs with hooks off. Use the existing `git()` helper in `crates/discard-core/src/worktree.rs`.
- Follow the existing compact code style. Do not run `cargo fmt` on files you do not change.
- Do not commit. The user approves the diff first. Add no AI attribution anywhere.

## Review Focus

- A worktree on a detached HEAD: shows a short SHA row, never panics (Task 1, Task 2).
- A prunable worktree (directory deleted by hand): its branch appears with no worktree (Task 1).
- A branch with equal distance to two worktree branches: the same parent every refresh (Task 2).
- A bare repository (first porcelain block is bare): no main row, no panic, branches still listed (Task 1, Task 2).
- A repository with no commits or no local branches: empty tree, section hidden (Task 3).

## File Structure

- Modify `crates/discard-core/src/worktree.rs`: `BranchEntry`, `parse_branches`, new `list_branch_tree`.
- Create `crates/discard-core/src/branch_tree.rs`: `TreeRow`, `build_tree`, `visible_rows`, `current_row`.
- Modify `crates/discard-core/src/lib.rs`: add `pub mod branch_tree;`.
- Modify `crates/discard-ui/src/view.rs`: render the tree.
- Modify `docs/worktrees.md`: the "Branch list" section.

---

### Task 1: Parse detached and prunable worktrees

**Files:**
- Modify: `crates/discard-core/src/worktree.rs` (`BranchEntry`, `parse_branches`, its tests)

**Interfaces:**
- Produces: `BranchEntry { branch: String, worktree: Option<PathBuf>, is_main_checkout: bool, detached: bool, is_discard: bool }`. The `is_current` field is removed. The view stops using it (Task 4). `parse_branches(worktree_porcelain: &str, branch_refs: &str) -> Vec<BranchEntry>` keeps its signature.

- [ ] **Step 1: Write failing tests** in the `tests` module of `worktree.rs`:
  - `parser_lists_a_detached_worktree_by_short_sha`: porcelain `"worktree /repo\nHEAD aaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree /d\nHEAD abcdef1234567890\ndetached\n"`, refs `"main\n"`. Assert the entries include `BranchEntry { branch: "abcdef1".into(), worktree: Some("/d".into()), is_main_checkout: false, detached: true, is_discard: false }`.
  - `parser_treats_a_prunable_worktree_as_no_worktree`: porcelain with a block `"worktree /gone\nHEAD abc\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n"` after the main block. Assert the `gone` entry has `worktree == None`.
  - `parser_skips_a_detached_block_with_no_head_line`: block `"worktree /x\ndetached\n"` produces no entry.
  - `parser_never_marks_a_bare_first_block_as_main`: porcelain `"worktree /bare\nbare\n\nworktree /w\nbranch refs/heads/w\n"`, refs `"w\n"`. Assert `is_main_checkout == false` for `w`.
- [ ] **Step 2: Update the three existing branch tests** to the new struct (drop `is_current`, add `detached: false`). Run `cargo test -p discard-core worktree::` and confirm the new tests fail to compile or fail.
- [ ] **Step 3: Change `BranchEntry` and `parse_branches`.** Record `detached` blocks that have a `HEAD ` line as entries named by the first 7 characters of that SHA. Ignore a `prunable` block's checkout. Keep the sort (main, worktrees, then others, each by name) and the dedup.
- [ ] **Step 4: Run** `cargo test -p discard-core worktree::`. Expected: all pass.

### Task 2: Pure tree builder

**Files:**
- Create: `crates/discard-core/src/branch_tree.rs`
- Modify: `crates/discard-core/src/lib.rs`

**Interfaces:**
- Consumes: `BranchEntry` from Task 1.
- Produces, all `pub` in `discard_core::branch_tree`:
  - `enum RowKind { Main, Worktree, Branch }` (derive `Debug, Clone, Copy, PartialEq, Eq`)
  - `struct TreeRow { pub name: String, pub kind: RowKind, pub worktree: Option<PathBuf>, pub detached: bool, pub is_discard: bool, pub parent: Option<String> }` (derive `Debug, Clone, PartialEq, Eq`), with `fn depth(&self) -> u8` (1 if `parent.is_some()`, else 0).
  - `fn build_tree(entries: &[BranchEntry], distance: &dyn Fn(&str, &str) -> Option<usize>) -> Vec<TreeRow>`. `distance(w, b)` is `Some(n)` when the tip of `w` is an ancestor of `b` with `n` commits between them, else `None`.
  - `fn visible_rows<'a>(rows: &'a [TreeRow], collapsed: &HashSet<String>) -> Vec<&'a TreeRow>`: hides rows whose `parent` is in `collapsed`.
  - `fn current_row(rows: &[TreeRow], cwd: &Path) -> Option<usize>`: the index of the depth-0 row whose `worktree` is the longest path prefix of `cwd`.

- [ ] **Step 1: Write failing tests** in `branch_tree.rs`. Build entries with a small helper. Use a `distance` closure over a table of `(w, b) -> n`.
  - `main_row_comes_first_then_worktree_rows_each_followed_by_their_branches`: order is `main`, main's children, then each worktree row (by name) with its children (by name).
  - `a_branch_nests_under_the_closest_worktree_branch`: `distance(w1, b) = 3`, `distance(w2, b) = 1` gives parent `w2`.
  - `a_branch_with_no_ancestor_worktree_goes_under_main`: all `None` gives parent `main`'s name.
  - `ties_go_to_main_then_alphabetical`: equal distance to main and `w1` gives main. Equal distance to `w2` and `w1` (main not a candidate) gives `w1`. Run twice with reversed entry order and assert the same result.
  - `a_branch_with_a_worktree_is_never_a_child`: every entry with a worktree has `parent == None`.
  - `without_a_main_row_branches_have_no_parent`: entries with no `is_main_checkout` give rows whose `parent` is `None`, and nothing panics.
  - `a_detached_worktree_row_keeps_its_short_sha_name_and_flag`.
  - `visible_rows_hides_children_of_collapsed_parents`.
  - `current_row_picks_the_longest_worktree_prefix`: worktrees `/repo` and `/repo/.wt/x`, `cwd = /repo/.wt/x/sub` gives the `x` row.
  - `current_row_is_none_outside_every_worktree`.
- [ ] **Step 2: Run** `cargo test -p discard-core branch_tree`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the types and functions above in `branch_tree.rs`. Add `pub mod branch_tree;` to `lib.rs`. The candidate parents of a branch are all entries that have a worktree, including the main checkout.
- [ ] **Step 4: Run** `cargo test -p discard-core branch_tree`. Expected: PASS.

### Task 3: Git-backed `list_branch_tree`

**Files:**
- Modify: `crates/discard-core/src/worktree.rs`

**Interfaces:**
- Consumes: `parse_branches`, `build_tree`, `TreeRow`.
- Produces: `pub fn list_branch_tree(repo: &Path) -> Result<Vec<TreeRow>>`. It replaces `list_branches` as the UI entry point. Keep `list_branches`; the new function calls it.

- [ ] **Step 1: Write failing tests** in the `tests` module (use the existing `rig()` and `git()` helpers):
  - `list_branch_tree_nests_by_ancestry_in_a_real_repository`: add worktree `feature/one`, commit once on it, create branch `feature/two` from `feature/one` with one more commit (no worktree), create branch `old` at the first commit of `main`. Assert the rows in order are `main` (Main), `old` (Branch, parent `main`), `feature/one` (Worktree), `feature/two` (Branch, parent `feature/one`).
  - `list_branch_tree_is_empty_for_a_repository_with_no_commits`: `git init` only. Assert `Ok` and an empty vector.
  - `list_branch_tree_reports_a_non_repository`: assert `Err(WorktreeError::NotARepo(_))`.
- [ ] **Step 2: Run** `cargo test -p discard-core list_branch_tree`. Expected: FAIL.
- [ ] **Step 3: Implement `list_branch_tree`.** For each entry with a worktree, run `git for-each-ref --contains <branch-or-sha> --format=%(refname:short) refs/heads`. For each branch in that output, run `git rev-list --count <w>..<b>`. Store the counts in a `HashMap<(String, String), usize>`. Pass a closure over the map to `build_tree`. Treat a failed git call for one pair as `None`.
- [ ] **Step 4: Run** `cargo test -p discard-core`. Expected: PASS.

### Task 4: Render the tree and update the docs

**Files:**
- Modify: `crates/discard-ui/src/view.rs` (fields `branches`, `set_branches`, `refresh_branches`, the spawn in `new`, the `branch-list` render block)
- Modify: `docs/worktrees.md` (the "Branch list" section)

**Interfaces:**
- Consumes: `list_branch_tree`, `TreeRow`, `visible_rows`, `current_row`.

- [ ] **Step 1: Switch the loaders.** Change both background spawns and `set_branches` to use `list_branch_tree` and store `Vec<TreeRow>` in `branch_rows`. Keep the generation counter. Remove the use of `BranchEntry` from the view.
- [ ] **Step 2: Compute the current row.** After each load, compute the channel's working directory: the worktree record path (`store.worktree_for`) when the channel uses a worktree and has one, the project path when it uses the main checkout, otherwise none. Store `current_row(&rows, &cwd)` as `current_branch_row: Option<usize>`.
- [ ] **Step 3: Render.** Rows come from `visible_rows` with a `collapsed: HashSet<String>` field. Indent depth-1 rows. Show a filled marker on rows that have a worktree. Dim rows without one. Highlight the current row with the same style as the selected channel row. Show a `main` tag on the main row. A click on a row that has children toggles its name in `collapsed`. A click on any other row does nothing. Keep the ellipsis on long names.
- [ ] **Step 4: Update `docs/worktrees.md`.** Rewrite the "Branch list" section: the list is a tree, the nesting rule, the markers, and that rows start no actions yet.
- [ ] **Step 5: Verify.** Run `cargo build -p discard-ui` and `cargo test -p discard-ui -p discard-core`. Expected: no errors, all tests pass.
- [ ] **Step 6: Manual check.** Launch the app on a repo that has one extra worktree and one branch without a worktree. Expected: the tree matches `git worktree list` and `git branch`, the current channel's row is highlighted, and clicking a parent row collapses it.
