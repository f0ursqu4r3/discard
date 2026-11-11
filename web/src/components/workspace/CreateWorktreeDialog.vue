<script setup lang="ts">
import { ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { TriangleAlert } from '@lucide/vue'
import { createBranchWorktree, worktreePlan } from '@/bridge/api'
import type { WorktreePlanDto } from '@/bridge/types'
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
const props = defineProps<{ projectId: string; repository: string; branch: string }>()
const busy = ref(false)
/** Where the worktree will go. Null while the app plans it, and when it cannot. */
const plan = ref<WorktreePlanDto | null>(null)
const planError = ref<string | null>(null)
watch(
  open,
  async (value) => {
    if (!value) return
    plan.value = null
    planError.value = null
    const { projectId, repository, branch } = props
    try {
      const next = await worktreePlan(projectId, repository, branch)
      if (branch === props.branch) plan.value = next
    } catch (error) {
      // The app refuses a branch that cannot get a worktree. The dialog says why.
      if (branch === props.branch) planError.value = String(error)
    }
  },
  { immediate: true },
)
async function confirm() {
  if (busy.value || !plan.value) return
  busy.value = true
  try {
    await createBranchWorktree(
      props.projectId,
      props.repository,
      props.branch,
      plan.value.channelId,
    )
    toast.success('Worktree created', { description: plan.value.path })
    open.value = false
  } catch (error) {
    // Git failed and nothing was saved. The dialog stays open, so the user can try again or cancel.
    toast.error('Could not create worktree', { description: String(error) })
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
      data-ui-id="create-worktree-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
    >
      <DialogHeader>
        <DialogTitle>Create worktree?</DialogTitle>
        <DialogDescription>
          This creates a linked worktree for the branch
          <span class="font-mono">{{ branch }}</span>
          .
        </DialogDescription>
      </DialogHeader>
      <div class="flex flex-col gap-1 text-sm">
        <span class="text-muted-foreground">Path</span>
        <code
          data-ui-id="create-worktree-path"
          class="rounded-md bg-muted px-2 py-1 font-mono text-xs break-all"
        >
          {{ plan?.path ?? '…' }}
        </code>
      </div>
      <Alert v-if="planError" data-ui-id="create-worktree-error" class="border-warning">
        <TriangleAlert aria-hidden="true" />
        <AlertDescription>{{ planError }}</AlertDescription>
      </Alert>
      <DialogFooter>
        <Button
          data-ui-id="create-worktree-cancel"
          variant="ghost"
          :disabled="busy"
          @click="open = false"
        >
          Cancel
        </Button>
        <Button data-ui-id="create-worktree-confirm" :disabled="busy || !plan" @click="confirm">
          {{ busy ? 'Creating…' : 'Create Worktree' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
