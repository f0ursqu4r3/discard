<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { searchAgentHistory, type HistoryMatch, type HistorySearch } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
const props = defineProps<{
  agentId: string
  agentName: string
  query: string
  revision: number
  busy?: boolean
}>()
const emit = defineEmits<{ select: [match: HistoryMatch] }>()
const result = ref<HistorySearch>({ results: [], hasMore: false }),
  loading = ref(false),
  error = ref('')
let generation = 0
async function search() {
  const current = ++generation
  result.value = { results: [], hasMore: false }
  error.value = ''
  if (!props.query.trim()) {
    loading.value = false
    return
  }
  loading.value = true
  try {
    const found = await searchAgentHistory(props.agentId, props.query)
    if (current === generation) result.value = found
  } catch (cause) {
    if (current === generation) error.value = String(cause)
  } finally {
    if (current === generation) loading.value = false
  }
}
watch(() => [props.agentId, props.query, props.revision], search, { immediate: true })
onUnmounted(() => {
  generation++
})
</script>
<template>
  <section data-ui-id="agent-history-search" class="flex h-full min-h-0 flex-col bg-card">
    <div class="flex flex-col gap-1 border-b border-border p-4">
      <h2 class="font-semibold">Search history</h2>
      <p class="break-words text-sm text-muted-foreground">
        Direct chats with {{ agentName }} · “{{ query }}”
      </p>
    </div>
    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <p v-if="loading" role="status">Searching saved messages…</p>
      <Alert v-else-if="error" variant="destructive" role="alert">
        <AlertDescription>
          {{ error }}
          <Button variant="outline" size="sm" @click="search">Retry</Button>
        </AlertDescription>
      </Alert>
      <template v-else>
        <p role="status" class="text-sm text-muted-foreground">
          {{
            result.hasMore
              ? 'Showing the newest 50 matches. Refine your search for more.'
              : `${result.results.length} matching messages`
          }}
        </p>
        <p v-if="!result.results.length" class="text-sm text-muted-foreground">
          No saved messages match this search.
        </p>
        <button
          v-for="match in result.results"
          :key="match.messageId"
          :disabled="busy"
          class="flex flex-col gap-2 rounded-lg border border-border p-3 text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
          @click="emit('select', match)"
        >
          <span class="text-xs text-muted-foreground">
            {{ match.channelName }} ·
            {{ match.role === 'user' ? 'You' : match.role === 'agent' ? agentName : 'System' }}
          </span>
          <span class="whitespace-pre-wrap break-words text-sm">{{ match.preview }}</span>
          <span class="text-xs text-primary">Jump to message</span>
        </button>
      </template>
    </div>
  </section>
</template>
