<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import type { MessageDto } from '@/bridge/types'
import { clearAction, pendingAction, type ActionRequest } from '../thread-actions'
import { agentFollows } from '../threads'
import DeleteThreadDialog from './DeleteThreadDialog.vue'
import RemoveFromChannelDialog from './RemoveFromChannelDialog.vue'
import RenameChannelDialog from './RenameChannelDialog.vue'
import RenameThreadDialog from './RenameThreadDialog.vue'

/** The one host of the thread and channel action dialogs. `pendingAction` says which dialog is open. */
const props = defineProps<{
  /** The transcript of the selected channel. The remove dialog reads it to see whether an agent message follows. */
  messages: MessageDto[]
}>()
// The last request keeps its values while the dialog closes. The warning of the remove dialog is fixed when the
// dialog opens: the row leaves the transcript when the user confirms.
const last = shallowRef<ActionRequest | null>(null)
const agentRead = shallowRef(false)
watch(
  pendingAction,
  (request) => {
    if (!request) return
    last.value = request
    agentRead.value =
      request.kind === 'remove-from-channel' && agentFollows(props.messages, request.messageId)
  },
  { immediate: true },
)
const isOpen = (kind: ActionRequest['kind']) => pendingAction.value?.kind === kind
const only = <K extends ActionRequest['kind']>(kind: K) =>
  computed(() =>
    last.value?.kind === kind ? (last.value as Extract<ActionRequest, { kind: K }>) : null,
  )
const renameThread = only('rename-thread')
const deleteThread = only('delete-thread')
const removeFromChannel = only('remove-from-channel')
const renameChannel = only('rename-channel')
</script>
<template>
  <RenameThreadDialog
    v-if="renameThread"
    :open="isOpen('rename-thread')"
    :thread-id="renameThread.threadId"
    :name="renameThread.name"
    @update:open="(value) => !value && clearAction()"
  />
  <DeleteThreadDialog
    v-if="deleteThread"
    :open="isOpen('delete-thread')"
    :thread-id="deleteThread.threadId"
    :name="deleteThread.name"
    @update:open="(value) => !value && clearAction()"
  />
  <RemoveFromChannelDialog
    v-if="removeFromChannel"
    :open="isOpen('remove-from-channel')"
    :message-id="removeFromChannel.messageId"
    :thread-name="removeFromChannel.threadName"
    :agent-read="agentRead"
    @update:open="(value) => !value && clearAction()"
  />
  <RenameChannelDialog
    v-if="renameChannel"
    :open="isOpen('rename-channel')"
    :project-id="renameChannel.projectId"
    :channel-id="renameChannel.channelId"
    :name="renameChannel.name"
    @update:open="(value) => !value && clearAction()"
  />
</template>
