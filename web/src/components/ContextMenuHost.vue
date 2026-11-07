<script setup lang="ts">
import { ref } from 'vue'
import type { ContextTargetDto, MenuEntryDto } from '../bridge/types'
import { contextMenu, pluginMenuAction, runAction } from '../bridge/api'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './ui/context-menu'
const props = defineProps<{ target: ContextTargetDto | null; quoteEnabled?: boolean }>()
const emit = defineEmits<{ quote: [] }>()
const quoted = ref(false)
function quote() {
  if (!props.quoteEnabled) return
  quoted.value = true
}
function closeAutoFocus(event: Event) {
  if (!quoted.value) return
  event.preventDefault()
  quoted.value = false
  // Wait for the menu's focus trap to close before focusing the composer.
  if (props.quoteEnabled) emit('quote')
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
    <ContextMenuTrigger as-child>
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
        v-if="target.type === 'message' && quoteEnabled !== undefined"
        :disabled="!quoteEnabled"
        @select="quote"
      >
        Quote
      </ContextMenuItem>
      <!-- TODO: Open a clarification thread with channel history through this source message. -->
      <ContextMenuItem v-if="target.type === 'message'" disabled>
        Start thread
        <span class="ml-auto text-xs text-muted-foreground">Soon</span>
      </ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>
  <slot v-else />
</template>
