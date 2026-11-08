<script setup lang="ts">
import { useSpinnerFrame } from './spinner'
import { useAgentIdentity } from './agents/useAgentIdentity'
import { agentAvatars } from './agents/useAgentAvatar'

const props = withDefaults(
  defineProps<{ label?: string; agentId?: string; avatar?: string | null }>(),
  { label: 'Working' },
)
const frame = useSpinnerFrame()
const identity = useAgentIdentity(
  () => props.agentId,
  () => props.avatar ?? agentAvatars.value[props.agentId ?? ''],
)
</script>
<template>
  <span
    role="img"
    :aria-label="label"
    class="font-mono text-agent"
    :style="agentId ? { '--agent-color': identity.mark } : undefined"
  >
    {{ frame }}
  </span>
</template>
