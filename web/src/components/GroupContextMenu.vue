<script setup lang="ts">
import { computed } from 'vue'
import { ContextMenu, ContextMenuTrigger, ContextMenuContent } from './ui/context-menu'
import ProjectMenuItems from './ProjectMenuItems.vue'
import { groupMenu, type GroupMenuAction, type MenuAction } from './project-menu'
import { openContextMenuFromKeyboard } from './context-menu-keyboard'
const props = defineProps<{ collapsed: boolean }>()
const emit = defineEmits<{ action: [action: GroupMenuAction, arg?: string] }>()
const entries = computed(() => groupMenu(props.collapsed))
</script>
<template>
  <ContextMenu>
    <ContextMenuTrigger as-child @keydown="openContextMenuFromKeyboard">
      <slot />
    </ContextMenuTrigger>
    <ContextMenuContent class="min-w-56" :collision-padding="8">
      <ProjectMenuItems
        :entries="entries"
        @action="
          (action: MenuAction, arg?: string) => emit('action', action as GroupMenuAction, arg)
        "
      />
    </ContextMenuContent>
  </ContextMenu>
</template>
