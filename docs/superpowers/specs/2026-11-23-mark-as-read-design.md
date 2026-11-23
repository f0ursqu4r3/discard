# Mark Project and Group as Read design

Date: 2026-11-23. Size: small change. Status: awaiting approval.

## Intent

The project menu item **Mark Project as Read** and the group menu item **Mark Group as Read** are visible, disabled, and labelled Soon. The user wants both to work: one click clears the unread marks of everything in the project, or in every project of the group.

## Goals

- Mark Project as Read clears the unread mark of every channel of the project and every thread under those channels, including thread chips.
- Mark Group as Read does the same for every member project of the group.
- The action works for channels that no window shows.

## Non-goals

- Pending approval badges. They are not read markers and stay as they are.
- Direct messages (they belong to no project).
- A per-project "read up to" watermark.
- Disabling the item when nothing is unread.

## Anti-goals

- A channel that looks read while a thread chip under it stays unread.
- A half-applied result: some markers saved, others not.

## Approaches

- **A. One Rust command per scope (chosen).** Rust walks the loaded channels and thread records, sets each marker to the last message, and saves once.
- **B. The web app calls `mark_read` per channel (rejected).** `mark_read_in` works only for a channel that a window shows.
- **C. A per-project watermark (rejected).** It changes how unread works everywhere.

## Decisions

1. **Scope of a project.** Every loaded non-thread channel whose project is the target (the same rule the rail uses to list a project's channels), and every thread record whose parent is one of those channels. Muted channels count: marking them is harmless.
2. **The marker.** A channel's or thread's marker becomes the ID of the last message of its history, whatever its role. An empty history changes nothing. A marker never moves backward: a marker already at or after the last message stays.
3. **One write.** `StateStore` gains `set_last_read_many(&mut self, items: &[(String, String)]) -> Result<(), String>`. It saves the file once. On error the file and the in-memory state stay as they were.
4. **Commands.** `mark_project_read(project_id: String)` and `mark_group_read(group_id: String)` in `discard-app`. The group command reads the member projects from the saved rail layout. An unknown group ID is an error. A project with no channels, or an empty group, is `Ok` with no change.
5. **Refresh.** For each channel whose marker changed, clear its `new_anchor`. Bump `rev`, and bump `history_rev` when a changed channel is one that a view shows (as `mark_read_in` does). Wake the pump once.
6. **Menus.** In `web/src/components/project-menu.ts` the project item becomes the action `mark-read` and the group item becomes `mark-group-read`. Remove the `planned(...)` calls and their `TODO` comments. The items are always enabled.
7. **Docs.** Update `docs/settings.md` (the project and group menu paragraphs that list these items as Soon) and `docs/project-channel-status.md`.

## Constraints

- Core logic is testable without the GUI: the walk over channels and thread records is a function with unit tests.
- Web tests go in a `__tests__` directory beside the code they test (AGENTS.md). Update the tests that assert the planned items (`project-menu.test.ts` and `ServerRail.groups.test.ts`).
- Follow AGENTS.md: no `TODO` for implemented items, no no-op handler.

## Left to the builder

- How to read the last message ID of an unloaded thread record (the record holds its rows).
- Whether the command takes a view or works app-wide. It takes an explicit project or group ID, so it works from any window.

## Tests

- `StateStore::set_last_read_many`: saves all markers in one write; a failed write changes nothing.
- The walk: a project with two channels and a thread under one marks all three; a channel of another project is untouched; an empty-history channel changes nothing; a marker already at the last message does not change.
- Group: markers of every member project change; an unknown group is an error.
- Pending approvals are unchanged after the action.
- Web: both menu items are enabled actions with no `Soon` text, and they invoke their commands.
