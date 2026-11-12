<script setup lang="ts">
import { FolderOpen } from '@lucide/vue'
import { cn } from '@/lib/utils'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip'
import GroupContextMenu from './GroupContextMenu.vue'
import type { GroupMenuAction } from './project-menu'
import type { RailSize } from './rail-layout'

defineProps<{
  groupId: string
  label: string
  size: RailSize
  /** CSS background of the container. Undefined keeps the neutral theme background. */
  tint?: string
  /** The container is the group being dragged. */
  dragging?: boolean
  /** A drop here lands inside the group. */
  dropInto?: boolean
}>()
const emit = defineEmits<{
  collapse: []
  action: [action: GroupMenuAction, arg?: string]
  /** The pointer went down on the header, which drags the whole group. */
  headerPointerdown: [event: PointerEvent]
}>()
</script>
<template>
  <div :class="cn('w-full', dragging && 'opacity-40')">
    <div
      :data-ui-id="`rail-group-container-${groupId}`"
      :style="tint ? { backgroundColor: tint } : undefined"
      :class="
        cn(
          'mx-1 flex w-[calc(100%-0.5rem)] flex-col items-center gap-2 rounded-xl py-1',
          !tint && 'bg-muted/60',
          dropInto && 'ring-2 ring-ring transition-shadow motion-reduce:transition-none',
        )
      "
    >
      <GroupContextMenu :collapsed="false" @action="(action, arg) => emit('action', action, arg)">
        <div
          :class="cn('flex w-full items-center justify-center', size === 'mini' ? 'h-5' : 'h-6')"
          @pointerdown="emit('headerPointerdown', $event)"
        >
          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                :data-ui-id="`rail-group-header-${groupId}`"
                :aria-label="label"
                :aria-expanded="true"
                variant="ghost"
                class="size-6 rounded-md p-0 text-muted-foreground hover:text-foreground"
                @click="emit('collapse')"
              >
                <FolderOpen class="size-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">
              {{ label }}
            </TooltipContent>
          </Tooltip>
        </div>
      </GroupContextMenu>
      <slot />
    </div>
  </div>
</template>
