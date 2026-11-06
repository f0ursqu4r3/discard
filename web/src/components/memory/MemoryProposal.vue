<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { MemoryProposal, MemoryResolution } from './types'
import { Button } from '../ui/button'
const props = defineProps<{
  proposal: MemoryProposal
  channelId?: string | null
}>()
const emit = defineEmits<{ changed: [] }>()
const error = ref('')
const busy = ref(false)
const settled = ref(false)
const notice = ref('')
async function resolve(approve: boolean) {
  if (settled.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const outcome = await invoke<MemoryResolution>('memory_resolve', {
      id: props.proposal.id,
      approve,
      channelId: props.channelId ?? null,
    })
    settled.value = true
    notice.value =
      outcome?.warning ?? (approve ? 'Memory is approved and saved.' : 'Memory proposal rejected.')
    emit('changed')
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <section class="space-y-2 rounded border border-border p-3 text-sm" data-protected="true">
    <p class="font-medium">Memory proposal · {{ proposal.action }} · {{ proposal.scope }}</p>
    <p class="whitespace-pre-wrap wrap-break-word">{{ proposal.content }}</p>
    <p class="text-xs text-muted-foreground">Approve only if this saved information is correct.</p>
    <div class="flex gap-2">
      <Button size="sm" :disabled="busy || settled" @click="resolve(true)">Approve</Button>
      <Button size="sm" variant="outline" :disabled="busy || settled" @click="resolve(false)">
        Reject
      </Button>
    </div>
    <p v-if="notice" role="status">{{ notice }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
  </section>
</template>
