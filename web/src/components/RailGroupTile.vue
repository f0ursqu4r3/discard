<script setup lang="ts">
import { computed, ref } from 'vue'
import { cn } from '@/lib/utils'
import { serverPillHeight } from './layout'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip'
import GroupContextMenu from './GroupContextMenu.vue'
import ProjectAvatar from './workspace/ProjectAvatar.vue'
import type { GroupMenuAction } from './project-menu'
import { railSizes, type RailGroupView, type RailSize } from './rail-layout'

const props = defineProps<{
  group: RailGroupView
  label: string
  size: RailSize
  /** The selected project is in this group. */
  selected: boolean
  unread: boolean
  approvals: number
  /** CSS background of the tile. Undefined keeps the neutral theme background. */
  tint?: string
  /** The tile is the item being dragged. */
  dragging?: boolean
  /** A drop here adds the project to the group. */
  dropInto?: boolean
}>()
const emit = defineEmits<{
  expand: []
  action: [action: GroupMenuAction, arg?: string]
}>()
const hovered = ref(false)
const focused = ref(false)
const preview = computed(() => props.group.projects.slice(0, 4))
</script>
<template>
  <div :class="cn('w-full', dragging && 'opacity-40')">
    <GroupContextMenu :collapsed="true" @action="(action, arg) => emit('action', action, arg)">
      <div
        @mouseenter="hovered = true"
        @mouseleave="hovered = false"
        @focusin="focused = true"
        @focusout="focused = false"
        class="group relative flex w-full items-center justify-center"
      >
        <span
          :data-ui-id="selected ? 'project-selection-pill' : `group-selection-pill-${group.id}`"
          class="pointer-events-none absolute left-0 w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none"
          :style="{
            height: `calc(var(--spacing) * ${serverPillHeight(selected, hovered || focused, unread, railSizes[size].pillUnits)})`,
          }"
        />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              :data-ui-id="`rail-group-${group.id}`"
              :aria-label="label"
              :aria-expanded="false"
              :aria-description="unread ? 'Unread messages' : undefined"
              :aria-current="selected ? 'page' : undefined"
              variant="ghost"
              :style="tint ? { backgroundColor: tint } : undefined"
              :class="
                cn(
                  'relative rounded-xl border-0 p-0 hover:brightness-110',
                  railSizes[size].groupTile,
                  !tint && 'bg-muted',
                  dropInto &&
                    'ring-2 ring-ring ring-offset-1 ring-offset-canvas transition-shadow motion-reduce:transition-none',
                )
              "
              @click="emit('expand')"
            >
              <span
                data-testid="group-preview"
                class="grid size-full grid-cols-2 grid-rows-2 content-center justify-items-center gap-1 p-1"
              >
                <ProjectAvatar
                  v-for="project in preview"
                  :key="project.id"
                  :name="project.name"
                  :avatar="project.avatar"
                  class="size-full rounded-lg text-[7px]"
                />
              </span>
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
            {{ label }}
          </TooltipContent>
        </Tooltip>
      </div>
    </GroupContextMenu>
  </div>
</template>
