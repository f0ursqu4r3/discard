<script setup lang="ts">
import { ref } from 'vue'
import { cn } from '@/lib/utils'
import type { ProjectEntry } from '../bridge/api'
import { serverPillHeight } from './layout'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip'
import ProjectContextMenu from './ProjectContextMenu.vue'
import ProjectAvatar from './workspace/ProjectAvatar.vue'
import type { ProjectMenuAction, ProjectRailContext, RailMenuAction } from './project-menu'
import { railSizes, type RailSize } from './rail-layout'

defineProps<{
  project: ProjectEntry
  size: RailSize
  selected: boolean
  unread: boolean
  approvals: number
  busy?: boolean
  rail?: ProjectRailContext
  /** The tile sits in an expanded group, whose container is inset from the rail edge. */
  inGroup?: boolean
  /** The tile is the item being dragged. */
  dragging?: boolean
  /** A drop here makes a group. */
  dropInto?: boolean
}>()
const emit = defineEmits<{
  select: []
  action: [action: ProjectMenuAction, project: ProjectEntry]
  rail: [action: RailMenuAction, project: ProjectEntry, arg?: string]
}>()
const hovered = ref(false)
const focused = ref(false)
</script>
<template>
  <div :class="cn('w-full', dragging && 'opacity-40')">
    <ProjectContextMenu
      :project="project"
      :busy="busy"
      :rail="rail"
      @action="(action, target) => emit('action', action, target)"
      @rail="(action, target, arg) => emit('rail', action, target, arg)"
    >
      <div
        @mouseenter="hovered = true"
        @mouseleave="hovered = false"
        @focusin="focused = true"
        @focusout="focused = false"
        :class="
          cn(
            'group relative flex w-full items-center justify-center',
            inGroup ? railSizes[size].groupRow : railSizes[size].row,
          )
        "
      >
        <span
          :data-ui-id="selected ? 'project-selection-pill' : `project-selection-pill-${project.id}`"
          :class="
            cn(
              'pointer-events-none absolute w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none',
              inGroup ? railSizes[size].groupPill : 'left-0',
            )
          "
          :style="{
            height: `calc(var(--spacing) * ${serverPillHeight(selected, hovered || focused, unread, railSizes[size].pillUnits)})`,
          }"
        />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              :aria-label="project.name"
              :aria-description="unread ? 'Unread messages' : undefined"
              :aria-current="selected ? 'page' : undefined"
              variant="ghost"
              :class="
                cn(
                  'relative rounded-lg border-0 p-0 hover:brightness-110',
                  railSizes[size].tile,
                  dropInto &&
                    'ring-2 ring-ring ring-offset-1 ring-offset-canvas transition-shadow motion-reduce:transition-none',
                )
              "
              @click="emit('select')"
            >
              <ProjectAvatar :name="project.name" :avatar="project.avatar" class="size-full" />
              <Badge
                v-if="approvals > 0"
                data-testid="approval-badge"
                class="absolute -right-1 -bottom-1 bg-warning text-background"
              >
                {{ approvals }}
              </Badge>
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">
            {{ project.name }}
          </TooltipContent>
        </Tooltip>
      </div>
    </ProjectContextMenu>
  </div>
</template>
