# Merge review design

Status: approved and implemented. Date: 2026-11-19.

## Intent

A branch channel is a conversation where an agent works on one Git branch in a worktree. Today the flow stops at the Changes tab. The user cannot say "this is ready" or "this needs more work" inside Discard, and the fourth tab is a disabled `Merge review · Soon` placeholder.

Merge review turns that tab into the last step of the flow. The user opens it on a finished branch, reads what the branch adds to the default branch, and records one of two decisions: **Approve** or **Request changes**. Request changes sends the user's note back to the agent in the same channel, so the work continues.

The tab reviews. It does not merge. `docs/ui-design-reference/README.md` says: "Keep review approval separate from performing a Git merge." `docs/worktrees.md` says Discard does not merge worktree branches. Both stay true.

## Goals

- A branch channel shows **source branch → target branch**, ahead and behind counts, the commits the branch adds, and the changed files with diffs.
- The user can record **Approve** or **Request changes** (with a required note). Decisions are stored and visible after a restart.
- An approval names the commit it covers. New commits make it stale, and the tab says so.
- Request changes posts the note into the channel conversation as a user message.
- The tab shows when the branch is already merged into the target.
- Every unavailable state shows an honest reason, not a fake merge request.

## Non-goals

- Running `git merge`, `rebase`, `push`, or deleting branches. (Option C of the scope discussion. A later design.)
- Gating anything on approval. Approval is a record and a label in this version.
- A stored reviewer summary or a summary the tab renders itself. The tab has a button that asks the channel's agent to write one in the conversation (see Decisions I made).
- Pull requests, remote hosting, or any network call.
- Plugin slots or Lua control of this tab. It is a Rust-rendered surface. Update the "Merge review tab" row in `2026-11-07-plugin-dogfooding-design.md` from "Planned (`Soon`). Out of scope" to "Rust-rendered, no plugin surface".

## Anti-goals

- A review view that looks real but shows fixture or invented data.
- An approval that stays green after the branch moved.
- A button that changes the repository. All Git calls in this feature are read-only, bounded, and run with `--no-ext-diff --no-textconv`, like `workspace_diff.rs`.

## Constraints

- `AGENTS.md`: remove the `TODO` comment and the `Soon` label from the tab when it works. A channel that cannot be reviewed shows the tab with an empty state that gives the reason. That is a state block, not a planned item, so it needs no `Soon` and no `TODO`.
- Web tests go in a `__tests__` directory beside the code they test.
- Reuse existing parts: `workspace_diff.rs` (bounded, read-only Git), `project_context/git.rs` (`default_branch`, `git_compare` logic, `resolve`, `counts`), and the Changes tab components (`FileDiff`, `ChangesTree`, `SegmentedControl`, `diffRows`).
- Store schema changes are additive and idempotent (`CREATE TABLE IF NOT EXISTS`), like the neighbouring tables in `store.rs`.
- TDD. Core logic is tested with temporary Git repositories, as in `project_context/git_tests.rs`.

## Approaches considered

- **A. Read-only review.** Commits, files, diffs, no actions. Smallest and safe. It does not finish the workflow, because the user still decides outside Discard.
- **B. Review with decisions. (Chosen by the user.)** A plus Approve, Request changes, and a stale-approval rule. No Git write. It makes the tab the end of the agent workflow and follows the repository's "review is not merge" rule.
- **C. Review plus merge.** B plus a Merge button. It needs conflict handling, rules for a dirty main checkout, and approval gating. It would be Discard's first write to the project's own branches. Rejected for now. It builds on B, so B does not block it.

## Design

### What the tab reviews

- **Source** is the channel's bound branch (`Store::channel_branch`).
- **Target** is the project's default branch (`default_branch` in `project_context/git.rs`: local origin HEAD, then `main`, `master`, then the main checkout's branch).
- The review is of **committed work**: commits on source that are not on target, compared from the merge base (three-dot diff, as in `git_compare`). Uncommitted changes in the worktree are not part of the review. If the worktree has uncommitted or untracked files, the tab shows a notice with the count and says they are not reviewed.
- The tab pins each snapshot to the source commit it read (`head`) and the merge base (`base`).

### Review snapshot (core, read-only)

A new function in `discard-core` (module `merge_review.rs`) takes the repository path, source branch, and target branch and returns:

- `source`, `target`, `head` (short and full id), `base` (merge base, short and full id). `head` is the commit of the source **branch ref** (`refs/heads/<branch>`), not the worktree's checked-out commit. Branch reads run against the repository path. The `dirty` count runs against the channel's worktree.
- `ahead`, `behind`
- `nothing_to_review`: `ahead` is 0. The tab shows "Nothing to review: no commits beyond <target>" and no actions. This also covers a fresh branch and a branch whose tip equals the target.
- `merged`: `ahead` is 0 **and** the newest stored decision's `head` is an ancestor of the target. Git cannot tell a merged branch from a fresh one by itself, so Discard uses its own record. A branch that was never reviewed and was merged outside Discard shows `Nothing to review`.
- `commits`: id, subject, author, time, newest first, bounded
- `files`: path, status, added, removed, diff, truncated (same shape as `ChangeFile`), bounded
- `dirty`: number of uncommitted and untracked paths in the worktree
- `reason`: set when no review is possible, with `files`/`commits` empty

`reason` values (plain sentences, shown in the empty state): channel has no branch; channel is on the default branch; source branch no longer exists; target branch cannot be determined; no merge base; the repository is unavailable. Git errors become a `reason`, never a panic or a raw error dialog.

### Decisions (store)

New table, append-only:

```
review_decisions (
  id TEXT PRIMARY KEY,
  channel_id TEXT NOT NULL REFERENCES channels(id),
  decision TEXT NOT NULL,        -- 'approved' | 'changes_requested'
  head TEXT NOT NULL,            -- full source commit the user reviewed
  base TEXT NOT NULL,            -- full merge base at that time
  target TEXT NOT NULL,          -- target branch name at that time
  note TEXT,                     -- required for changes_requested, optional for approved
  decided_at INTEGER NOT NULL,
  posted_at INTEGER              -- changes_requested only: when the note reached the conversation; NULL until then
)
```

- The **current state** of a channel is its newest row, compared with the live snapshot:
  - no row → `Not reviewed`
  - `approved` and row `head` = live `head` → `Approved`
  - `approved` and row `head` ≠ live `head` → `Approval out of date` (shows the old and new short ids)
  - `changes_requested` and row `head` = live `head` → `Changes requested`
  - `changes_requested` and row `head` ≠ live `head` → `Changes requested · new commits since` (the agent has pushed work, the user should look again)
- The tab also shows `Merged` when `merged` is true, whatever the last decision. A merged branch and a branch with nothing to review hide both actions.
- The user can record a new decision at any time. Older rows stay as history, shown in a list of all decisions, newest first.
- The backend checks that the `head` the UI sends still equals the live branch-ref head when it records the decision. It reads the ref again at that moment. If it moved, the call fails with "The branch changed. Reload the review." and the UI refreshes. This stops approving a diff the user did not see. A decision needs this check whether or not the note is optional.

### Request changes

- The note is required and is capped (same cap as a normal message).
- The backend records the decision row, then posts the note into the channel conversation through the existing send path, as a user message, with a short fixed prefix naming the reviewed commit (for example `Review of abc1234: ` followed by the note).
- The busy check runs **before** the row is written, so a refused request leaves no row. It is refused while the channel is busy (a turn runs, a sub-agent runs, an approval waits, or an archive runs), with the same busy rule the branch actions use. The button is disabled with a reason instead of hidden.
- The backend calls the same Rust path that sends a user message from the composer (`AppCore::send_to_agent`), so the whole flow runs in one command.
- If posting fails after the row is written, the row keeps `posted_at` empty. The tab shows the error and a Retry on that row. Retry posts the stored note and its original `Review of <commit>:` prefix, even if the head has moved since, then sets `posted_at`. A restart keeps the Retry, because the state is in the row. No second row is written.

### Approve

- Writes the decision row. Does nothing else. No Git call writes. No message is posted.
- Approval has no effect on any other feature in this version.

### Summary button

- The tab has an **Ask for a summary** button. It posts a fixed request into the conversation: ask the channel's agent to summarize the branch against the target (the agent already has `project_git_compare`, `project_git_log` and read tools). It is disabled under the same busy rule.
- The summary appears in the Conversation tab, not in the review tab.

### Surface

- Threads have no branch and no tab bar, so they do not show the tab. An archived channel shows the tab read-only: no actions, with the reason `This channel is archived.`
- Tab: always shown for project channels, enabled. Label `Merge review`, with a small state badge (`Approved`, `Changes requested`, `Out of date`, `Merged`) when there is one. No badge for `Not reviewed`.
- Layout, top to bottom: header (source → target, ahead/behind), state line with the decision controls and the `Merged` marker, uncommitted-work notice if any, commits list, files and diffs.
- Files and diffs reuse the Changes tab components, with the same tree, split/inline choice, large-diff rules (`MANY_FILES`, `LARGE_DIFF`, `OPEN_FILES`), and safe diff rendering.
- Refresh: a refresh button, and an automatic refresh when the channel's turn ends and when the tab opens.
- Unavailable (`reason` set): an empty state with the reason and no actions.
- Backend access: two Tauri commands, `channel_review` (read the snapshot plus the channel's decisions) and `review_decide` (record a decision, and post the note for Request changes), plus one for the summary request. The frontend wrappers go in `web/src/bridge/api.ts` with DTOs in `types.ts`.

## Decisions I made

These were not discussed. Mark any you want changed.

1. **Target is the default branch, with no picker.** A target picker is easy to add later.
2. **Review covers committed work only.** Uncommitted work is shown as a notice, not reviewed.
3. **Approval is a stored label tied to a commit.** It gates nothing in this version.
4. **Request changes posts a user message.** The decision row is the source of truth, and the message is the delivery.
5. **The summary is a button that asks the agent in the conversation.** The tab does not render a summary itself.
6. **The tab is always visible** for project channels and shows a reason when review is not possible.
7. **History is kept** as append-only rows.
8. **`Merged` comes from Discard's own record** (see the snapshot), because Git cannot tell merged from fresh.
9. **A failed note post is kept in the row** (`posted_at` empty) and survives a restart.
10. **`head` is the branch ref.** Threads show no tab. An archived channel shows it read-only.

## Left to the builder

- Module and function names, file layout, and how much of `project_context/git.rs` becomes shared helpers.
- Exact bounds (commit count, file count, diff bytes, timeouts). Follow `workspace_diff.rs`.
- Exact empty-state wording, within "plain sentence that names the cause".
- Component split in `web/src/components/review/`.
- Whether `Update` events refresh the tab or it polls on turn end.

## Tests

- Core, with temporary Git repositories: normal branch with commits; no commits ahead (`Nothing to review`, not `Merged`); branch behind the target; merged branch (stored decision whose head is on the target); default branch; missing source; no merge base; dirty worktree; truncation; the file list uses rename detection (`-M`).
- Store: decision rows append; current-state rules for each of the five states; stale head; a rejected decision when the head moved.
- Request changes: the row is written once; a busy channel is refused and leaves no row; the post failure keeps `posted_at` empty and allows retry after a restart; Retry posts the stored text even when the head moved.
- Web (in `__tests__`): each state renders; actions disabled when busy; stale approval message; decision flow calls the bridge with the pinned head; empty state shows its reason; diff reuse renders.
- A live check in the app with a temp `DISCARD_HOME`, as done for the slash commands.
