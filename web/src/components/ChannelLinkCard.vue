<script setup lang="ts">
import { computed, ref } from 'vue'
import { switchChannel } from '@/bridge/api'
import { useStore } from '@/bridge/store'
import type { BlockDto } from '../bridge/types'
import { Button } from './ui/button'
const props = defineProps<{ block: Extract<BlockDto, { type: 'channel_link' }> }>()
const store = useStore()
const busy = ref(false)
const error = ref('')
const label = computed(
  () =>
    `${props.block.direction === 'moved_to' ? 'Moved to' : 'Moved from'} #${props.block.channel_name}`,
)
async function open() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await switchChannel(props.block.channel_id)
    await store.refresh()
  } catch (cause) {
    error.value = `Could not open channel: ${String(cause)}`
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <!-- A report, not a request: it uses the plain card colors, not the protected colors. -->
  <div
    data-ui-id="channel-link"
    class="my-3 flex flex-wrap items-center gap-2 rounded-md border border-border bg-card p-3 text-sm text-card-foreground"
  >
    <p class="min-w-0 break-all font-semibold">{{ label }}</p>
    <Button variant="outline" size="sm" class="ml-auto" :disabled="busy" @click="open">
      Open channel
    </Button>
    <p v-if="error" role="alert" class="w-full break-words">{{ error }}</p>
  </div>
</template>
