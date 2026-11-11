<script setup lang="ts">
import { ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { TriangleAlert } from '@lucide/vue'
import { archiveProjectChannel } from '@/bridge/api'
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
const props = defineProps<{ projectId: string; channelId: string; name: string }>()
const busy = ref(false)
/** The items that a forced archive deletes. Null until the app asks for the second confirm. */
const items = ref<string[] | null>(null)
watch(
  open,
  (value) => {
    if (value) items.value = null
  },
  { immediate: true },
)
// Cancel is the safe choice, so it takes the focus.
function focusCancel(event: Event) {
  event.preventDefault()
  document.querySelector<HTMLElement>('[data-ui-id="archive-channel-cancel"]')?.focus()
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
    const result = await archiveProjectChannel(
      props.projectId,
      props.channelId,
      items.value !== null,
    )
    if (result.archived) {
      open.value = false
      return
    }
    // Archiving would delete files. The dialog asks again with the list.
    items.value = result.items
  } catch (error) {
    // The dialog stays open, so the user can read the reason or cancel.
    toast.error('Could not archive channel', { description: String(error) })
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
      data-ui-id="archive-channel-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusCancel"
      @keydown.enter="enter"
    >
      <DialogHeader>
        <DialogTitle>Archive channel?</DialogTitle>
        <DialogDescription>
          This archives “{{ name }}”. Its messages stay in the Archived list.
        </DialogDescription>
      </DialogHeader>
      <Alert v-if="items" data-ui-id="archive-channel-items" class="border-warning">
        <TriangleAlert aria-hidden="true" />
        <AlertDescription>
          <p>Archiving will delete these items:</p>
          <ul class="mt-1 list-disc pl-4 font-mono text-xs">
            <li v-for="item in items" :key="item" class="break-all">{{ item }}</li>
          </ul>
        </AlertDescription>
      </Alert>
      <DialogFooter>
        <Button
          data-ui-id="archive-channel-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button
          data-ui-id="archive-channel-confirm"
          :variant="items ? 'destructive' : 'default'"
          :disabled="busy"
          @click="confirm"
        >
          {{ busy ? 'Archiving…' : items ? 'Archive and delete' : 'Archive Channel' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
