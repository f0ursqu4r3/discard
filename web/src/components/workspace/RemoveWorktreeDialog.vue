<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { TriangleAlert } from '@lucide/vue'
import { removeGitWorktree, worktreeRemovalInfo } from '@/bridge/api'
import type { WorktreeFilesDto } from '@/bridge/types'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Alert, AlertDescription } from '../ui/alert'
import { Checkbox } from '../ui/checkbox'
import { Label } from '../ui/label'
import { Button } from '../ui/button'

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ projectId: string; repository: string; worktree: string }>()
const busy = ref(false)
/** The changed and untracked files of the working tree. Null while the app reads the status, and when it cannot. */
const changes = ref<WorktreeFilesDto | null>(null)
const reading = ref(false)
/** Why the app refuses the removal, when it does. */
const refusal = ref<string | null>(null)
/** The user chose to discard the changes. */
const forced = ref(false)
const dirty = computed(() => (changes.value?.files.length ?? 0) > 0)
const rest = computed(() => {
  const count = changes.value?.more ?? 0
  return count === 1 ? 'and 1 more file' : `and ${count} more files`
})
watch(
  open,
  async (value) => {
    if (!value) return
    changes.value = null
    refusal.value = null
    forced.value = false
    reading.value = true
    const worktree = props.worktree
    try {
      const next = await worktreeRemovalInfo(props.projectId, props.repository, worktree)
      if (worktree === props.worktree) changes.value = next
    } catch (error) {
      if (worktree === props.worktree) refusal.value = String(error)
    } finally {
      reading.value = false
    }
  },
  { immediate: true },
)
// The confirm button needs the explicit choice while files would be lost.
const blocked = computed(() => reading.value || !!refusal.value || (dirty.value && !forced.value))
// Cancel is the safe choice, so it takes the focus.
function focusCancel(event: Event) {
  event.preventDefault()
  document.querySelector<HTMLElement>('[data-ui-id="remove-worktree-cancel"]')?.focus()
}
// Enter confirms unless a button has the focus: a focused button keeps its own action.
function enter(event: KeyboardEvent) {
  if ((event.target as HTMLElement | null)?.closest('button, [role="checkbox"]')) return
  event.preventDefault()
  void confirm()
}
async function confirm() {
  if (busy.value || blocked.value) return
  busy.value = true
  try {
    const result = await removeGitWorktree(
      props.projectId,
      props.repository,
      props.worktree,
      dirty.value && forced.value,
    )
    if (result.removed) {
      open.value = false
      return
    }
    // Files changed after the dialog read the status. Nothing was removed: ask again with the new list.
    changes.value = { files: result.files, more: result.more }
    forced.value = false
  } catch (error) {
    // The dialog stays open, so the user can read the reason or cancel.
    toast.error('Could not remove worktree', { description: String(error) })
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
      data-ui-id="remove-worktree-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusCancel"
      @keydown.enter="enter"
    >
      <DialogHeader>
        <DialogTitle>Remove worktree?</DialogTitle>
        <DialogDescription>
          This removes the working tree folder. The branch and its commits stay.
        </DialogDescription>
      </DialogHeader>
      <code
        data-ui-id="remove-worktree-path"
        class="rounded-md bg-muted px-2 py-1 font-mono text-xs break-all"
      >
        {{ worktree }}
      </code>
      <Alert v-if="refusal" data-ui-id="remove-worktree-refusal" class="border-warning">
        <TriangleAlert aria-hidden="true" />
        <AlertDescription>{{ refusal }}</AlertDescription>
      </Alert>
      <template v-else-if="dirty">
        <Alert data-ui-id="remove-worktree-changes" class="border-warning">
          <TriangleAlert aria-hidden="true" />
          <AlertDescription>
            <p>This working tree has changed or untracked files:</p>
            <ul class="mt-1 list-disc pl-4 font-mono text-xs">
              <li v-for="file in changes!.files" :key="file" class="break-all">{{ file }}</li>
            </ul>
            <p v-if="changes!.more" data-ui-id="remove-worktree-more" class="mt-1">{{ rest }}</p>
          </AlertDescription>
        </Alert>
        <div class="flex items-center gap-2">
          <Checkbox
            id="remove-worktree-force"
            v-model="forced"
            data-ui-id="remove-worktree-force"
            :disabled="busy"
          />
          <Label for="remove-worktree-force" class="font-normal">Discard these changes</Label>
        </div>
      </template>
      <DialogFooter>
        <Button
          data-ui-id="remove-worktree-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button
          data-ui-id="remove-worktree-confirm"
          variant="destructive"
          :disabled="busy || blocked"
          @click="confirm"
        >
          {{ busy ? 'Removing…' : 'Remove Worktree' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
