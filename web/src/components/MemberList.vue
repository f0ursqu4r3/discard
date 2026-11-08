<script setup lang="ts">
import { computed } from 'vue'
import BrailleSpinner from './BrailleSpinner.vue'
import { Users, UserRound } from '@lucide/vue'
import AgentAvatar from './agents/AgentAvatar.vue'
import { useAgentIdentity } from './agents/useAgentIdentity'
import { Button } from './ui/button'
import { Card, CardHeader, CardTitle, CardContent, CardFooter } from './ui/card'
import ContextMenuHost from './ContextMenuHost.vue'
import type { ContextTargetDto } from '../bridge/types'
const props = defineProps<{
  agentId?: string
  agent: string
  avatar?: string | null
  status: string
  provider: string
  target?: ContextTargetDto
}>()
const identityId = computed(
  () => props.agentId ?? (props.target?.type === 'agent' ? props.target.id : undefined),
)
const identity = useAgentIdentity(identityId, () => props.avatar)
const emit = defineEmits<{ settings: [] }>()
const statusLabel = computed(
  () =>
    ({
      idle: 'Idle',
      working: 'Working',
      awaiting_approval: 'Waiting for approval',
      offline: 'Offline',
    })[props.status] ?? props.status,
)
const statusColor = computed(
  () =>
    ({
      idle: 'bg-muted-foreground',
      working: 'bg-success',
      awaiting_approval: 'bg-warning',
      offline: 'bg-border',
    })[props.status] ?? 'bg-border',
)
</script>
<template>
  <Card data-ui-id="project-members" size="sm">
    <CardHeader>
      <CardTitle class="section-label">
        <Users class="size-4 text-muted-foreground" />
        Members
      </CardTitle>
    </CardHeader>
    <CardContent>
      <ContextMenuHost :target="target ?? null">
        <div class="flex items-center gap-3" :style="{ '--agent-color': identity.mark }">
          <AgentAvatar :agent-id="identityId" :name="agent" :avatar="avatar" class="size-8" />
          <div class="min-w-0 flex-1">
            <p class="truncate text-xs font-semibold" :style="{ color: identity.text }">
              {{ agent }}
            </p>
            <p class="text-xs text-muted-foreground">{{ statusLabel }} · {{ provider }}</p>
          </div>
          <BrailleSpinner v-if="status === 'working'" />
          <span
            v-else
            data-testid="presence-dot"
            class="size-1.5 shrink-0 rounded-full"
            :class="statusColor"
            :aria-label="statusLabel"
          />
        </div>
      </ContextMenuHost>
    </CardContent>
    <CardFooter>
      <Button variant="outline" size="sm" class="w-full" @click="emit('settings')">
        <UserRound data-icon="inline-start" />
        View profile
      </Button>
    </CardFooter>
    <slot />
  </Card>
</template>
