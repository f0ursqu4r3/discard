import type { ThreadState } from '../bridge/types'

/** Labels of the thread actions. Each one works. A state can still block one: see `threadDeleteBlock`. */
export const threadActionLabels = {
  list: 'Threads',
  rename: 'Rename thread',
  remove: 'Delete thread',
  removeFromChannel: 'Remove from channel',
} as const

/** What the app reports about a thread that decides whether it can be deleted. */
export interface ThreadActivity {
  /** A turn runs in the thread. */
  running?: boolean
  /** The state of a sub-agent task. */
  state?: ThreadState | null
  pendingApprovals?: number
}

/**
 * Why a thread cannot be deleted now, or null. This is a temporary state, not a planned action: the item stays
 * enabled-looking in every other state. The app checks again when the user confirms.
 */
export function threadDeleteBlock(thread: ThreadActivity): string | null {
  if (thread.running) return 'A turn runs in this thread.'
  if (thread.state === 'running') return 'A sub-agent task runs in this thread.'
  if ((thread.pendingApprovals ?? 0) > 0) return 'An approval waits in this thread.'
  return null
}
