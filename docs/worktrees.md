# Worktrees

Coding sessions that use a worktree run outside the project checkout, under `$DISCARD_HOME/worktrees/` (default
`~/.discard/worktrees/`). This keeps those sessions from overwriting the project checkout.

## Rules

- **Created before a Coding session starts.** When a channel uses a worktree, Discard creates it before starting
  the provider session. A running session is never moved to another directory.
- **Serialized per repository.** Parallel channels create their worktrees one at a time, so git's locks do not
  collide.
- **Branches from `HEAD`.** Each worktree gets a branch `discard/<channel>-<id>` that starts at the repository's
  current `HEAD`. Uncommitted and untracked files in the main checkout are **not** copied. Discard shows a notice
  that lists them.
- **No hooks.** Discard turns off the repository's git hooks for its own worktree commands.
- **Failure stops the turn.** If the worktree cannot be created (no commits, not a repository, a git error), the
  turn does not start and the error appears in the channel. A channel set to use a worktree never falls back to
  the main checkout on its own.
- **Idempotent.** Asking again returns the existing worktree. A worktree directory deleted by hand is recreated
  on the next turn, and the kept branch is reused.

## Choosing the mode

- New channels in a git repository use a worktree by default. Other projects use the main checkout.
- `discard --worktree` or `discard --main-checkout` chooses explicitly.
- Channels from before worktree support stay in the main checkout. The default never moves them.
- Changing an existing channel's mode starts a **new session**, because the provider's old session belongs to the
  old directory. The new session does not carry over the old conversation. A "confirm a summary first" screen is
  not built yet.
- Project channels are durable conversations associated with `general` or a canonical branch. Opening a project
  or branch channel restores its conversation; it does not switch the Git checkout or create a worktree. Normal
  Conversation mode still uses an empty provider working directory. Worktree creation and branch mutation remain
  separate operations.

## Branch list

The channel list shows local branches in a two-level tree. The main checkout and other worktrees are top-level
rows. A branch without a worktree appears below the worktree whose branch tip is its closest ancestor. If no
worktree branch is an ancestor, it appears below the main checkout. Ties go to the main checkout, then the
worktree branch with the first name alphabetically.

A filled marker shows a row with a worktree. Rows without a worktree are dimmed. The current channel's worktree
row is highlighted, and the main checkout has a `main` tag. Click a parent row to collapse or expand its
branches. Click a branch row to open its canonical project conversation. If it has no canonical conversation,
Discard creates one. Opening the conversation does not switch the Git checkout or create a worktree.

## Archiving

`GitWorkspace::archive` removes a channel's worktree only if all of these are true: no uncommitted changes, no
untracked files, no ignored files that hold data. Otherwise it returns what would be lost and changes nothing. A
confirmed call (`force`) removes the directory anyway. **The branch is always kept**, so committed work stays in
git. Commits that no other branch has are reported but do not block removal, because they stay on the kept branch.

There is no button for archiving yet. The function and its tests exist in `discard-core`.

## Limits you should know

- Worktrees are a git feature. A project that is not a git repository uses the main checkout.
- Submodules and sparse checkouts are not handled specially.
- Discard does not merge or delete the worktree branches. Use git for that.
