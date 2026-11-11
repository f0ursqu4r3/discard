<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { TriangleAlert } from '@lucide/vue'
import { branchDeletionInfo, deleteGitBranch } from '@/bridge/api'
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
const props = defineProps<{ projectId: string; repository: string; branch: string }>()
const busy = ref(false)
/** Commits of the branch that the default branch does not have. Null while the app counts, and when it cannot. */
const unmerged = ref<number | null>(null)
const counting = ref(false)
/** Why the app refuses the delete, when it does. */
const refusal = ref<string | null>(null)
/** The user chose to delete the unmerged commits too. */
const forced = ref(false)
const commits = computed(() => {
  const count = unmerged.value ?? 0
  return count === 1 ? '1 unmerged commit' : `${count} unmerged commits`
})
watch(
  open,
  async (value) => {
    if (!value) return
    unmerged.value = null
    refusal.value = null
    forced.value = false
    counting.value = true
    const branch = props.branch
    try {
      const count = await branchDeletionInfo(props.projectId, props.repository, branch)
      if (branch === props.branch) unmerged.value = count
    } catch (error) {
      if (branch === props.branch) refusal.value = String(error)
    } finally {
      counting.value = false
    }
  },
  { immediate: true },
)
// The confirm button needs the explicit choice while commits would be lost.
const blocked = computed(
  () => counting.value || !!refusal.value || ((unmerged.value ?? 0) > 0 && !forced.value),
)
// Cancel is the safe choice, so it takes the focus.
function focusCancel(event: Event) {
  event.preventDefault()
  document.querySelector<HTMLElement>('[data-ui-id="delete-branch-cancel"]')?.focus()
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
    const result = await deleteGitBranch(
      props.projectId,
      props.repository,
      props.branch,
      (unmerged.value ?? 0) > 0 && forced.value,
    )
    if (result.deleted) {
      open.value = false
      return
    }
    // Commits appeared after the dialog counted. Nothing was deleted: ask again with the new count.
    unmerged.value = result.unmerged
    forced.value = false
  } catch (error) {
    // The dialog stays open, so the user can read the reason or cancel.
    toast.error('Could not delete branch', { description: String(error) })
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
      data-ui-id="delete-branch-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusCancel"
      @keydown.enter="enter"
    >
      <DialogHeader>
        <DialogTitle>Delete branch?</DialogTitle>
        <DialogDescription>This deletes the local branch “{{ branch }}”.</DialogDescription>
      </DialogHeader>
      <Alert v-if="refusal" data-ui-id="delete-branch-refusal" class="border-warning">
        <TriangleAlert aria-hidden="true" />
        <AlertDescription>{{ refusal }}</AlertDescription>
      </Alert>
      <template v-else-if="unmerged">
        <Alert data-ui-id="delete-branch-warning" class="border-warning">
          <TriangleAlert aria-hidden="true" />
          <AlertDescription>
            “{{ branch }}” has {{ commits }} that the default branch does not have. Deleting the
            branch drops them.
          </AlertDescription>
        </Alert>
        <div class="flex items-center gap-2">
          <Checkbox
            id="delete-branch-force"
            v-model="forced"
            data-ui-id="delete-branch-force"
            :disabled="busy"
          />
          <Label for="delete-branch-force" class="font-normal">Delete {{ commits }}</Label>
        </div>
      </template>
      <DialogFooter>
        <Button
          data-ui-id="delete-branch-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button
          data-ui-id="delete-branch-confirm"
          variant="destructive"
          :disabled="busy || blocked"
          @click="confirm"
        >
          {{ busy ? 'Deleting…' : 'Delete Branch' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
