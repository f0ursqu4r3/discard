import { shallowRef } from 'vue'

/**
 * A thread or channel action that needs a dialog. The control that starts the action sets the request. One host
 * in `App.vue` (`ThreadActionDialogs.vue`) shows the dialog, so a menu deep in the tree needs no event chain.
 */
export type ActionRequest =
  | { kind: 'rename-thread'; threadId: string; name: string }
  | { kind: 'delete-thread'; threadId: string; name: string }
  | { kind: 'remove-from-channel'; messageId: string; threadName: string }
  | { kind: 'rename-channel'; projectId: string; channelId: string; name: string }
  | { kind: 'archive-channel'; projectId: string; channelId: string; name: string }
  | { kind: 'create-worktree'; projectId: string; repository: string; branch: string }
  | { kind: 'rename-branch'; projectId: string; repository: string; branch: string }
  | { kind: 'delete-branch'; projectId: string; repository: string; branch: string }
  | { kind: 'create-branch'; projectId: string; repository: string; worktree: string; name: string }
  | { kind: 'remove-worktree'; projectId: string; repository: string; worktree: string }

export const pendingAction = shallowRef<ActionRequest | null>(null)

export function requestAction(request: ActionRequest) {
  pendingAction.value = request
}
export function clearAction() {
  pendingAction.value = null
}
