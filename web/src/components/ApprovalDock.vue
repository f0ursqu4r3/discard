<script setup lang="ts">
import { computed } from 'vue'
import type { PromptDto } from '../bridge/types'
import ApprovalCard from './ApprovalCard.vue'

const props = defineProps<{ channelId: string; prompts: PromptDto[] }>()
const pending = computed(() => props.prompts.filter((prompt) => prompt.view === 'pending'))
const first = computed(() => pending.value[0])
</script>

<template>
  <div v-if="first" data-ui-id="approval-dock" class="space-y-1">
    <p
      v-if="pending.length > 1"
      data-ui-id="approval-dock-count"
      class="px-1 text-xs font-medium"
      :style="{ color: 'var(--protected-fg)' }"
    >
      1 of {{ pending.length }}
    </p>
    <ApprovalCard :key="first.approvalId" :channel-id="channelId" :prompt="first" />
  </div>
</template>
