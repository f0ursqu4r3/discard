<script setup lang="ts">
import { ref, computed } from 'vue'
import { Settings } from '@lucide/vue'
import { Badge } from './ui/badge'
import { Avatar, AvatarFallback } from './ui/avatar'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import { serverInitials, serverPillHeight, settingsTarget } from './layout'
import ContextMenuHost from './ContextMenuHost.vue'
import type { ContextTargetDto } from '../bridge/types'
const props = defineProps<{ project: string; unread: boolean; pendingApprovals: number; reviewCount: number; target?: ContextTargetDto }>()
const emit = defineEmits<{ settings: [] }>()
const hovered = ref(false)
const settingsLabel = computed(() => settingsTarget(props.reviewCount)[1])
</script>
<template>
  <TooltipProvider>
    <aside class="flex h-full w-18 shrink-0 flex-col items-center gap-2 border-r border-border bg-sidebar py-3 text-sidebar-foreground">
      <ContextMenuHost :target="target ?? null"><div class="relative" @mouseenter="hovered = true" @mouseleave="hovered = false">
        <span class="absolute -left-2 top-1/2 w-1 -translate-y-1/2 rounded-full bg-sidebar-foreground" :style="{ height: `calc(var(--spacing) * ${serverPillHeight(true, hovered, unread)})` }" />
        <Tooltip><TooltipTrigger as-child>
          <button type="button" :aria-label="project" class="relative">
            <Avatar class="size-12 rounded-xl"><AvatarFallback class="rounded-xl bg-primary font-bold text-primary-foreground">{{ serverInitials(project) }}</AvatarFallback></Avatar>
            <Badge v-if="pendingApprovals > 0" data-testid="approval-badge" class="absolute -right-1 -bottom-1 bg-warning text-background">{{ pendingApprovals }}</Badge>
          </button>
        </TooltipTrigger><TooltipContent side="right">{{ project }}</TooltipContent></Tooltip>
      </div></ContextMenuHost>
      <Tooltip><TooltipTrigger as-child>
        <Button type="button" variant="ghost" size="icon" class="relative mt-auto" :aria-label="settingsLabel" @click="emit('settings')">
          <Settings /><Badge v-if="reviewCount > 0" class="absolute -right-1 -top-1 bg-warning text-background">{{ reviewCount }}</Badge>
        </Button>
      </TooltipTrigger><TooltipContent side="right">{{ settingsLabel }}</TooltipContent></Tooltip>
    </aside>
  </TooltipProvider>
</template>
