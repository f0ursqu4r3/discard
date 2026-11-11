<script setup lang="ts">
import { computed } from 'vue'
import { toast } from 'vue-sonner'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './ui/context-menu'
import { planned } from './project-menu'
import { openContextMenuFromKeyboard } from './context-menu-keyboard'

const props = defineProps<{
  kind: 'branch' | 'worktree' | 'channel'
  name: string
  id?: string
  path?: string | null
  busy?: boolean
  /** The channel can be renamed. `#general` cannot. */
  renamable?: boolean
}>()
const emit = defineEmits<{ open: [] }>()
const label = computed(() => props.kind[0]!.toUpperCase() + props.kind.slice(1))
const plannedEntries = computed(() => {
  // TODO: Create worktrees and rename or delete Git branches with validation and confirmation.
  if (props.kind === 'branch')
    return [planned('Create Worktree…'), planned('Rename Branch…'), planned('Delete Branch…')]
  // TODO: Create a branch in a worktree and remove worktrees after checking for local changes.
  if (props.kind === 'worktree') return [planned('Create Branch…'), planned('Remove Worktree…')]
  // TODO: Persist channel mute settings and archive channels from the sidebar.
  // TODO: Rename text channels with the same name checks as channel creation.
  if (props.renamable)
    return [planned('Mute Channel'), planned('Rename Channel…'), planned('Archive Channel…')]
  return [planned('Mute Channel'), planned('Archive Channel…')]
})
async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
  } catch (error) {
    toast.error('Could not copy to clipboard', { description: String(error) })
  }
}
</script>
<template>
  <ContextMenu>
    <ContextMenuTrigger as-child @keydown="openContextMenuFromKeyboard">
      <slot />
    </ContextMenuTrigger>
    <ContextMenuContent
      :data-ui-id="`${kind}-context-menu`"
      class="min-w-56"
      :collision-padding="8"
    >
      <ContextMenuItem :disabled="busy" @select="!busy && emit('open')">
        Open {{ label }}
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem @select="copy(name)">Copy {{ label }} Name</ContextMenuItem>
      <ContextMenuItem v-if="kind === 'worktree'" :disabled="!path" @select="path && copy(path)">
        Copy Worktree Path
      </ContextMenuItem>
      <ContextMenuItem :disabled="!id" @select="id && copy(id)">
        Copy {{ label }} ID
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem v-for="(entry, index) in plannedEntries" :key="index" disabled>
        {{ entry.label }}
        <span class="ml-auto text-xs text-muted-foreground">Soon</span>
      </ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>
</template>
