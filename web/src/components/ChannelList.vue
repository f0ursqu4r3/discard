<script setup lang="ts">
import { ChevronDown, ChevronRight, Monitor, Users, Cpu, FileText, Settings, Hash, GitBranch, FolderGit2, Cable, Boxes, ShieldCheck, Plus, Bot } from '@lucide/vue'
import ContextMenuHost from './ContextMenuHost.vue'
import BrandMark from './workspace/BrandMark.vue'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { cn } from '@/lib/utils'
import type { BranchRowDto, ContextTargetDto } from '../bridge/types'
import type { WorkspacePage } from './workspace/types'
const props = withDefaults(defineProps<{ branches: BranchRowDto[]; project?: string; projectPath?: string; channel?: string; agent?: string; unread?: boolean; pendingApprovals?: number; target?: ContextTargetDto; page?: WorkspacePage }>(), { page: 'conversation' })
const emit = defineEmits<{ toggleBranch: [key: string]; navigate: [page: WorkspacePage]; action: [id: string] }>()
const navigation = [{ id: 'overview', label: 'Overview', icon: Monitor }, { id: 'agents', label: 'Agents', icon: Users }, { id: 'memory', label: 'Memory', icon: Cpu }, { id: 'artifacts', label: 'Artifacts', icon: FileText }] as const
function hasChildren(index: number): boolean {
  const row = props.branches[index]!
  return row.kind !== 'branch' && (row.collapsed || (props.branches[index + 1]?.depth ?? 0) > row.depth)
}
</script>
<template>
  <nav data-ui-id="channel-panel" aria-label="Workspace navigation" class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground">
    <header data-ui-id="project-header" class="flex h-16 shrink-0 items-center gap-3 border-b border-border/60 px-4">
      <BrandMark v-if="page !== 'direct-messages'" class="size-8" />
      <div class="min-w-0 flex-1"><h1 class="truncate text-sm font-semibold">{{ page === 'direct-messages' ? 'Direct messages' : project }}</h1><p class="truncate text-xs text-muted-foreground" :title="projectPath">{{ page === 'direct-messages' ? 'Your agents' : projectPath }}</p></div>
      <ChevronDown class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    </header>
    <div class="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto overscroll-contain px-2.5 py-3">
      <template v-if="page === 'direct-messages'">
        <Button variant="outline" size="sm" class="justify-start" @click="emit('action', 'open_agent_settings')"><Plus data-icon="inline-start" />Manage agents</Button>
        <div class="flex flex-col gap-1"><h2 class="px-2 pb-2 text-xs font-semibold text-muted-foreground">Agents</h2><Button variant="secondary" size="sm" class="justify-start" @click="emit('navigate', 'direct-messages')"><Bot data-icon="inline-start" /><span class="truncate">{{ agent }}</span></Button></div>
      </template>
      <template v-else>
        <div class="flex flex-col gap-1">
          <Button v-for="item in navigation" :key="item.id" :data-ui-id="`nav-${item.id}`" size="sm" :variant="page === item.id ? 'secondary' : 'ghost'" class="justify-start" @click="emit('navigate', item.id)"><component :is="item.icon" data-icon="inline-start" />{{ item.label }}</Button>
          <Button variant="ghost" size="sm" class="justify-start" @click="emit('action', 'open_workspace_settings')"><Settings data-icon="inline-start" />Settings</Button>
        </div>
        <section data-ui-id="channel-list" class="flex flex-col gap-1">
          <h2 class="flex items-center gap-1 px-2 pb-2 text-xs font-semibold text-muted-foreground"><ChevronDown class="size-3" />Channels</h2>
          <ContextMenuHost :target="target ?? null"><Button v-if="channel" :variant="page === 'conversation' ? 'secondary' : 'ghost'" size="sm" :class="cn('w-full justify-start', unread && 'font-semibold')" @click="emit('navigate', 'conversation')"><Hash data-icon="inline-start" /><span class="truncate">{{ channel }}</span><Badge v-if="pendingApprovals" variant="secondary" class="ml-auto">{{ pendingApprovals }}</Badge></Button></ContextMenuHost>
        </section>
        <section v-if="branches.length" data-ui-id="branch-tree" class="flex flex-col gap-1">
          <h2 class="flex items-center gap-1 px-2 pb-2 text-xs font-semibold text-muted-foreground"><ChevronDown class="size-3" />Branches &amp; worktrees</h2>
          <button v-for="(branch, index) in branches" :key="branch.key" type="button" :data-depth="branch.depth" :data-ui-id="`branch-${branch.key}`"
            :aria-label="hasChildren(index) ? `${branch.collapsed ? 'Expand' : 'Collapse'} ${branch.label}` : branch.label" :aria-expanded="hasChildren(index) ? !branch.collapsed : undefined"
            :class="cn('flex min-h-8 w-full items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring', branch.current && 'bg-sidebar-accent font-medium')"
            @click="hasChildren(index) && emit('toggleBranch', branch.key)">
            <span v-for="n in branch.depth" :key="n" class="w-2 shrink-0" />
            <component :is="branch.collapsed ? ChevronRight : ChevronDown" v-if="hasChildren(index)" class="size-3 shrink-0 text-muted-foreground" />
            <component :is="branch.kind === 'branch' ? GitBranch : FolderGit2" class="size-3.5 shrink-0 text-muted-foreground" />
            <span class="flex-1 truncate" :title="branch.label">{{ branch.label }}</span><span v-if="branch.current" class="size-1.5 shrink-0 rounded-full bg-success" aria-label="Current" />
          </button>
        </section>
        <section data-ui-id="integrations-navigation" class="flex flex-col gap-1">
          <h2 class="flex items-center gap-1 px-2 pb-2 text-xs font-semibold text-muted-foreground"><ChevronDown class="size-3" />Integrations</h2>
          <Button variant="ghost" size="sm" class="justify-start" @click="emit('action', 'open_settings')"><Cable data-icon="inline-start" />CLI connections</Button>
          <Button variant="ghost" size="sm" class="justify-start" @click="emit('action', 'review_plugins')"><Boxes data-icon="inline-start" />Lua plugins</Button>
          <Button variant="ghost" size="sm" class="justify-start" @click="emit('navigate', 'conversation')"><ShieldCheck data-icon="inline-start" />Approvals<Badge v-if="pendingApprovals" variant="secondary" class="ml-auto">{{ pendingApprovals }}</Badge></Button>
        </section>
      </template>
      <slot />
    </div>
  </nav>
</template>
