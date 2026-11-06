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
import { projectMenu, type ProjectMenuAction } from './project-menu'
const props = defineProps<{ project: ProjectEntry; busy?: boolean }>()
const emit = defineEmits<{ action: [action: ProjectMenuAction, project: ProjectEntry] }>()
type PluginEntry = Extract<MenuEntryDto, { kind: 'plugin' }>
const plugins = ref<PluginEntry[]>([])
const confirmRemove = ref(false)
const target = computed(() => ({ type: 'server' as const, ...props.project }))
const entries = computed(() => projectMenu(!!props.busy))
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
function select(action: ProjectMenuAction) {
  if (props.busy && ['settings', 'remove'].includes(action)) return
  if (action === 'remove') confirmRemove.value = true
  else emit('action', action, props.project)
}
function openFromKeyboard(event: KeyboardEvent) {
  if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return
  event.preventDefault()
  const element = event.currentTarget as HTMLElement
  const bounds = element.getBoundingClientRect()
  element.dispatchEvent(
    new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: bounds.right,
      clientY: bounds.top,
    }),
  )
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
    <ContextMenuTrigger as-child @keydown="openFromKeyboard"><slot /></ContextMenuTrigger>
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
