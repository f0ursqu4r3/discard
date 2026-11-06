<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { MemoryRecord, MemoryProposal } from './types'
import MemoryProposalCard from './MemoryProposal.vue'
import { Button } from '../ui/button'
const props = defineProps<{
  records: MemoryRecord[]
  proposals: MemoryProposal[]
  channelId: string
}>()
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
function proposal(record: MemoryRecord) {
  return props.proposals.find((p) => p.id === record.proposalId)
}
</script>
<template>
  <div v-if="records.length" class="mx-6 my-2 space-y-2 text-xs" data-ui-id="response-memory">
    <template v-for="record in records" :key="record.key">
      <section v-if="record.item" class="space-y-2 rounded border border-border p-3">
        <p class="font-medium">
          {{ record.type === 'automatic' ? 'Automatic save' : 'Approved update' }}
          · {{ record.item.scope }} · v{{ record.item.version }}
        </p>
        <p class="whitespace-pre-wrap wrap-break-word">
          {{ record.item.deleted ? 'Forgotten:' : '' }}
          {{ record.item.content }}
        </p>
        <Button size="sm" variant="outline" :disabled="busy" @click="undo(record)">Undo</Button>
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
