<script setup lang="ts">
import { ContextMenuPortal } from 'reka-ui'
import {
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubTrigger,
  ContextMenuSubContent,
  ContextMenuCheckboxItem,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
} from './ui/context-menu'
import type { ProjectMenuEntry, ProjectMenuAction } from './project-menu'
defineProps<{ entries: ProjectMenuEntry[] }>()
const emit = defineEmits<{ action: [action: ProjectMenuAction] }>()
</script>
<template>
  <template v-for="(entry, index) in entries" :key="index">
    <ContextMenuSeparator v-if="entry.kind === 'separator'" />
    <ContextMenuSub v-else-if="entry.kind === 'submenu'">
      <ContextMenuSubTrigger>{{ entry.label }}</ContextMenuSubTrigger>
      <ContextMenuPortal>
        <ContextMenuSubContent
          class="min-w-56 max-h-(--reka-context-menu-content-available-height) overflow-y-auto"
          :collision-padding="8"
        >
          <ProjectMenuItems :entries="entry.children" @action="emit('action', $event)" />
        </ContextMenuSubContent>
      </ContextMenuPortal>
    </ContextMenuSub>
    <ContextMenuRadioGroup v-else-if="entry.kind === 'planned-radio'">
      <ContextMenuRadioItem v-for="label in entry.labels" :key="label" :value="label" disabled>
        {{ label }}
        <span class="ml-auto text-xs text-muted-foreground">Soon</span>
      </ContextMenuRadioItem>
    </ContextMenuRadioGroup>
    <ContextMenuCheckboxItem
      v-else-if="entry.kind === 'planned' && entry.control === 'checkbox'"
      disabled
    >
      {{ entry.label }}
      <span class="ml-auto text-xs text-muted-foreground">Soon</span>
    </ContextMenuCheckboxItem>
    <ContextMenuItem v-else-if="entry.kind === 'planned'" disabled>
      {{ entry.label }}
      <span class="ml-auto text-xs text-muted-foreground">Soon</span>
    </ContextMenuItem>
    <ContextMenuItem
      v-else
      :disabled="entry.disabled"
      :variant="entry.destructive ? 'destructive' : 'default'"
      @select="!entry.disabled && emit('action', entry.action)"
    >
      {{ entry.label }}
    </ContextMenuItem>
  </template>
</template>
