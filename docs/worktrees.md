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

Detached worktrees are selectable. Each detached worktree opens a saved conversation bound to its repository,
path, and displayed commit. Two worktrees at the same commit have separate conversations. Opening a detached
worktree does not create a branch or change the checkout. Discard borrows the existing directory. If the checkout
changes, a stale row cannot open it and Coding workspace validation rejects the old binding.

Branch, worktree, and channel rows have custom context menus. Right-click a row, or press Shift+F10 while it has
focus. Menus open the selected row and copy its name or ID. Worktree menus also copy the directory path. Planned
management actions stay disabled and show **Soon**. Unread markers on detached worktrees match the saved path
and commit, so worktrees at the same commit do not share an unread marker.

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

## Conversation worktree tools

Project conversations with a saved worktree binding expose `worktree_inspect` and `worktree_request_removal` through the scoped Discard MCP server. The tools use the token's conversation and project. They accept no path, shell command, force flag, or approval argument. Direct chats and summary tokens cannot use these tools.

Inspection reads status (including ignored and untracked paths), tracked working changes, or the current commit patch. Each Git command has a ten-second limit and a 32 KiB output limit. Incomplete inspection fails closed. Hooks, external diff programs, text conversion, and interactive Git prompts are disabled.

Removal creates an app approval card. The request does not remove files. Only the user can approve or deny it in the app. Approval is single-use, expires after ten minutes, and is cancelled by an app restart. Pending cards are session state; completed decisions are saved as system notes in the conversation.

Before removal, Discard waits for app responses and archive operations to finish, resolves the saved binding again, and compares the current Git state with the approved snapshot. Main checkouts, locked worktrees, dirty files, untracked files, ignored files, and submodule configurations block removal. Git runs without `--force`. If no local branch or tag contains the commit, the card includes creation of a recovery branch under `discard/retained-worktrees/`. Discard creates that branch before removal. A failed removal can leave this recovery branch in place; the result reports it.

The app serializes these tool operations. External Git processes are not controlled by Discard. Git performs its own final removal checks. Removing a worktree keeps the conversation history available.
