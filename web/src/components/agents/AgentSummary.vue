<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getAgents, type AgentEntry } from '@/bridge/api'
import type { SnapshotDto } from '@/bridge/types'
import AutorunIndicator from '@/components/workspace/AutorunIndicator.vue'
import AgentAvatar from '@/components/agents/AgentAvatar.vue'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
const props = defineProps<{
  agentId: string
  name: string
  status: string
  autorun?: SnapshotDto['autorun']
}>()
const emit = defineEmits<{ profile: [id: string] }>()
const agent = ref<AgentEntry | null>(null),
  loading = ref(true),
  error = ref('')
let generation = 0,
  disposed = false,
  stop: UnlistenFn | undefined
async function reload() {
  const current = ++generation
  agent.value = null
  error.value = ''
  loading.value = true
  try {
    const agents = await getAgents()
    if (!disposed && current === generation)
      agent.value = agents.find((a) => a.id === props.agentId) ?? null
  } catch (cause) {
    if (!disposed && current === generation) error.value = String(cause)
  } finally {
    if (!disposed && current === generation) loading.value = false
  }
}
watch(() => props.agentId, reload, { immediate: true })
onMounted(async () => {
  try {
    const unlisten = await listen('discard://agents', () => void reload())
    if (disposed) unlisten()
    else stop = unlisten
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  generation++
  stop?.()
})
</script>
<template>
  <section data-ui-id="agent-summary" class="flex min-h-full flex-col gap-5">
    <div class="h-16 shrink-0 rounded-t-lg bg-muted" aria-hidden="true" />
    <AgentAvatar
      :agent-id="agentId"
      :name="agent?.name || name"
      :avatar="agent?.avatar"
      class="-mt-12 ml-4 size-20 ring-4 ring-card"
    />
    <div class="flex min-h-0 flex-1 flex-col gap-5 px-3 pb-3">
      <div>
        <h2 class="break-words text-lg font-semibold">{{ agent?.name || name }}</h2>
        <p class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
          <span>
            {{
              agent?.archived
                ? 'Archived'
                : {
                    idle: 'Idle',
                    working: 'Working',
                    awaiting_approval: 'Waiting for approval',
                    offline: 'Offline',
                  }[status] || status
            }}
          </span>
          <AutorunIndicator :autorun="autorun ?? null" />
        </p>
      </div>
      <p v-if="loading" role="status">Loading profile…</p>
      <Alert v-else-if="error" variant="destructive">
        <AlertDescription>
          {{ error }}
          <Button variant="outline" size="sm" @click="reload">Retry</Button>
        </AlertDescription>
      </Alert>
      <template v-else-if="agent">
        <div class="flex flex-wrap gap-2">
          <Badge variant="secondary">{{ agent.provider }}</Badge>
          <span class="break-all text-sm text-muted-foreground">{{ agent.model }}</span>
        </div>
        <div>
          <h3 class="mb-2 text-sm font-semibold">About</h3>
          <p class="line-clamp-6 whitespace-pre-wrap break-words text-sm text-muted-foreground">
            {{ agent.systemPrompt || 'No personality added yet.' }}
          </p>
        </div>
        <Button variant="outline" class="mt-auto w-full" @click="emit('profile', agent.id)">
          View full profile
        </Button>
      </template>
      <p v-else class="text-sm text-muted-foreground">
        This agent is no longer available. Saved chats remain readable.
      </p>
    </div>
  </section>
</template>
