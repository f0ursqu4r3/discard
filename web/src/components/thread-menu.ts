import { planned, type PlannedMenuEntry } from './project-menu'

type PlannedItem = Extract<PlannedMenuEntry, { kind: 'planned' }>

/** Thread controls that are visible and disabled until their behaviour exists. */
export const threadActions = {
  // TODO: List all threads of the channel, including threads that are not followed.
  list: planned('Threads'),
  // TODO: Edit the thread name.
  rename: planned('Rename thread'),
  // TODO: Delete a thread, with rules for messages that were posted to the parent channel.
  remove: planned('Delete thread'),
  // TODO: Remove a posted reference row from the channel, with rules for a parent agent that already read it.
  removeFromChannel: planned('Remove from channel'),
}

/** Planned entries of the thread panel menu and the thread row menu. */
export const threadMenuPlanned: PlannedItem[] = [threadActions.rename, threadActions.remove]
/** Planned entries of the menu of a posted reference row. */
export const referenceMenuPlanned: PlannedItem[] = [threadActions.removeFromChannel]
