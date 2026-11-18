<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { MemoryRecord, MemoryProposal } from './types'
import MemoryProposalCard from './MemoryProposal.vue'
import { Button } from '../ui/button'
const props = withDefaults(
  defineProps<{
    records: MemoryRecord[]
    proposals: MemoryProposal[]
    channelId: string
    scopes?: { scope: string; title: string }[]
  }>(),
  { scopes: () => [] },
)
const emit = defineEmits<{ changed: [] }>()
const error = ref('')
const busy = ref(false)
async function undo(record: MemoryRecord) {
  if (!record.item) return
  error.value = ''
  busy.value = true
  try {
    await invoke('memory_mutate', {
      id: record.item.id,
      expectedVersion: record.item.version,
      action: 'undo',
      channelId: props.channelId,
    })
    emit('changed')
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
const expanded = ref<Record<string, boolean>>({})
function toggle(key: string) {
  expanded.value[key] = !expanded.value[key]
}
function scopeLabel(scope: string) {
  return props.scopes.find((entry) => entry.scope === scope)?.title ?? scope
}
function preview(item: NonNullable<MemoryRecord['item']>) {
  const line = item.content.split('\n').find((text) => text.trim()) ?? ''
  return item.deleted ? `Forgotten: ${line}` : line
}
function proposal(record: MemoryRecord) {
  return props.proposals.find((p) => p.id === record.proposalId)
}
</script>
<template>
  <div v-if="records.length" class="mx-6 my-2 space-y-2 text-xs" data-ui-id="response-memory">
    <template v-for="record in records" :key="record.key">
      <section v-if="record.item" class="rounded border border-border px-3 py-1.5">
        <div class="flex items-center gap-2">
          <span class="shrink-0 font-medium">
            {{ record.type === 'automatic' ? 'Automatic save' : 'Approved update' }}
            · {{ scopeLabel(record.item.scope) }} · v{{ record.item.version }}
          </span>
          <span v-if="!expanded[record.key]" class="min-w-0 flex-1 truncate text-muted-foreground">
            {{ preview(record.item) }}
          </span>
          <span v-else class="flex-1" />
          <Button
            size="sm"
            variant="ghost"
            :aria-expanded="!!expanded[record.key]"
            @click="toggle(record.key)"
          >
            {{ expanded[record.key] ? 'Hide' : 'Show' }}
          </Button>
          <Button size="sm" variant="outline" :disabled="busy" @click="undo(record)">Undo</Button>
        </div>
        <div v-if="expanded[record.key]" class="mt-2 max-h-64 overflow-y-auto">
          <p class="whitespace-pre-wrap wrap-break-word">
            {{ record.item.deleted ? 'Forgotten:' : '' }}
            {{ record.item.content }}
          </p>
        </div>
      </section>
      <MemoryProposalCard
        v-else-if="record.pending && proposal(record)"
        :proposal="proposal(record)!"
        :channel-id="channelId"
        @changed="emit('changed')"
      />
      <p v-else-if="record.type === 'proposal'" class="text-muted-foreground">
        Memory proposal resolved.
      </p>
    </template>
    <p v-if="error" role="alert">{{ error }}</p>
  </div>
</template>
