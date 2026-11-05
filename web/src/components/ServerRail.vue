<script setup lang="ts">
import { ref } from 'vue'
import { serverPillHeight } from './layout'
import { MessagesSquare, Plus } from '@lucide/vue'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Separator } from './ui/separator'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import ContextMenuHost from './ContextMenuHost.vue'
import BrandMark from './workspace/BrandMark.vue'
import { cn } from '@/lib/utils'
import type { ContextTargetDto } from '../bridge/types'
withDefaults(defineProps<{ project: string; unread: boolean; pendingApprovals: number; reviewCount?: number; target?: ContextTargetDto; directMessages?: boolean; compact?: boolean }>(), { directMessages: false, compact: false })
const hovered = ref(false)
const focused = ref(false)
const emit = defineEmits<{ project: []; directMessages: []; addProject: [] }>()
</script>
<template>
  <TooltipProvider>
    <aside data-ui-id="project-rail" aria-label="Projects" :class="cn('flex h-full shrink-0 flex-col border-r border-border/60 bg-canvas', compact ? 'w-13' : 'w-18')">
      <div data-ui-id="rail-home" class="flex h-16 shrink-0 items-center justify-center border-b border-border/60">
        <Tooltip><TooltipTrigger as-child>
          <Button data-ui-id="direct-messages-button" :variant="directMessages ? 'default' : 'secondary'" :class="cn('rounded-xl', compact ? 'size-9' : 'size-12')" aria-label="Direct messages" :aria-current="directMessages ? 'page' : undefined" @click="emit('directMessages')"><MessagesSquare /></Button>
        </TooltipTrigger><TooltipContent side="right">Direct messages</TooltipContent></Tooltip>
      </div>
      <div class="flex min-h-0 flex-1 flex-col items-center gap-3 overflow-x-hidden overflow-y-auto overscroll-contain py-3">
        <div class="w-full"><ContextMenuHost :target="target ?? null">
          <div @mouseenter="hovered = true" @mouseleave="hovered = false" @focusin="focused = true" @focusout="focused = false" :class="cn('group relative flex items-center justify-center', compact ? 'h-10 w-full' : 'h-12 w-full')">
            <span data-ui-id="project-selection-pill" class="pointer-events-none absolute left-0 w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none" :style="{ height: `calc(var(--spacing) * ${serverPillHeight(!directMessages, hovered || focused, unread, compact ? 10 : 12)})` }" />
            <Tooltip><TooltipTrigger as-child>
              <Button :aria-label="project" :aria-current="!directMessages ? 'page' : undefined" :variant="directMessages ? 'secondary' : 'default'" :class="cn('relative rounded-xl', compact ? 'size-9' : 'size-12')" @click="emit('project')">
                <BrandMark :class="cn('text-current', compact ? 'size-6' : 'size-8')" />
                <Badge v-if="pendingApprovals > 0" data-testid="approval-badge" class="absolute -right-1 -bottom-1 bg-warning text-background">{{ pendingApprovals }}</Badge>
              </Button>
            </TooltipTrigger><TooltipContent side="right">{{ project }}</TooltipContent></Tooltip>
          </div>
        </ContextMenuHost></div>
        <Separator class="max-w-8" />
        <Tooltip><TooltipTrigger as-child><Button variant="secondary" :class="cn('rounded-xl', compact ? 'size-9' : 'size-12')" aria-label="Manage projects" @click="emit('addProject')"><Plus /></Button></TooltipTrigger><TooltipContent side="right">Manage projects</TooltipContent></Tooltip>
      </div>
    </aside>
  </TooltipProvider>
</template>
