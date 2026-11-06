<script setup lang="ts">
import { computed, reactive, watch } from 'vue';
import {
  ChevronDown,
  ChevronRight,
  Monitor,
  Users,
  Cpu,
  FileText,
  Settings,
  Hash,
  Cable,
  Boxes,
  ShieldCheck,
  Plus,
} from '@lucide/vue';
import BrandMark from './workspace/BrandMark.vue';
import { Button } from './ui/button';
import { Badge } from './ui/badge';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from './ui/collapsible';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';
import { cn } from '@/lib/utils';
import type {
  BranchRowDto,
  ChannelDto,
  ContextTargetDto,
  ProjectChannelDto,
} from '../bridge/types';
import type { WorkspacePage } from './workspace/types';
const props = withDefaults(
  defineProps<{
    projectId?: string;
    activeProjectId?: string | null;
    branches: BranchRowDto[];
    channels?: ChannelDto[];
    projectChannels?: ProjectChannelDto[];
    channelId?: string;
    project?: string;
    projectPath?: string;
    projectDisplayPath?: string;
    channel?: string;
    agent?: string;
    unread?: boolean;
    pendingApprovals?: number;
    target?: ContextTargetDto;
    page?: WorkspacePage;
    busy?: boolean;
  }>(),
  { page: 'conversation', channels: () => [], projectChannels: () => [] }
);
const emit = defineEmits<{
  toggleBranch: [key: string];
  navigate: [page: WorkspacePage];
  action: [id: string];
  selectChannel: [id: string];
  inspectChannel: [id: string];
  openBranch: [key: string];
  approvals: [];
}>();
const generalChannel = computed(() =>
  props.projectChannels.find(
    (channel) => channel.kind === 'general' && !channel.archived
  )
);
const navigation = [
  { id: 'overview', label: 'Overview', icon: Monitor },
  { id: 'agents', label: 'Agents', icon: Users },
  { id: 'memory', label: 'Memory', icon: Cpu },
  { id: 'artifacts', label: 'Artifacts', icon: FileText },
] as const;
const sections = reactive({
  branches: true,
  worktrees: true,
  integrations: true,
  archived: false,
});
const gitBranches = computed(() =>
  props.branches.filter((row) => !row.detached)
);
const worktrees = computed(() =>
  props.branches.filter((row) => row.kind !== 'branch')
);
const savedChannels = computed(() =>
  props.projectChannels.filter((channel) => {
    if (channel.kind === 'general') return false;
    if (
      !channel.archived &&
      channel.kind === 'branch' &&
      channel.branch &&
      gitBranches.value.some((row) => row.key === channel.branch)
    )
      return false;
    return true;
  })
);
watch(
  () => props.projectPath,
  (path) => {
    try {
      Object.assign(
        sections,
        {
          branches: true,
          worktrees: true,
          integrations: true,
          archived: false,
        },
        JSON.parse(localStorage.getItem(`discard:sections:${path}`) ?? '{}')
      );
    } catch {
      /* Use the default section state. */
    }
  },
  { immediate: true }
);
watch(sections, (value) => {
  try {
    localStorage.setItem(
      `discard:sections:${props.projectPath}`,
      JSON.stringify(value)
    );
  } catch {
    /* Navigation remains usable when storage is unavailable. */
  }
});
const approvals = computed(
  () =>
    props.channels.reduce(
      (count, channel) => count + channel.pendingApprovals,
      0
    ) ||
    props.pendingApprovals ||
    0
);
</script>
<template>
  <nav
    data-ui-id="channel-panel"
    aria-label="Workspace navigation"
    class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground"
  >
    <DropdownMenu v-if="projectId">
      <DropdownMenuTrigger as-child>
        <button
          data-ui-id="project-header"
          type="button"
          aria-label="Project menu"
          class="flex h-16 w-full shrink-0 items-center gap-3 border-b border-border/60 px-4 text-left outline-none transition-colors hover:bg-sidebar-accent/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        >
          <BrandMark class="size-8 shrink-0" />
          <span class="min-w-0 flex-1">
            <span class="block truncate text-sm font-semibold">
              {{ project }}
            </span>
            <span
              class="block truncate text-xs text-muted-foreground"
              :title="projectPath"
            >
              {{ projectDisplayPath ?? projectPath }}
            </span>
          </span>
          <ChevronDown
            class="size-4 shrink-0 text-muted-foreground"
            aria-hidden="true"
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent data-ui-id="project-menu" align="start" class="w-60">
        <DropdownMenuItem
          :disabled="!projectId"
          @select="emit('navigate', 'overview')"
        >
          <Monitor />
          Project overview
        </DropdownMenuItem>
        <!-- TODO: Create project channels from a branch instead of creating an unbound conversation. -->
        <DropdownMenuItem disabled>
          <Plus />
          New conversation
          <span class="ml-auto text-xs text-muted-foreground">Soon</span>
        </DropdownMenuItem>
        <DropdownMenuItem @select="emit('action', 'show_friends')">
          <Users />
          Agents
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          :disabled="!projectId"
          @select="emit('navigate', 'project-settings')"
        >
          <Settings />
          Project settings
        </DropdownMenuItem>
        <DropdownMenuItem @select="emit('action', 'open_provider_settings')">
          <Cable />
          CLI connections
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <div
      data-ui-id="channel-panel-scroll"
      class="flex min-h-0 flex-1 flex-col overflow-x-hidden overflow-y-auto overscroll-contain p-2.5"
    >
      <template v-if="projectId">
        <div data-ui-id="project-navigation" class="flex flex-col gap-0.5">
          <Button
            v-for="item in navigation"
            :key="item.id"
            :data-ui-id="`nav-${item.id}`"
            :aria-current="page === item.id ? 'page' : undefined"
            size="sm"
            :variant="page === item.id ? 'secondary' : 'ghost'"
            class="h-8 justify-start gap-2 px-2 font-normal"
            @click="emit('navigate', item.id)"
          >
            <component :is="item.icon" data-icon="inline-start" />
            {{ item.label }}
          </Button>
          <Button
            data-ui-id="nav-settings"
            :variant="page === 'project-settings' ? 'secondary' : 'ghost'"
            :aria-current="page === 'project-settings' ? 'page' : undefined"
            size="sm"
            class="h-8 justify-start gap-2 px-2 font-normal"
            @click="emit('navigate', 'project-settings')"
          >
            <Settings data-icon="inline-start" />
            Settings
          </Button>
        </div>

        <button
          v-if="projectId"
          type="button"
          data-ui-id="project-general-channel"
          :aria-current="
            generalChannel?.id === channelId && page === 'conversation'
              ? 'page'
              : undefined
          "
          :disabled="busy || !generalChannel"
          :class="
            cn(
              'mt-3 flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring',
              generalChannel?.id === channelId && page === 'conversation'
                ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                : 'text-muted-foreground'
            )
          "
          @click="generalChannel && emit('selectChannel', generalChannel.id)"
        >
          <Hash class="size-4 shrink-0" />
          <span class="truncate">general</span>
        </button>

        <Collapsible
          v-for="group in [
            {
              id: 'branches',
              label: 'Branches',
              rows: gitBranches,
              icon: Hash,
            },
            {
              id: 'worktrees',
              label: 'Worktrees',
              rows: worktrees,
              icon: Monitor,
            },
          ] as const"
          :key="group.id"
          v-model:open="sections[group.id]"
          :data-ui-id="`${group.id}-list`"
          class="mt-5"
        >
          <CollapsibleTrigger as-child>
            <button
              :data-ui-id="`${group.id}-toggle`"
              class="flex h-7 w-full items-center gap-1 px-1 text-xs font-semibold text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component
                :is="sections[group.id] ? ChevronDown : ChevronRight"
                class="size-3"
              />
              {{ group.label }}
              <span class="ml-1 font-normal tabular-nums">{{
                group.rows.length
              }}</span>
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <button
              v-for="row in group.rows"
              :key="row.key"
              type="button"
              :data-ui-id="`${group.id}-row-${row.key}`"
              :aria-current="
                row.current && page === 'conversation' ? 'page' : undefined
              "
              :disabled="busy || !projectId"
              :title="row.label"
              :class="
                cn(
                  'flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring',
                  row.current && page === 'conversation'
                    ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                    : 'text-muted-foreground'
                )
              "
              @click="emit('openBranch', row.key)"
            >
              <component :is="group.icon" class="size-4 shrink-0" />
              <span class="min-w-0 flex-1 truncate">{{ row.label }}</span>
              <span
                v-if="group.id === 'worktrees'"
                class="size-1.5 shrink-0 rounded-full bg-success"
                aria-label="Worktree available"
              />
            </button>
            <p
              v-if="!group.rows.length"
              class="px-2 py-1 text-xs text-muted-foreground"
            >
              No {{ group.label.toLowerCase() }} found
            </p>
          </CollapsibleContent>
        </Collapsible>
        <Collapsible
          v-if="savedChannels.length"
          v-model:open="sections.archived"
          data-ui-id="saved-channel-navigation"
          class="mt-3"
        >
          <CollapsibleTrigger as-child>
            <button
              data-ui-id="saved-channel-toggle"
              class="flex h-7 w-full items-center gap-1 px-1 text-xs font-semibold text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component
                :is="sections.archived ? ChevronDown : ChevronRight"
                class="size-3"
              />
              Saved channels
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <button
              v-for="channel in savedChannels"
              :key="channel.id"
              type="button"
              :data-ui-id="`saved-channel-${channel.id}`"
              :disabled="busy"
              class="flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left text-xs text-muted-foreground outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring"
              @click="emit('selectChannel', channel.id)"
            >
              <Hash class="size-4 shrink-0" />
              <span class="min-w-0 flex-1 truncate">{{
                channel.branch ?? channel.name
              }}</span>
              <span class="shrink-0">
                {{
                  channel.archived
                    ? 'History'
                    : channel.kind === 'legacy'
                      ? 'Saved'
                      : 'Channel'
                }}
              </span>
            </button>
          </CollapsibleContent>
        </Collapsible>
        <Collapsible
          v-model:open="sections.integrations"
          data-ui-id="integrations-navigation"
          class="mt-5"
        >
          <CollapsibleTrigger as-child>
            <button
              data-ui-id="integrations-toggle"
              class="flex h-7 w-full items-center gap-1 px-1 text-xs font-semibold text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component
                :is="sections.integrations ? ChevronDown : ChevronRight"
                class="size-3"
              />
              Integrations
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <Button
              data-ui-id="nav-connections"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('action', 'open_provider_settings')"
            >
              <Cable data-icon="inline-start" />
              CLI connections
            </Button>
            <Button
              data-ui-id="nav-plugins"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('action', 'review_plugins')"
            >
              <Boxes data-icon="inline-start" />
              Lua plugins
            </Button>
            <Button
              data-ui-id="nav-approvals"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('approvals')"
            >
              <ShieldCheck data-icon="inline-start" />
              Approvals
              <Badge
                v-if="approvals"
                class="ml-auto h-4 min-w-4 rounded-full px-1 text-xs tabular-nums"
              >
                {{ approvals }}
              </Badge>
            </Button>
          </CollapsibleContent>
        </Collapsible>
      </template>
      <slot />
    </div>
  </nav>
</template>
