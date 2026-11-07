<script setup lang="ts">
import { CornerDownRight, Monitor, ExternalLink } from '@lucide/vue';
import type { BranchRowDto } from '../bridge/types';
import { branchActions, worktreeName } from './branch-navigation';
import NavigationContextMenu from './NavigationContextMenu.vue';
import { Button } from './ui/button';
import { Badge } from './ui/badge';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from './ui/tooltip';

defineProps<{
  row: BranchRowDto;
  busy?: boolean;
  selected?: boolean;
  unread?: boolean;
}>();
const emit = defineEmits<{ open: [key: string] }>();
</script>
<template>
  <div class="flex min-w-0 items-center gap-1 pl-3" data-ui-id="worktree-child">
    <CornerDownRight
      v-if="!row.detached"
      class="size-3.5 shrink-0 self-start mt-2 text-muted-foreground/60"
      aria-hidden="true"
    />
    <NavigationContextMenu
      kind="worktree"
      :name="worktreeName(row)"
      :id="row.key"
      :path="row.worktree"
      :busy="busy"
      @open="emit('open', row.key)"
    >
      <Button
        variant="ghost"
        :data-ui-id="`worktrees-row-${row.key}`"
        :aria-current="selected ? 'page' : undefined"
        :disabled="busy"
        :title="`${row.worktree || 'Worktree path unavailable'}${row.detached ? ` — detached HEAD ${row.label}` : ''}`"
        class="flex h-8 min-w-0 flex-1 justify-start items-center gap-2 rounded-md px-1 text-left text-xs outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          selected
            ? 'bg-sidebar-accent text-sidebar-accent-foreground'
            : 'text-muted-foreground'
        "
        @click="emit('open', row.key)"
      >
        <Monitor class="size-3.5 shrink-0 text-primary" aria-hidden="true" />
        <span class="min-w-0 truncate">{{ worktreeName(row) }}</span>
        <Badge
          v-if="row.kind === 'main'"
          variant="outline"
          class="shrink-0 rounded-sm px-1.5 text-[10px]"
        >
          Main
        </Badge>
        <span
          v-if="row.detached"
          class="max-w-24 shrink-0 truncate text-[10px]"
        >
          Detached {{ row.label }}
        </span>
        <span
          v-if="unread"
          data-ui-id="channel-unread-pip"
          role="img"
          aria-label="Unread messages"
          class="ml-auto size-2 shrink-0 rounded-full bg-sidebar-foreground"
        />
      </Button>
    </NavigationContextMenu>
    <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
      <Tooltip>
        <TooltipTrigger as-child>
          <span
            class="inline-flex shrink-0"
            tabindex="0"
            aria-disabled="true"
            :aria-label="`${branchActions.reveal.label} — Soon`"
          >
            <Button
              disabled
              variant="ghost"
              size="icon-xs"
              :aria-label="`${branchActions.reveal.label} — Soon`"
            >
              <ExternalLink aria-hidden="true" />
            </Button>
          </span>
        </TooltipTrigger>
        <TooltipContent>{{ branchActions.reveal.label }} · Soon</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  </div>
</template>
