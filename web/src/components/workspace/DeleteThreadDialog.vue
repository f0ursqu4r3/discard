<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { deleteThread, threadPostedCount } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Button } from '../ui/button'

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ threadId: string; name: string }>()
const busy = ref(false)
/** Messages of the thread that were posted to the channel. Null while the app counts, and when it cannot. */
const posted = ref<number | null>(null)
const counting = ref(false)
const stays = computed(() => {
  const count = posted.value ?? 0
  return count === 1
    ? '1 posted message stays in the channel as a normal message.'
    : `${count} posted messages stay in the channel as normal messages.`
})
watch(
  open,
  async (value) => {
    if (!value) return
    posted.value = null
    counting.value = true
    const id = props.threadId
    try {
      const count = await threadPostedCount(id)
      if (id === props.threadId) posted.value = count
    } catch {
      // The delete does not need the count. The dialog then names no number.
    } finally {
      counting.value = false
    }
  },
  { immediate: true },
)
// Cancel is the safe choice, so it takes the focus.
function focusCancel(event: Event) {
  event.preventDefault()
  document.querySelector<HTMLElement>('[data-ui-id="delete-thread-cancel"]')?.focus()
}
// Enter confirms unless a button has the focus: a focused button keeps its own action.
function enter(event: KeyboardEvent) {
  if ((event.target as HTMLElement | null)?.closest('button')) return
  event.preventDefault()
  void confirm()
}
async function confirm() {
  if (busy.value || counting.value) return
  busy.value = true
  try {
    await deleteThread(props.threadId)
    open.value = false
  } catch (error) {
    // The dialog stays open, so the user can read the reason or cancel.
    toast.error('Could not delete thread', { description: String(error) })
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
      data-ui-id="delete-thread-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusCancel"
      @keydown.enter="enter"
    >
      <DialogHeader>
        <DialogTitle>Delete thread?</DialogTitle>
        <DialogDescription>
          This deletes “{{ name }}” with its messages, approvals, and history. You cannot undo it.
        </DialogDescription>
      </DialogHeader>
      <p
        v-if="posted && posted > 0"
        data-ui-id="delete-thread-posted"
        class="text-sm text-muted-foreground"
      >
        {{ stays }}
      </p>
      <DialogFooter>
        <Button
          data-ui-id="delete-thread-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button
          data-ui-id="delete-thread-confirm"
          variant="destructive"
          :disabled="busy || counting"
          @click="confirm"
        >
          {{ busy ? 'Deleting…' : 'Delete thread' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
