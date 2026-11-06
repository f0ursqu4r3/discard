<script setup lang="ts">
import { computed, ref } from 'vue'
import { serverPillHeight } from './layout'
import { MessagesSquare, Plus } from '@lucide/vue'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Separator } from './ui/separator'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import ProjectContextMenu from './ProjectContextMenu.vue'
import type { ProjectMenuAction } from './project-menu'
import type { ProjectEntry } from '../bridge/api'
import BrandMark from './workspace/BrandMark.vue'
import { cn } from '@/lib/utils'
const props = withDefaults(
  defineProps<{
    projects?: import('@/bridge/api').ProjectEntry[]
    projectId?: string
    project: string
    unread: boolean
    pendingApprovals: number
    reviewCount?: number
    directMessages?: boolean
    compact?: boolean
    busy?: boolean
  }>(),
  { directMessages: false, compact: false },
)
const entries = computed(
  () => props.projects ?? [{ id: props.projectId ?? '', name: props.project, path: '' }],
)
const selected = (id: string) => id === (props.projectId ?? '') && !props.directMessages
const homeHovered = ref(false)
const homeFocused = ref(false)
const hovered = ref<string | null>(null)
const focused = ref<string | null>(null)
const emit = defineEmits<{
  project: [id?: string]
  directMessages: []
  addProject: []
  projectAction: [action: ProjectMenuAction, project: ProjectEntry]
}>()
</script>
<template>
  <TooltipProvider>
    <aside
      data-ui-id="project-rail"
      aria-label="Projects"
      :class="
        cn(
          'flex h-full shrink-0 flex-col items-center border-r border-border/60 bg-canvas',
          compact ? 'w-13' : 'w-18',
        )
      "
    >
      <div
        data-ui-id="rail-home"
        class="relative flex h-16 w-full shrink-0 items-center justify-center"
        @mouseenter="homeHovered = true"
        @mouseleave="homeHovered = false"
        @focusin="homeFocused = true"
        @focusout="homeFocused = false"
      >
        <span
          data-ui-id="dm-selection-pill"
          class="pointer-events-none absolute left-0 w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none"
          :style="{
            height: `calc(var(--spacing) * ${serverPillHeight(directMessages, homeHovered || homeFocused, false, compact ? 10 : 12)})`,
          }"
        />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              data-ui-id="direct-messages-button"
              @click="emit('directMessages')"
              :variant="directMessages ? 'default' : 'secondary'"
              :class="cn('rounded-xl', compact ? 'size-9' : 'size-12')"
              aria-label="Direct messages"
              :aria-current="directMessages ? 'page' : undefined"
            >
              <MessagesSquare />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Direct messages</TooltipContent>
        </Tooltip>
      </div>
      <Separator class="max-w-8" />
      <div
        class="flex min-h-0 w-full flex-1 flex-col items-center gap-3 overflow-x-hidden overflow-y-auto overscroll-contain py-3"
      >
        <div v-for="entry in entries" :key="entry.id" class="w-full">
          <ProjectContextMenu
            :project="entry"
            :busy="busy"
            @action="(action, project) => emit('projectAction', action, project)"
          >
            <div
              @mouseenter="hovered = entry.id"
              @mouseleave="hovered = null"
              @focusin="focused = entry.id"
              @focusout="focused = null"
              :class="
                cn(
                  'group relative flex items-center justify-center',
                  compact ? 'h-10 w-full' : 'h-12 w-full',
                )
              "
            >
              <span
                :data-ui-id="
                  selected(entry.id)
                    ? 'project-selection-pill'
                    : `project-selection-pill-${entry.id}`
                "
                class="pointer-events-none absolute left-0 w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none"
                :style="{
                  height: `calc(var(--spacing) * ${serverPillHeight(selected(entry.id), hovered === entry.id || focused === entry.id, entry.id === projectId && unread, compact ? 10 : 12)})`,
                }"
              />
              <Tooltip>
                <TooltipTrigger as-child>
                  <Button
                    :aria-label="entry.name"
                    :aria-current="selected(entry.id) ? 'page' : undefined"
                    :variant="selected(entry.id) ? 'default' : 'secondary'"
                    :class="cn('relative rounded-xl', compact ? 'size-9' : 'size-12')"
                    @click="emit('project', entry.id)"
                  >
                    <BrandMark
                      v-if="entry.id === projectId || entries.length === 1"
                      :class="cn('text-current', compact ? 'size-6' : 'size-8')"
                    />
                    <span
                      v-if="entry.id !== projectId && entries.length > 1"
                      class="text-sm font-semibold"
                    >
                      {{ entry.name.slice(0, 2).toUpperCase() }}
                    </span>
                    <Badge
                      v-if="entry.id === (projectId ?? '') && pendingApprovals > 0"
                      data-testid="approval-badge"
                      class="absolute -right-1 -bottom-1 bg-warning text-background"
                    >
                      {{ pendingApprovals }}
                    </Badge>
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">{{ entry.name }} · {{ entry.path }}</TooltipContent>
              </Tooltip>
            </div>
          </ProjectContextMenu>
        </div>
        <Separator class="max-w-8" />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              variant="secondary"
              :class="cn('rounded-xl', compact ? 'size-9' : 'size-12')"
              aria-label="Import project"
              @click="emit('addProject')"
            >
              <Plus />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Import project</TooltipContent>
        </Tooltip>
      </div>
    </aside>
  </TooltipProvider>
</template>
