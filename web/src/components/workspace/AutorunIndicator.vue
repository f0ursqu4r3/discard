<script setup lang="ts">
import { computed } from 'vue'
import type { SnapshotDto } from '@/bridge/types'
import { Badge } from '@/components/ui/badge'
const props = defineProps<{ autorun: SnapshotDto['autorun'] }>()
const off = computed(() =>
  props.autorun?.reason ? `Autorun off: ${props.autorun.reason}` : 'Autorun off',
)
</script>
<template>
  <Badge v-if="autorun?.state === 'on'" variant="secondary">Autorun</Badge>
  <span v-else-if="autorun" class="text-muted-foreground">{{ off }}</span>
</template>
