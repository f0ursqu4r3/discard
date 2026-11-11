<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ContextTargetDto, MenuEntryDto } from '../bridge/types'
import { contextMenu, pluginMenuAction, runAction } from '../bridge/api'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './ui/context-menu'
const props = defineProps<{
  target: ContextTargetDto | null
  replyEnabled?: boolean
  /** The thread item of a message menu. Left out where a message has no thread action. */
  threadAction?: { label: string; disabled: boolean }
  /** The "Post to channel" item of a thread message. `posted` shows the done state. */
  postAction?: { posted: boolean; disabled: boolean }
  /** The "Remove from channel" item of a posted reference row. Left out on any other message. */
  removeAction?: { label: string }
}>()
const emit = defineEmits<{ reply: []; thread: []; post: []; remove: [] }>()
// Message text, code, diffs, and tool calls can be selected. Other targets keep the default.
const selectable = computed(() =>
  ['message', 'code_block', 'diff_block', 'tool_call'].includes(props.target?.type ?? ''),
)
const replying = ref(false)
function reply() {
  if (!props.replyEnabled) return
  replying.value = true
}
function closeAutoFocus(event: Event) {
  if (!replying.value) return
  event.preventDefault()
  replying.value = false
  // Wait for the menu's focus trap to close before focusing the composer.
  if (props.replyEnabled) emit('reply')
}
const entries = ref<MenuEntryDto[]>([])
function load() {
  if (props.target)
    void contextMenu(props.target).then((items) => {
      entries.value = items
    })
}
async function select(entry: MenuEntryDto) {
  if (!props.target) return
  if (entry.kind === 'plugin') await pluginMenuAction(entry.plugin, entry.index, props.target)
  if (entry.kind === 'action' && entry.enabled) {
    const text = await runAction(entry.id, props.target)
    if (text !== null) await navigator.clipboard.writeText(text)
  }
}
</script>
<template>
  <ContextMenu v-if="target">
    <ContextMenuTrigger as-child :class="selectable ? 'select-text' : undefined">
      <div @contextmenu.stop="load"><slot /></div>
    </ContextMenuTrigger>
    <ContextMenuContent @close-auto-focus="closeAutoFocus">
      <template v-for="(entry, index) in entries" :key="index">
        <ContextMenuSeparator v-if="entry.kind === 'separator'" />
        <ContextMenuItem
          v-else
          :disabled="entry.kind === 'action' && !entry.enabled"
          @select="select(entry)"
        >
          {{ entry.label }}
        </ContextMenuItem>
      </template>
      <ContextMenuItem
        v-if="target.type === 'message' && replyEnabled !== undefined"
        :disabled="!replyEnabled"
        @select="reply"
      >
        Reply
      </ContextMenuItem>
      <ContextMenuItem
        v-if="target.type === 'message' && threadAction"
        :disabled="threadAction.disabled"
        @select="emit('thread')"
      >
        {{ threadAction.label }}
      </ContextMenuItem>
      <ContextMenuItem
        v-if="target.type === 'message' && postAction"
        :disabled="postAction.disabled || postAction.posted"
        @select="emit('post')"
      >
        {{ postAction.posted ? 'Posted to channel' : 'Post to channel' }}
      </ContextMenuItem>
      <ContextMenuItem
        v-if="target.type === 'message' && removeAction"
        variant="destructive"
        @select="emit('remove')"
      >
        {{ removeAction.label }}
      </ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>
  <slot v-else />
</template>
