<script setup lang="ts">
import { computed } from 'vue'
import type { SnapshotDto } from '@/bridge/types'
import { Badge } from '@/components/ui/badge'
const props = defineProps<{ resumeState: SnapshotDto['resumeState'] }>()
const replay = computed(() =>
  props.resumeState?.kind === 'replay' ? `Fresh replay: ${props.resumeState.reason}` : '',
)
</script>
<template>
  <Badge
    v-if="resumeState?.kind === 'resumed'"
    variant="secondary"
    :title="`${resumeState.agent} continued its saved provider session.`"
  >
    Resumed session
  </Badge>
  <span
    v-else-if="resumeState"
    class="text-muted-foreground"
    :title="`${resumeState.agent} started a new provider session.`"
  >
    {{ replay }}
  </span>
</template>
