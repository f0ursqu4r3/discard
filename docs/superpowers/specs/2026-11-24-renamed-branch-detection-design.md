# Renamed branch detection design

Date: 2026-11-24. Size: small. Status: approved.

## Intent

A branch renamed outside Discard (a terminal, an IDE) keeps its channel and history. Today the binding keeps the old name and the new name gets a fresh, empty conversation.

## Goals

- Discard finds a rename that Git recorded and moves the channel binding to the new name.
- The channel keeps its identity, history, and worktree record.
- The channel gets one system note: "Branch renamed from `old` to `new`."

## Non-goals

- A Rebind picker for deleted branches or lost reflogs. Branch retirement stays pending.
- Guessing from commit tips. Two branches at one commit cannot be told apart.
- Detached worktrees.

## Anti-goals

- A binding that moves to the wrong branch. When a result is not exact, the binding stays.
- A rename that overwrites a live channel.

## Approaches

- **A. Follow the reflog. (Chosen.)** Git writes "Branch: renamed refs/heads/old to refs/heads/new" to the reflog of the new branch. The match is exact. It misses a rename after reflog expiry, in a bare repository, or with reflogs off.
- **B. Reflog plus a tip heuristic.** Rejected: wrong pairs for branches at one commit.
- **C. Ask the user with a Rebind picker.** Deferred: it needs new UI and fits the pending retirement work.

## Design

- Detection runs inside `refresh_branches`, once per repository, for active branch channels of the project.
- A bound name that `list_branches` no longer lists is a candidate. For each candidate, Discard reads the reflogs of the listed branches that have no channel. It follows the chain of "renamed X to Y" entries from the candidate name to a listed branch.
- Discard moves the binding with `rename_project_branch`. It then posts the system note.
- Discard skips a candidate when:
  - its channel is busy (the rule of **Rename Branch…**),
  - the target name already has an active channel,
  - the chain is not exact, or two candidates reach one target.
- The check reads Git with an argument list and no hooks, like the other branch code.
- The branch list refreshes after a move.

## Left to the builder

- The reflog read API in `discard-core` (for example `git reflog show --format=%gs <branch>`).
- Where the system note posts and its exact storage call.

## Tests

Real temp repository: one rename, a chain of renames, a busy channel, a name clash, a deleted branch (no move), two branches at one commit.

## Docs

`docs/project-channel-status.md` and `docs/worktrees.md`: replace the "pending" wording with the new behavior.
