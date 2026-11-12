<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppearance } from '@/bridge/appearance'
import { activeTheme } from '@/bridge/theme'
import { serverPillHeight } from './layout'
import { Folder, MessagesSquare, Plus } from '@lucide/vue'
import { Button } from './ui/button'
import { Separator } from './ui/separator'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import RailProjectTile from './RailProjectTile.vue'
import RailGroupTile from './RailGroupTile.vue'
import RailGroupContainer from './RailGroupContainer.vue'
import RenameGroupDialog from './RenameGroupDialog.vue'
import type { GroupMenuAction, ProjectMenuAction, RailMenuAction } from './project-menu'
import type { ProjectEntry, RailLayoutDto } from '../bridge/api'
import type { ChannelDto } from '../bridge/types'
import ProjectAvatar from './workspace/ProjectAvatar.vue'
import { channelApprovals } from './threads'
import { useRailDrag } from './rail-drag'
import {
  groupLabel,
  groupOf,
  groupTint,
  railSizes,
  resolveRailView,
  type RailChange,
  type RailGroupView,
  type RailSize,
} from './rail-layout'
import { cn } from '@/lib/utils'
const props = withDefaults(
  defineProps<{
    projects?: import('@/bridge/api').ProjectEntry[]
    /** The saved rail order and groups. Without it, every project is ungrouped. */
    layout?: RailLayoutDto | null
    projectId?: string
    channels?: ChannelDto[]
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
const appearance = useAppearance()
const mini = computed(() => appearance.value.miniProjectRail)
const size = computed<RailSize>(() => (mini.value ? 'mini' : props.compact ? 'compact' : 'default'))
const entries = computed(
  () => props.projects ?? [{ id: props.projectId ?? '', name: props.project, path: '' }],
)
const view = computed(() => resolveRailView(entries.value, props.layout))
const groups = computed(() =>
  view.value.filter((item): item is { kind: 'group' } & RailGroupView => item.kind === 'group'),
)
const groupOptions = computed(() =>
  groups.value.map((group) => ({ id: group.id, label: groupLabel(group) })),
)
const railContext = (id: string) => ({
  groups: groupOptions.value,
  groupId: groupOf(view.value, id)?.id ?? null,
})
const selected = (id: string) => id === (props.projectId ?? '') && !props.directMessages
// A muted channel does not count toward the unread marker of its project. Pending approvals still count.
const projectUnread = (id: string) =>
  props.channels?.some((channel) => channel.projectId === id && channel.unread && !channel.muted) ??
  (id === (props.projectId ?? '') && props.unread)
const projectApprovals = (id: string) =>
  props.channels?.reduce(
    (total, channel) => total + (channel.projectId === id ? channelApprovals(channel) : 0),
    0,
  ) ?? (id === (props.projectId ?? '') ? props.pendingApprovals : 0)
// A collapsed group shows what its members would show: any unread member, and the sum of approvals.
const groupSelected = (group: RailGroupView) =>
  group.projects.some((project) => selected(project.id))
const groupUnread = (group: RailGroupView) =>
  group.projects.some((project) => projectUnread(project.id))
const groupApprovals = (group: RailGroupView) =>
  group.projects.reduce((total, project) => total + projectApprovals(project.id), 0)
const tokens = computed(() => activeTheme.value?.tokens ?? {})
const tileTint = (group: RailGroupView) => groupTint(group.color, tokens.value, 30)
const containerTint = (group: RailGroupView) => groupTint(group.color, tokens.value, 18)
const directUnread = computed(
  () =>
    props.channels?.some(
      (channel) => channel.projectId === null && channel.unread && !channel.muted,
    ) ?? false,
)
const homeHovered = ref(false)
const homeFocused = ref(false)
const emit = defineEmits<{
  project: [id?: string]
  directMessages: []
  addProject: []
  projectAction: [action: ProjectMenuAction, project: ProjectEntry]
  /** A change to the saved rail: order, groups, or group settings. */
  railChange: [change: RailChange]
}>()
const scroller = ref<HTMLElement | null>(null)
const { drag, onPointerDown, onClickCapture } = useRailDrag({
  scroller,
  view,
  apply: (change) => emit('railChange', change),
})
const dragging = computed(() => (drag.value?.active ? drag.value : null))
const draggedProject = computed(() => {
  const source = dragging.value?.source
  return source?.kind === 'project'
    ? entries.value.find((project) => project.id === source.id)
    : undefined
})
const draggedGroup = computed(() => {
  const source = dragging.value?.source
  return source?.kind === 'group' ? groups.value.find((group) => group.id === source.id) : undefined
})
// A drop target shows only when the drop would change the rail.
const dropIntoProject = (id: string) => {
  const target = dragging.value?.change ? dragging.value.target : null
  return target?.type === 'into-project' && target.projectId === id
}
const dropIntoGroup = (id: string) => {
  const target = dragging.value?.change ? dragging.value.target : null
  return (
    (target?.type === 'into-group' && target.groupId === id) ||
    (target?.type === 'reorder' && target.groupId === id)
  )
}
const isDragged = (kind: 'project' | 'group', id: string) =>
  dragging.value?.source.kind === kind && dragging.value.source.id === id
const renamingId = ref<string | null>(null)
const renaming = computed(() => groups.value.find((group) => group.id === renamingId.value))
function onProjectAction(action: ProjectMenuAction, project: ProjectEntry) {
  emit('projectAction', action, project)
}
function onRailAction(action: RailMenuAction, project: ProjectEntry, arg?: string) {
  const current = groupOf(view.value, project.id)
  const move = (groupId: string | null, index: number) =>
    emit('railChange', { type: 'move-project', projectId: project.id, groupId, index })
  if (action === 'group-new')
    emit('railChange', {
      type: 'create-group',
      projectId: project.id,
      // The new group has no name yet. Ask for one.
      created: (groupId) => (renamingId.value = groupId),
    })
  else if (action === 'group-none') {
    // The project lands right after the group it leaves.
    if (current)
      move(
        null,
        view.value.findIndex((item) => item.kind === 'group' && item.id === current.id) + 1,
      )
  } else if (action === 'group-join') {
    const target = groups.value.find((group) => group.id === arg)
    if (target && target.id !== current?.id)
      move(target.id, target.projects.filter((member) => member.id !== project.id).length)
  } else {
    // First and last in the group, or in the top-level list when the project has no group.
    const ids = current
      ? current.projects.map((member) => member.id)
      : view.value.map((item) => (item.kind === 'project' ? item.project.id : item.id))
    const first = action === 'move-first'
    if (ids[first ? 0 : ids.length - 1] !== project.id)
      move(current?.id ?? null, first ? 0 : ids.length - 1)
  }
}
function onGroupAction(group: RailGroupView, action: GroupMenuAction, arg?: string) {
  if (action === 'group-rename') renamingId.value = group.id
  else if (action === 'group-color')
    emit('railChange', { type: 'color-group', groupId: group.id, color: arg ?? null })
  else if (action === 'group-toggle')
    emit('railChange', { type: 'collapse-group', groupId: group.id, collapsed: !group.collapsed })
  else if (action === 'group-ungroup') emit('railChange', { type: 'ungroup', groupId: group.id })
}
const setCollapsed = (group: RailGroupView, collapsed: boolean) =>
  emit('railChange', { type: 'collapse-group', groupId: group.id, collapsed })
</script>
<template>
  <TooltipProvider>
    <aside
      data-ui-id="project-rail"
      aria-label="Projects"
      :class="
        cn(
          'flex h-full shrink-0 flex-col items-center border-r border-border/60 bg-canvas',
          mini ? 'w-12' : compact ? 'w-13' : 'w-18',
        )
      "
    >
      <div
        data-ui-id="rail-home"
        class="relative flex my-2 w-full shrink-0 items-center justify-center"
        @mouseenter="homeHovered = true"
        @mouseleave="homeHovered = false"
        @focusin="homeFocused = true"
        @focusout="homeFocused = false"
      >
        <span
          data-ui-id="dm-selection-pill"
          class="pointer-events-none absolute left-0 w-1 rounded-r-full bg-foreground transition-[height] duration-150 motion-reduce:transition-none"
          :style="{
            height: `calc(var(--spacing) * ${serverPillHeight(directMessages, homeHovered || homeFocused, directUnread, mini ? 8 : compact ? 10 : 12)})`,
          }"
        />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              data-ui-id="direct-messages-button"
              @click="emit('directMessages')"
              :variant="directMessages ? 'default' : 'secondary'"
              :class="
                cn(
                  'rounded-lg',
                  mini ? 'size-8 rounded-lg border-0 p-0' : compact ? 'size-9' : 'size-12',
                )
              "
              aria-label="Direct messages"
              :aria-description="directUnread ? 'Unread messages' : undefined"
              :aria-current="directMessages ? 'page' : undefined"
            >
              <MessagesSquare />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Direct messages</TooltipContent>
        </Tooltip>
      </div>
      <Separator :class="mini ? 'max-w-6' : 'max-w-8'" />
      <div
        ref="scroller"
        data-ui-id="rail-projects"
        :class="
          cn(
            'flex min-h-0 w-full flex-1 flex-col items-center gap-2 overflow-x-hidden overflow-y-auto overscroll-contain py-2',
            dragging && 'cursor-grabbing select-none',
          )
        "
        @click.capture="onClickCapture"
      >
        <template v-for="item in view" :key="item.kind === 'project' ? item.project.id : item.id">
          <RailProjectTile
            v-if="item.kind === 'project'"
            :data-rail-entry="`project:${item.project.id}`"
            :project="item.project"
            :size="size"
            :selected="selected(item.project.id)"
            :unread="projectUnread(item.project.id)"
            :approvals="projectApprovals(item.project.id)"
            :busy="busy"
            :rail="railContext(item.project.id)"
            :dragging="isDragged('project', item.project.id)"
            :drop-into="dropIntoProject(item.project.id)"
            @pointerdown="onPointerDown($event, { kind: 'project', id: item.project.id })"
            @select="emit('project', item.project.id)"
            @action="onProjectAction"
            @rail="onRailAction"
          />
          <RailGroupTile
            v-else-if="item.collapsed"
            :data-rail-entry="`group:${item.id}`"
            :group="item"
            :label="groupLabel(item)"
            :size="size"
            :selected="groupSelected(item) && !directMessages"
            :unread="groupUnread(item)"
            :approvals="groupApprovals(item)"
            :tint="tileTint(item)"
            :dragging="isDragged('group', item.id)"
            :drop-into="dropIntoGroup(item.id)"
            @pointerdown="onPointerDown($event, { kind: 'group', id: item.id })"
            @expand="setCollapsed(item, false)"
            @action="(action, arg) => onGroupAction(item, action, arg)"
          />
          <RailGroupContainer
            v-else
            :data-rail-entry="`group:${item.id}`"
            :group-id="item.id"
            :label="groupLabel(item)"
            :size="size"
            :tint="containerTint(item)"
            :dragging="isDragged('group', item.id)"
            :drop-into="dropIntoGroup(item.id)"
            @header-pointerdown="onPointerDown($event, { kind: 'group', id: item.id })"
            @collapse="setCollapsed(item, true)"
            @action="(action, arg) => onGroupAction(item, action, arg)"
          >
            <RailProjectTile
              v-for="member in item.projects"
              :key="member.id"
              :data-rail-member="member.id"
              :project="member"
              :size="size"
              in-group
              :selected="selected(member.id)"
              :unread="projectUnread(member.id)"
              :approvals="projectApprovals(member.id)"
              :busy="busy"
              :rail="railContext(member.id)"
              :dragging="isDragged('project', member.id)"
              @pointerdown="onPointerDown($event, { kind: 'project', id: member.id })"
              @select="emit('project', member.id)"
              @action="onProjectAction"
              @rail="onRailAction"
            />
          </RailGroupContainer>
        </template>
        <Separator :class="mini ? 'max-w-6' : 'max-w-8'" />
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              variant="secondary"
              :class="
                cn(
                  'rounded-lg',
                  mini ? 'size-8 rounded-lg border-0 p-0' : compact ? 'size-9' : 'size-12',
                )
              "
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
    <div
      v-if="dragging?.line"
      data-testid="rail-drop-line"
      class="pointer-events-none fixed z-50 h-0.5 rounded-full bg-ring"
      :style="{
        top: `${dragging.line.top - 1}px`,
        left: `${dragging.line.left}px`,
        width: `${dragging.line.width}px`,
      }"
    />
    <div
      v-if="dragging && (draggedProject || draggedGroup)"
      data-testid="rail-drag-ghost"
      :class="
        cn(
          'pointer-events-none fixed z-50 -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-lg opacity-80 shadow-lg',
          railSizes[size].tile,
        )
      "
      :style="{ top: `${dragging.y}px`, left: `${dragging.x}px` }"
    >
      <ProjectAvatar
        v-if="draggedProject"
        :name="draggedProject.name"
        :avatar="draggedProject.avatar"
        class="size-full"
      />
      <span
        v-else-if="draggedGroup"
        class="flex size-full items-center justify-center bg-muted text-muted-foreground"
        :style="tileTint(draggedGroup) ? { backgroundColor: tileTint(draggedGroup) } : undefined"
      >
        <Folder class="size-1/2" />
      </span>
    </div>
    <RenameGroupDialog
      v-if="renaming"
      :open="true"
      :name="renaming.name"
      :placeholder="groupLabel(renaming)"
      @update:open="(open: boolean) => !open && (renamingId = null)"
      @save="
        (name: string) => emit('railChange', { type: 'rename-group', groupId: renaming!.id, name })
      "
    />
  </TooltipProvider>
</template>
