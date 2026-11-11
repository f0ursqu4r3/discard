# Navigation menu actions

Date: 2026-11-11

## Goal

The row menus of the channel list have seven planned items, labelled `Soon`. This spec makes them work.

| Row kind | Items |
|---|---|
| Channel | Mute Channel, Archive Channel… |
| Branch | Create Worktree…, Rename Branch…, Delete Branch… |
| Worktree | Create Branch…, Remove Worktree… |

## Owner decisions (2026-11-11)

- A muted channel shows no unread marker. Pending approval badges still show.
- Delete Branch… warns about unmerged commits, then allows the delete after a second, explicit choice.
- Remove Worktree… warns about uncommitted changes, then allows the removal after an explicit choice.

The other decisions are the designer's calls.

## Terms

- **Channel row**: a row of kind `channel` in `NavigationContextMenu.vue` (`#general` and text channels).
- **Main checkout**: the repository's primary working tree. It is not a linked worktree.
- **Busy channel**: a channel with a running turn, a running sub-agent task in one of its threads, a pending approval, or an archive in progress.

## Decisions

### Mute Channel

1. Mute is a saved flag on a channel. A schema migration adds it. The default is off.
2. The channel row menu shows **Mute Channel** when the flag is off and **Unmute Channel** when it is on. The item works at once, with no dialog.
3. A muted channel row shows no unread marker. Its name is dimmed, and a bell-off icon shows beside it with the accessible label `Muted`.
4. Unread threads of a muted channel do not mark the channel row. The Threads list of the channel still shows them as unread.
5. A muted channel does not count toward the unread marker of its project on the project rail.
6. Pending approval badges are not changed by mute, on the row and on the project rail.
7. Mute does not change read markers, messages, agents, or turns. Unmute shows the unread marker again if the channel has unread messages.
8. A thread cannot be muted. `#general` can be muted.

### Archive Channel…

9. **Archive Channel…** in the channel row menu archives that row's channel. It uses the same rules, confirm step, and store path as the **Archive Channel…** item of the menu bar. The menu bar item does not change.
10. The item is disabled, with no `Soon` label, for `#general`, for an archived channel, and for a busy channel.
11. A confirm dialog opens with Cancel in focus. It names the channel. The confirm button is `Archive Channel`.
12. If the archived channel was open, the app opens `#general` of the same project.
13. This spec adds no restore action.

### Create Worktree… (branch row)

14. The item creates a linked worktree for the row's branch with the existing worktree service (`create_for_branch`). The worktree path follows the existing rule for owned worktrees.
15. The item is disabled, with no `Soon` label, when the branch is checked out in any working tree or its channel already has a worktree.
16. A confirm dialog shows the branch name and the target path. The confirm button is `Create Worktree`.
17. A Git failure shows as an error toast with the Git message. No record is saved on failure.

### Rename Branch… (branch row)

18. A dialog asks for the new name. The field starts with the current name, selected.
19. The app validates the name with `git check-ref-format --branch`. It refuses an empty name, the current name, and a name that another local branch has. The dialog shows the reason and stays open.
20. The rename runs `git branch -m <old> <new>` in the branch's repository. It works for a branch that is checked out.
21. The store then moves the channel binding to the new name with the existing branch rename path, in one transaction. The channel keeps its messages, sessions, and worktree record. The worktree record's branch name is updated.
22. The item is disabled, with no `Soon` label, when the branch's channel is busy.
23. If Git succeeds and the store update fails, the app renames the branch back and shows the error.

### Delete Branch… (branch row)

24. The item is disabled, with no `Soon` label, when the branch is checked out in any working tree or its channel is busy.
25. The confirm dialog opens with Cancel in focus. It names the branch.
26. The app counts the commits on the branch that are not reachable from the repository's default branch (the branch of the main checkout). If the count is zero, one confirm deletes the branch with `git branch -d`.
27. If the count is more than zero, the dialog shows a warning with the count and a checkbox: `Delete <N> unmerged commit(s)`. The confirm button is disabled until the checkbox is on. The delete then runs `git branch -D`.
28. The backend checks the count again at confirm time. If unmerged commits exist and the request does not carry the explicit force choice, the backend refuses.
29. After the delete, the channel follows the existing rule for a deleted branch: it moves to Archived only if it has a conversation.
30. The default branch cannot be deleted. The item is disabled for it.

### Create Branch… (worktree row)

31. A dialog asks for the branch name. Validation is the same as decision 19.
32. The app runs `git switch -c <name>` in that working tree. The new branch starts at the working tree's HEAD. Uncommitted changes stay in the working tree.
33. The item is disabled, with no `Soon` label, when a channel that uses that working tree is busy.
34. After success, the app refreshes the branch list. Channel bindings follow the existing rules for a working tree whose branch changed.

### Remove Worktree… (worktree row)

35. The item is disabled, with no `Soon` label, for the main checkout and when a channel that uses the working tree is busy.
36. The confirm dialog opens with Cancel in focus. It shows the path and states that the branch and its commits stay.
37. The app reads the working tree status. If it is clean, one confirm removes it with `git worktree remove`.
38. If it has changed or untracked files, the dialog lists up to 10 of them, gives the count of the rest, and shows a checkbox: `Discard these changes`. The confirm button is disabled until the checkbox is on. The removal then uses force.
39. The backend reads the status again at confirm time. If work would be lost and the request does not carry the explicit force choice, the backend refuses and returns the file list.
40. The store deletes the worktree record of any channel bound to that path. The channel stays. Its next turn follows the existing prepare rules.
41. The branch is never deleted by this action.

### Shared rules

42. Every Git command runs with an argument list, never through a shell. Branch names and paths are passed as separate arguments, after `--` where Git accepts it.
43. The backend checks every disabled rule again. The web state is not trusted.
44. The `TODO` comments and planned entries of the seven actions are gone. The shared planned-item model stays for other planned items. An action that a state blocks is disabled without `Soon`.
45. After each Git action, the app starts a branch refresh so the channel list shows the new state.
46. Lua-drawn UI that shows channel rows gets the muted state and the same visual treatment.

## Tests

- Core: Git actions against temporary repositories, including each refusal, the unmerged count, the force paths, and the rename rollback.
- Store: mute flag migration and round trip, unread rollup with a muted channel, branch rename binding.
- App: each command's state checks.
- Web: menu labels and disabled states for each row kind, each dialog's validation and confirm gate, muted row display.
