<script setup lang="ts">
import { shortPath } from '@/settings/paths'
import { Monitor, ExternalLink } from '@lucide/vue'
import BrailleSpinner from './BrailleSpinner.vue'
import type { BranchRowDto } from '../bridge/types'
import { branchActions, worktreeName, treePrefix } from './branch-navigation'
import NavigationContextMenu from './NavigationContextMenu.vue'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'

defineProps<{
  isLast?: boolean
  workingAgentId?: string
  row: BranchRowDto
  busy?: boolean
  selected?: boolean
  unread?: boolean
  /** Create Branch… is blocked: a channel that uses the working tree is busy. */
  createBranchDisabled?: boolean
  /** Remove Worktree… is blocked: the main checkout, or a channel that uses the working tree is busy. */
  removeWorktreeDisabled?: boolean
}>()
const emit = defineEmits<{ open: [key: string]; createBranch: []; removeWorktree: [] }>()
</script>
<template>
  <div class="flex min-w-0 items-center gap-1 pl-3" data-ui-id="worktree-child">
    <span
      v-if="!row.detached"
      aria-hidden="true"
      data-testid="tree-prefix"
      class="w-[1ch] shrink-0 font-mono text-muted-foreground/70"
    >
      {{ treePrefix(!!isLast) }}
    </span>
    <NavigationContextMenu
      kind="worktree"
      :name="worktreeName(row)"
      :id="row.key"
      :path="row.worktree"
      :busy="busy"
      :create-branch-disabled="createBranchDisabled"
      :remove-worktree-disabled="removeWorktreeDisabled"
      @create-branch="emit('createBranch')"
      @remove-worktree="emit('removeWorktree')"
      @open="emit('open', row.key)"
    >
      <Button
        variant="ghost"
        :data-ui-id="`worktrees-row-${row.key}`"
        :aria-current="selected ? 'page' : undefined"
        :disabled="busy"
        :title="`${row.worktree ? shortPath(row.worktree) : 'Worktree path unavailable'}${row.detached ? ` — detached HEAD ${row.label}` : ''}`"
        class="flex h-8 min-w-0 flex-1 justify-start items-center gap-2 rounded-md px-1 text-left text-xs outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          selected
            ? 'nav-active-edge bg-sidebar-accent text-sidebar-accent-foreground'
            : 'text-muted-foreground'
        "
        @click="emit('open', row.key)"
      >
        <Monitor class="size-3.5 shrink-0 text-primary" aria-hidden="true" />
        <span class="min-w-0 truncate font-mono text-[0.92em]">
          {{ worktreeName(row) }}
        </span>
        <Badge
          v-if="row.kind === 'main'"
          variant="outline"
          class="shrink-0 rounded-sm px-1.5 font-mono text-[10px]"
        >
          Main
        </Badge>
        <span v-if="row.detached" class="max-w-24 shrink-0 truncate font-mono text-[10px]">
          Detached {{ row.label }}
        </span>
        <BrailleSpinner v-if="workingAgentId" :agent-id="workingAgentId" class="ml-auto shrink-0" />
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
