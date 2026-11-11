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
import { openContextMenuFromKeyboard } from './context-menu-keyboard'

const props = defineProps<{
  kind: 'branch' | 'worktree' | 'channel'
  name: string
  id?: string
  path?: string | null
  busy?: boolean
  /** The channel can be renamed. `#general` cannot. */
  renamable?: boolean
  /** The channel is muted, so the menu offers Unmute Channel. */
  muted?: boolean
  // A blocked action is disabled and has no Soon label. The app checks every rule again when the action runs.
  /** Archive Channel… is blocked: `#general`, an archived channel, or a busy channel. */
  archiveDisabled?: boolean
  /** Create Worktree… is blocked: the branch is checked out, or its channel has a worktree. */
  createWorktreeDisabled?: boolean
  /** Rename Branch… is blocked: the branch's channel is busy. */
  renameBranchDisabled?: boolean
  /** Delete Branch… is blocked: the branch is checked out, is the default branch, or its channel is busy. */
  deleteBranchDisabled?: boolean
  /** Create Branch… is blocked: a channel that uses the working tree is busy. */
  createBranchDisabled?: boolean
  /** Remove Worktree… is blocked: the main checkout, or a channel that uses the working tree is busy. */
  removeWorktreeDisabled?: boolean
}>()
const emit = defineEmits<{
  open: []
  rename: []
  mute: []
  archive: []
  createWorktree: []
  renameBranch: []
  deleteBranch: []
  createBranch: []
  removeWorktree: []
}>()
const label = computed(() => props.kind[0]!.toUpperCase() + props.kind.slice(1))
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
      <template v-if="kind === 'channel'">
        <ContextMenuItem
          data-ui-id="mute-channel-item"
          :disabled="!id"
          @select="id && emit('mute')"
        >
          {{ muted ? 'Unmute Channel' : 'Mute Channel' }}
        </ContextMenuItem>
        <ContextMenuItem
          v-if="renamable"
          data-ui-id="rename-channel-item"
          :disabled="busy || !id"
          @select="!busy && id && emit('rename')"
        >
          Rename Channel…
        </ContextMenuItem>
        <ContextMenuItem
          data-ui-id="archive-channel-item"
          :disabled="!id || archiveDisabled"
          @select="id && !archiveDisabled && emit('archive')"
        >
          Archive Channel…
        </ContextMenuItem>
      </template>
      <template v-else-if="kind === 'branch'">
        <ContextMenuItem
          data-ui-id="create-worktree-item"
          :disabled="createWorktreeDisabled"
          @select="!createWorktreeDisabled && emit('createWorktree')"
        >
          Create Worktree…
        </ContextMenuItem>
        <ContextMenuItem
          data-ui-id="rename-branch-item"
          :disabled="renameBranchDisabled"
          @select="!renameBranchDisabled && emit('renameBranch')"
        >
          Rename Branch…
        </ContextMenuItem>
        <ContextMenuItem
          data-ui-id="delete-branch-item"
          variant="destructive"
          :disabled="deleteBranchDisabled"
          @select="!deleteBranchDisabled && emit('deleteBranch')"
        >
          Delete Branch…
        </ContextMenuItem>
      </template>
      <template v-else>
        <ContextMenuItem
          data-ui-id="create-branch-item"
          :disabled="createBranchDisabled"
          @select="!createBranchDisabled && emit('createBranch')"
        >
          Create Branch…
        </ContextMenuItem>
        <ContextMenuItem
          data-ui-id="remove-worktree-item"
          variant="destructive"
          :disabled="removeWorktreeDisabled"
          @select="!removeWorktreeDisabled && emit('removeWorktree')"
        >
          Remove Worktree…
        </ContextMenuItem>
      </template>
    </ContextMenuContent>
  </ContextMenu>
</template>
