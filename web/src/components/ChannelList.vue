<script setup lang="ts">
import ContextMenuHost from './ContextMenuHost.vue'
import type { BranchRowDto, ContextTargetDto } from '../bridge/types'
const props = defineProps<{ branches: BranchRowDto[]; project?: string; channel?: string; unread?: boolean; pendingApprovals?: number; target?: ContextTargetDto }>()
const emit = defineEmits<{ toggleBranch: [key: string] }>()
function hasChildren(index: number): boolean {
  const row = props.branches[index]!
  return row.kind !== 'branch' && (row.collapsed || (props.branches[index + 1]?.depth ?? 0) > row.depth)
}
</script>
<template>
  <nav aria-label="Branches" class="h-full overflow-auto bg-sidebar text-sidebar-foreground">
    <h1 class="border-b border-border px-4 py-3 font-bold">{{ project }}</h1>
    <ContextMenuHost :target="target ?? null"><div v-if="channel" class="m-2 rounded-md bg-sidebar-accent px-2 py-1" :class="{ 'font-bold': unread }"># {{ channel }} <span v-if="pendingApprovals">{{ pendingApprovals }}</span></div></ContextMenuHost>
    <h2 v-if="branches.length" class="px-4 py-2 text-xs text-muted-foreground">BRANCHES</h2>
    <button v-for="(branch, index) in branches" :key="branch.key" type="button" :data-depth="branch.depth"
      :aria-expanded="hasChildren(index) ? !branch.collapsed : undefined"
      class="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left hover:bg-sidebar-accent"
      :class="{ 'bg-sidebar-accent': branch.current }" @click="hasChildren(index) && emit('toggleBranch', branch.key)">
      <span v-for="n in branch.depth" :key="n" class="w-4 shrink-0" />
      <span v-if="hasChildren(index)">{{ branch.collapsed ? '▸' : '▾' }}</span>
      <span v-if="branch.kind !== 'branch'">●</span><span class="flex-1 truncate">{{ branch.label }}</span><span v-if="branch.kind === 'main'" class="text-xs">main</span>
    </button>
    <slot />
  </nav>
</template>
