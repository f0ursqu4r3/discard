<script setup lang="ts">
import type { ThreadState } from '../bridge/types'
import BrailleSpinner from './BrailleSpinner.vue'
import { threadStates } from './threads'
defineProps<{ state: ThreadState; agentId?: string }>()
</script>
<template>
  <span
    data-ui-id="thread-state"
    :data-state="state"
    class="inline-flex shrink-0 items-center gap-1"
    :class="threadStates[state].class"
  >
    <!-- The visible label names the state. -->
    <BrailleSpinner v-if="state === 'running'" aria-hidden="true" :agent-id="agentId" />
    <component :is="threadStates[state].icon" v-else class="size-3" aria-hidden="true" />
    <span>{{ threadStates[state].label }}</span>
  </span>
</template>
