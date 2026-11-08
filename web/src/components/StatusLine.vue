<script setup lang="ts">
import { computed } from 'vue'
import { activeTheme } from '@/bridge/theme'
import { agentIdentity } from './agents/agent-icon'
import BrailleSpinner from './BrailleSpinner.vue'

const props = defineProps<{
  project: string
  branch: string | null
  workingAgents: { id: string; name: string; avatar: string | null }[]
  pendingApprovals: number
}>()
const agents = computed(() =>
  props.workingAgents.map((agent) => ({
    ...agent,
    identity: agentIdentity(agent.avatar, agent.id, activeTheme.value?.tokens ?? {}),
  })),
)
</script>
<template>
  <footer
    data-ui-id="status-line"
    class="flex h-6 shrink-0 items-center gap-4 overflow-hidden whitespace-nowrap border-t bg-background pr-3 font-mono text-[11.5px] text-muted-foreground"
  >
    <span class="flex h-full items-center bg-primary px-2.5 font-bold text-primary-foreground">
      {{ project }}
    </span>
    <span v-if="branch">⎇ {{ branch }}</span>
    <span
      v-for="agent in agents"
      :key="agent.id"
      :style="{ '--agent-color': agent.identity.mark }"
      class="flex items-center gap-1.5"
    >
      <BrailleSpinner />
      {{ agent.name.toLowerCase() }}
    </span>
    <span v-if="pendingApprovals > 0" class="ml-auto text-warning">
      ▲ {{ pendingApprovals }} {{ pendingApprovals === 1 ? 'approval' : 'approvals' }}
    </span>
  </footer>
</template>
