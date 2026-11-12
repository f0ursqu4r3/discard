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
import { activeTheme } from '@/bridge/theme'
import { resolveColor } from './agents/agent-icon'
import type { ProjectMenuEntry, MenuAction } from './project-menu'
defineProps<{ entries: ProjectMenuEntry[] }>()
const emit = defineEmits<{ action: [action: MenuAction, arg?: string] }>()
const swatchColor = (slot: string) => resolveColor(slot, activeTheme.value?.tokens ?? {})
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
          <ProjectMenuItems
            :entries="entry.children"
            @action="(action, arg) => emit('action', action, arg)"
          />
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
      @select="!entry.disabled && emit('action', entry.action, entry.arg)"
    >
      <span
        v-if="entry.swatch"
        data-testid="menu-swatch"
        class="size-3 shrink-0 rounded-full border border-border"
        :style="{ backgroundColor: swatchColor(entry.swatch) }"
        aria-hidden="true"
      />
      {{ entry.label }}
    </ContextMenuItem>
  </template>
</template>
