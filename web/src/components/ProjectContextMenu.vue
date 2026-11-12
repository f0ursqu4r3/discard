<script setup lang="ts">
import { computed, ref } from 'vue'
import { toast } from 'vue-sonner'
import { contextMenu, pluginMenuAction, type ProjectEntry } from '@/bridge/api'
import type { MenuEntryDto } from '@/bridge/types'
import {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuSeparator,
  ContextMenuItem,
} from './ui/context-menu'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  DialogClose,
} from './ui/dialog'
import { Button } from './ui/button'
import ProjectMenuItems from './ProjectMenuItems.vue'
import {
  projectMenu,
  type MenuAction,
  type ProjectMenuAction,
  type ProjectRailContext,
  type RailMenuAction,
} from './project-menu'
import { openContextMenuFromKeyboard } from './context-menu-keyboard'
const props = defineProps<{ project: ProjectEntry; busy?: boolean; rail?: ProjectRailContext }>()
const emit = defineEmits<{
  action: [action: ProjectMenuAction, project: ProjectEntry]
  rail: [action: RailMenuAction, project: ProjectEntry, arg?: string]
}>()
const railActions: MenuAction[] = [
  'group-none',
  'group-join',
  'group-new',
  'move-first',
  'move-last',
]
type PluginEntry = Extract<MenuEntryDto, { kind: 'plugin' }>
const plugins = ref<PluginEntry[]>([])
const confirmRemove = ref(false)
const target = computed(() => ({ type: 'server' as const, ...props.project }))
const entries = computed(() => projectMenu(!!props.busy, props.rail))
let request = 0
async function openChanged(open: boolean) {
  const id = ++request
  plugins.value = []
  if (!open) return
  const projectId = props.project.id
  try {
    const result = await contextMenu(target.value)
    if (id === request && projectId === props.project.id)
      plugins.value = result.filter((entry): entry is PluginEntry => entry.kind === 'plugin')
  } catch (error) {
    if (id === request) toast.error('Could not load plugin actions', { description: String(error) })
  }
}
function select(action: MenuAction, arg?: string) {
  if (railActions.includes(action)) emit('rail', action as RailMenuAction, props.project, arg)
  else if (props.busy && ['settings', 'remove'].includes(action)) return
  else if (action === 'remove') confirmRemove.value = true
  else emit('action', action as ProjectMenuAction, props.project)
}
async function runPlugin(entry: PluginEntry) {
  try {
    await pluginMenuAction(entry.plugin, entry.index, target.value)
  } catch (error) {
    toast.error('Could not complete plugin action', { description: String(error) })
  }
}
</script>
<template>
  <ContextMenu @update:open="openChanged">
    <ContextMenuTrigger as-child @keydown="openContextMenuFromKeyboard">
      <slot />
    </ContextMenuTrigger>
    <ContextMenuContent class="min-w-64" :collision-padding="8">
      <ProjectMenuItems :entries="entries" @action="select" />
      <ContextMenuSeparator v-if="plugins.length" />
      <ContextMenuItem
        v-for="entry in plugins"
        :key="`${entry.plugin}:${entry.index}`"
        @select="runPlugin(entry)"
      >
        {{ entry.label }}
        <span class="ml-auto text-xs text-muted-foreground">{{ entry.plugin }}</span>
      </ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>
  <Dialog v-model:open="confirmRemove">
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Remove {{ project.name }}?</DialogTitle>
        <DialogDescription>
          This removes the project from the rail. Files, messages, and memory stay saved. Import the
          folder again to restore it.
        </DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <DialogClose as-child><Button variant="ghost">Cancel</Button></DialogClose>
        <DialogClose as-child>
          <Button
            variant="destructive"
            :disabled="busy"
            @click="!busy && emit('action', 'remove', project)"
          >
            Remove project
          </Button>
        </DialogClose>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
