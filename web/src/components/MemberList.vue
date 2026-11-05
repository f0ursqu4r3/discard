<script setup lang="ts">
import { computed } from 'vue'
import { Avatar, AvatarFallback } from './ui/avatar'
import { serverInitials } from './layout'
import ContextMenuHost from './ContextMenuHost.vue'
import type { ContextTargetDto } from '../bridge/types'
const props = defineProps<{ agent: string; status: string; provider: string; target?: ContextTargetDto }>()
const statusLabel = computed(() => ({ idle: 'Idle', working: 'Working', awaiting_approval: 'Waiting for approval', offline: 'Offline' }[props.status] ?? props.status))
const statusColor = computed(() => ({ idle: 'bg-muted-foreground', working: 'bg-success', awaiting_approval: 'bg-warning', offline: 'bg-border' }[props.status] ?? 'bg-border'))
</script>
<template>
  <aside class="h-full bg-sidebar p-3 text-sidebar-foreground">
    <h2 class="text-xs text-muted-foreground">AGENTS</h2>
    <ContextMenuHost :target="target ?? null"><div class="mt-2 flex items-center gap-2">
      <div class="relative"><Avatar><AvatarFallback>{{ serverInitials(agent) }}</AvatarFallback></Avatar><span data-testid="presence-dot" class="absolute -bottom-1 -right-1 size-2 rounded-full border-2 border-sidebar" :class="statusColor" /></div>
      <div><p>{{ agent }}</p><p class="text-xs text-muted-foreground">{{ statusLabel }} · {{ provider }}</p></div>
    </div></ContextMenuHost>
    <slot />
  </aside>
</template>
