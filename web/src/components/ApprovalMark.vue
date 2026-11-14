<script setup lang="ts">
import { computed } from 'vue'
import { approvalMark, type ApprovalStatus } from './messages'

const props = defineProps<{ status?: ApprovalStatus }>()
const mark = computed(() => approvalMark(props.status))
</script>

<template>
  <span
    v-if="mark"
    data-ui-id="approval-mark"
    role="img"
    :aria-label="mark.label"
    :class="mark.pending ? 'font-medium' : 'text-muted-foreground'"
    :style="mark.pending ? { color: 'var(--protected-fg)' } : undefined"
  >
    {{ mark.text }}
  </span>
</template>
