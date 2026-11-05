<script setup lang="ts">
import { ref } from 'vue'
import type { ContextTargetDto, MenuEntryDto } from '../bridge/types'
import { contextMenu, pluginMenuAction, runAction } from '../bridge/api'
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from './ui/context-menu'
const props = defineProps<{ target: ContextTargetDto | null }>()
const entries = ref<MenuEntryDto[]>([])
function load() { if (props.target) void contextMenu(props.target).then(items => { entries.value = items }) }
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
    <ContextMenuTrigger as-child><div @contextmenu.stop="load"><slot /></div></ContextMenuTrigger>
    <ContextMenuContent><template v-for="(entry, index) in entries" :key="index">
      <ContextMenuSeparator v-if="entry.kind === 'separator'" />
      <ContextMenuItem v-else :disabled="entry.kind === 'action' && !entry.enabled" @select="select(entry)">{{ entry.label }}</ContextMenuItem>
    </template></ContextMenuContent>
  </ContextMenu>
  <slot v-else />
</template>
