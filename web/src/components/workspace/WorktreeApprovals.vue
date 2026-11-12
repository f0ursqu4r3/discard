<script setup lang="ts">
import { shortPath } from '@/settings/paths'
import { onMounted, onUnmounted, ref, watch } from 'vue'
import {
  getWorktreeRequests,
  decideWorktreeRemoval,
  type WorktreeRemovalRequest,
} from '@/bridge/api'
import { Button } from '../ui/button'
const props = defineProps<{ channelId: string; running: boolean }>()
const requests = ref<WorktreeRemovalRequest[]>([])
const busy = ref<string | null>(null)
const error = ref('')
const buttonClass =
  'border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)] hover:bg-[var(--protected-bg)] hover:text-[var(--protected-fg)] focus-visible:ring-[var(--protected-border)]'
let disposed = false
let generation = 0
let timer: ReturnType<typeof setInterval> | undefined
async function reload() {
  const current = ++generation
  try {
    const rows = await getWorktreeRequests(props.channelId)
    if (!disposed && current === generation) {
      requests.value = rows
      error.value = ''
    }
  } catch (cause) {
    if (!disposed && current === generation)
      error.value = `Worktree approvals could not load: ${String(cause)}`
  }
}
async function decide(request: WorktreeRemovalRequest, approve: boolean) {
  if (busy.value) return
  const channel = props.channelId
  busy.value = request.id
  error.value = ''
  try {
    const updated = await decideWorktreeRemoval(channel, request.id, approve)
    if (!disposed && props.channelId === channel)
      requests.value = requests.value.map((row) => (row.id === updated.id ? updated : row))
  } catch (cause) {
    if (!disposed && props.channelId === channel) error.value = String(cause)
  } finally {
    busy.value = null
  }
}
watch(
  () => props.channelId,
  () => {
    requests.value = []
    void reload()
  },
)
watch(
  () => props.running,
  () => void reload(),
)
onMounted(() => {
  void reload()
  timer = setInterval(() => {
    if (!busy.value) void reload()
  }, 2000)
})
onUnmounted(() => {
  disposed = true
  generation++
  clearInterval(timer)
})
</script>
<template>
  <div v-if="requests.length || error" class="flex flex-col gap-3 px-3 py-3 sm:px-6">
    <p v-if="error" role="alert" class="text-sm">{{ error }}</p>
    <section
      v-for="request in requests"
      :key="request.id"
      data-ui-id="worktree-removal-approval"
      data-protected="true"
      class="flex flex-col gap-3 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-4 text-sm text-[var(--protected-fg)]"
      aria-label="Worktree removal approval"
    >
      <p class="font-semibold">
        {{ request.state === 'pending' ? 'Remove this worktree?' : 'Worktree removal' }}
      </p>
      <dl class="grid min-w-0 gap-1">
        <dt class="font-medium">Path</dt>
        <dd class="break-all font-mono">{{ shortPath(request.inspection.path) }}</dd>
        <dt class="mt-2 font-medium">Commit</dt>
        <dd class="break-all font-mono">{{ request.inspection.head }}</dd>
      </dl>
      <template v-if="request.state === 'pending'">
        <p>
          This removes the worktree directory. Discard will recheck its Git state and will not force
          removal.
        </p>
        <p v-if="request.inspection.recoveryBranch" class="break-words">
          Approval also preserves the commit on branch
          <code class="break-all">{{ request.inspection.recoveryBranch }}</code>
          .
        </p>
        <p v-else>The commit is contained in a local branch or tag. Those refs will remain.</p>
        <p>
          Approval applies once and expires after 10 minutes. Restarting Discard cancels pending
          requests.
        </p>
        <p v-if="running">Wait for the current response to finish before approving removal.</p>
        <div class="flex gap-2">
          <Button
            variant="outline"
            :class="buttonClass"
            :disabled="!!busy"
            @click="decide(request, false)"
          >
            Deny
          </Button>
          <Button
            variant="outline"
            :class="buttonClass"
            :disabled="!!busy || running"
            @click="decide(request, true)"
          >
            {{ busy === request.id ? 'Working…' : 'Approve removal' }}
          </Button>
        </div>
      </template>
      <p v-else role="status" class="break-words">{{ request.result || request.state }}</p>
    </section>
  </div>
</template>
