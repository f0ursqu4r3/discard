# Branch tree in the channel list

## Intent

The user wants one place to see every branch and worktree of a project, to see which ones are checked out and
workable, to switch between them, and to work on them. The channel list is that place.

Today the channel list has a flat, read-only BRANCHES section (commit `7070afe`). Each row is tagged `main` or
`worktree`. This design replaces that section with a tree whose rows are actions.

## Scope split

The app runs one channel per process (`bootstrap::ensure_workspace`). It cannot switch channels or create one at
runtime. "New Channel…" and "Switch Channel…" are placeholders. Goals 3 to 5 need a multi-channel runtime. That is
a separate design.

- **Phase 1 (planned now):** goals 1 and 2. The tree shows nesting, which rows have a worktree, and which row is
  the current channel. Rows start no actions. The only click is collapsing a parent row.
- **Phase 2 (own design, later):** goals 3 to 5. These decisions below belong to phase 2: "Worktree row =
  channel", "Branch already checked out elsewhere", "Worktree with no Discard channel", "Existing branch, no
  worktree", "New channel", and "Worktree creation timing". They are recorded now and built later.

## Goals

1. See all local branches and worktrees in one tree.
2. See at a glance which branches are checked out (have a worktree) and which are not.
3. Switch to a branch with one click.
4. Work on any branch directly, in its own worktree.
5. Create a new channel. This creates a new branch and worktree.

## Non-goals

- No visual groups or section headers inside the tree. It is one list.
- Discard does not merge, rebase, or delete branches. Git does that (see `docs/worktrees.md`).
- No change to how a channel's provider session works. A running session never moves to another directory.
- No support for remote branches.

## Anti-goals

- A click must never create a second worktree for a branch that already has one. Git does not allow one branch in
  two worktrees.
- A click must never create a new branch when the user picked an existing one.
- A failed worktree creation must never fall back to the main checkout silently (existing rule in
  `docs/worktrees.md`).

## Decisions

These come from the user.

- **Tree shape.** A worktree is a row. Its checked-out branch names the row. Branches nest under the worktree
  whose checked-out branch is their closest ancestor. The builder finds the ancestor with `git merge-base`.
- **No ancestor.** A branch with no matching ancestor goes under `main`.
- **Main checkout.** The main checkout is the top row. The user can work in it directly.
- **Worktree row = channel.** A row with a worktree is a channel. Clicking it switches to that channel.
- **Active vs inactive.** Rows with a worktree look active and show a marker. Rows without a worktree look dimmed.
- **Existing branch, no worktree.** Clicking it creates a worktree for that existing branch and opens a new
  channel in it. No new branch is created.
- **New channel.** "New channel" keeps today's behavior. It creates a `discard/<channel>-<id>` branch from the
  `HEAD` of the main checkout, creates its worktree, and opens a channel in it.
- **Branch already checked out elsewhere.** Clicking it opens the channel of that worktree.
- **Worktree with no Discard channel.** This covers the main checkout and worktrees made with
  `git worktree add`. A click opens the most recent channel bound to that worktree. If none exists, the click
  creates a channel bound to that worktree.
- **Tree and channel list.** The tree replaces only the BRANCHES section. The channel list above it stays as it
  is. A channel can show in both places.

These are the author's calls. The user approved them with "lgtm" but did not discuss them in detail.

- Worktree rows collapse.
- The tree stays read-only in one respect: Discard does not rename or delete branches from it.
- Existing `discard/<channel>-<id>` branches appear in the tree like any other branch.
- **Nesting rule.** Branch B nests under worktree branch W only if W's tip is an ancestor of B. The closest W is
  the one with the fewest commits between W and B. If no W qualifies, B goes under `main`. Ties go to the builder.
- **Two levels only.** The main checkout row comes first. Worktree rows follow it. These are depth-0 siblings.
  Branches without a worktree are depth-1 children of a worktree row or of the main checkout row. A branch that
  has a worktree is always a depth-0 row, never a child. The main checkout competes as a parent like any other
  worktree. On a tie it wins, then the worktree branch name decides alphabetically.
- **Odd worktrees.** A detached HEAD shows its short SHA as the row name. A bare worktree is not listed. A
  prunable worktree (directory deleted) shows as a branch with no worktree, so a click recreates it.
- **Worktree creation timing.** Clicking a branch with no worktree creates the channel at once. The worktree is
  created on that channel's first turn, per the existing rule in `docs/worktrees.md`. The row looks inactive
  until then.
- **Active marker.** "Active" means the worktree exists on disk. The current channel's row gets a separate
  highlight.

## Constraints

- Branch listing runs off the render path. It loads at startup and when a turn ends (existing behavior in
  `crates/discard-ui/src/view.rs`). Keep it that way.
- Tree building is a pure function in `discard-core` with unit tests. The UI only renders it.
- Worktree creation stays serialized per repository and keeps git hooks off (`docs/worktrees.md`).
- Update `docs/worktrees.md`. Its "Branch list" section says the list is read-only. That becomes false.

## Left to the builder

- Exact visuals: indent width, marker shape, dimming colors, collapse control.
- How a channel binds to an existing branch's worktree. Today a channel creates its own worktree. The builder
  chooses the smallest change to the channel model.
- What happens to a channel's worktree when the user archives it, if the branch was not created by Discard.
- Keyboard access to the tree.
- Tie-breaks when two worktree branches are equally close ancestors of a branch.

## Process

The user wants Codex to implement the plan. Claude reviews the diff after. Use the `codex-build` skill for the
build step.
