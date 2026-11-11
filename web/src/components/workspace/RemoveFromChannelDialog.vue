<script setup lang="ts">
import { ref } from 'vue'
import { toast } from 'vue-sonner'
import { TriangleAlert } from '@lucide/vue'
import { removePostedMessage } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Alert, AlertDescription } from '../ui/alert'
import { Button } from '../ui/button'

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{
  /** The ID of the reference row in the channel. */
  messageId: string
  threadName: string
  /** An agent message follows the row in the channel, so the agent already read it. */
  agentRead?: boolean
}>()
const busy = ref(false)
// Cancel is the safe choice, so it takes the focus.
function focusCancel(event: Event) {
  event.preventDefault()
  document.querySelector<HTMLElement>('[data-ui-id="remove-from-channel-cancel"]')?.focus()
}
// Enter confirms unless a button has the focus: a focused button keeps its own action.
function enter(event: KeyboardEvent) {
  if ((event.target as HTMLElement | null)?.closest('button')) return
  event.preventDefault()
  void confirm()
}
async function confirm() {
  if (busy.value) return
  busy.value = true
  try {
    await removePostedMessage(props.messageId)
    open.value = false
  } catch (error) {
    // The dialog stays open, so the user can read the reason or cancel.
    toast.error('Could not remove the message', { description: String(error) })
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog
    :open="open"
    @update:open="
      (value) => {
        if (!busy) open = value
      }
    "
  >
    <DialogContent
      data-ui-id="remove-from-channel-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusCancel"
      @keydown.enter="enter"
    >
      <DialogHeader>
        <DialogTitle>Remove from channel?</DialogTitle>
        <DialogDescription>
          This removes the message from the channel. It stays in the thread “{{ threadName }}”. You
          can post it again.
        </DialogDescription>
      </DialogHeader>
      <Alert v-if="agentRead" data-ui-id="remove-from-channel-warning" class="border-warning">
        <TriangleAlert aria-hidden="true" />
        <AlertDescription>
          The agent already read this message. Earlier replies can still refer to it. Later turns
          will not see it.
        </AlertDescription>
      </Alert>
      <DialogFooter>
        <Button
          data-ui-id="remove-from-channel-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button
          data-ui-id="remove-from-channel-confirm"
          variant="destructive"
          :disabled="busy"
          @click="confirm"
        >
          {{ busy ? 'Removing…' : 'Remove from channel' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
