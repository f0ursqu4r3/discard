# Renamed branch detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move a branch channel binding to the new name when Git recorded a rename made outside Discard.

**Architecture:** A pure Git helper in `discard-core` follows reflog "renamed" entries from an old name to a listed branch. `Core::refresh_branches` calls it for bound names that Git no longer lists, applies `rename_project_branch`, and posts one system note.

**Tech Stack:** Rust (`discard-core`, `discard-app`), real temp Git repositories in tests.

**Spec:** `docs/superpowers/specs/2026-11-24-renamed-branch-detection-design.md`

## Global Constraints

- Git runs through `git(dir, args)` / `git_cmd` in `crates/discard-core/src/worktree.rs` (argument list, hooks off).
- No move unless the result is exact. Never overwrite a live channel.
- Note text, exact: ``Branch renamed from `old` to `new`.``
- User-facing prose: short, direct sentences. No new UI.
- Run Rust tests with `CLAUDE*`/`ANTHROPIC*` env vars stripped.

## Review Focus

- A name deleted with no rename: binding stays, no note.
- Two stranded bindings whose chains reach one target: neither moves.
- A busy channel: skipped now, moved on a later refresh.
- The target name already has an active channel: nothing changes.
- A chain of renames (a→b→c): the binding moves once, to c.

---

### Task 1: Reflog rename chain

**Files:**
- Modify: `crates/discard-core/src/branch_actions.rs`
- Test: same file (`#[cfg(test)]`, real temp repo, as the existing tests there do)

**Interfaces:**
- Produces: `pub fn renamed_to(repo: &Path, old: &str, listed: &[String]) -> Option<String>`. `listed` holds the local branch names that exist now. It returns the listed branch that the reflog chain from `old` reaches, or `None`.

- [ ] **Step 1: Write failing tests** in `branch_actions.rs`: `renamed_to_follows_one_rename`, `renamed_to_follows_a_chain` (a→b→c returns `c` from `a`), `renamed_to_is_none_for_a_deleted_branch`, `renamed_to_is_none_when_two_branches_claim_the_old_name` (rename a→b, delete b, create a, rename a→c; from `a` the answer is ambiguous only if both b and c are listed — build that case and assert `None`).
- [ ] **Step 2: Run** `cargo test -p discard-core renamed_to` — Expected: FAIL (function missing).
- [ ] **Step 3: Implement.** For each name in `listed`, read `git reflog show --format=%gs <name>` and collect lines of the form `Branch: renamed refs/heads/X to refs/heads/Y` as edges X→Y. Follow edges from `old`. Return the end only if it is in `listed` and exactly one listed branch is reachable. Reflog errors for a branch mean no edges.
- [ ] **Step 4: Run** `cargo test -p discard-core renamed_to` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: follow branch renames in the reflog`.

### Task 2: Follow renames on refresh

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs` (`refresh_branches`, new private method)
- Test: `crates/discard-app/src/desktop/channels.rs` test module (it already builds real repos with branch channels; reuse its helpers) or a new `crates/discard-app/tests/` file if cleaner

**Interfaces:**
- Consumes: `renamed_to` (Task 1); `Store::rename_project_branch(project, channel, new)`; `Core::channel_busy_reason(id)`; `branch_source.store` as `branch_hint` uses it.
- Produces: `Core::follow_renamed_branches(&self, view, project: &ProjectId, repository: &str, repo_path: &Path, listed: &[String])`, called by `refresh_branches` once per repository, before it builds the rows for that repository.

- [ ] **Step 1: Write failing tests** (each uses a real repo, a bound branch channel, then `git branch -m` outside Discard, then `refresh_branches`):
  - `outside_rename_moves_the_binding_and_posts_the_note` — channel id unchanged, `project_repository_branch_channel(project, repo, new)` returns it, history ends with ``Branch renamed from `old` to `new`.``
  - `outside_rename_chain_moves_once`
  - `deleted_branch_keeps_the_binding_and_posts_nothing`
  - `busy_channel_is_skipped_then_moved_on_a_later_refresh`
  - `rename_onto_a_name_with_an_active_channel_changes_nothing`
  - `two_stranded_bindings_reaching_one_target_do_not_move`
- [ ] **Step 2: Run** `cargo test -p discard-app outside_rename` — Expected: FAIL.
- [ ] **Step 3: Implement `follow_renamed_branches`.** Stranded = active branch channels of the project and repository whose bound name is not in `listed` and not detached. Candidates for each = `listed` names with no active channel. Skip a channel when `channel_busy_reason` is `Some`. Compute `renamed_to` per stranded channel; drop any pair whose target another stranded channel also reaches. Apply `rename_project_branch`, post the note (find how the "Moved to" link is posted in `channels.rs:~938` and reuse that storage call), and log nothing else. Errors skip that channel.
- [ ] **Step 4: Run** `cargo test -p discard-app outside_rename` — Expected: PASS. Then the full `cargo test` with env stripped — Expected: all pass.
- [ ] **Step 5: Commit** `feat: follow branches renamed outside Discard`.

### Task 3: Docs

**Files:**
- Modify: `docs/project-channel-status.md` (line 21 "Detection … remain pending"), `docs/worktrees.md` (branch list or rename section)

- [ ] **Step 1:** Replace the "pending" wording: Discard follows a rename that Git recorded in the reflog, posts a note, and leaves the binding alone when the result is not exact. Branch retirement stays pending.
- [ ] **Step 2: Commit** `docs: renamed branch detection`.
