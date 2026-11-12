<script setup lang="ts">
import { shortPath } from '@/settings/paths'
defineOptions({ name: 'SettingsWorkspaces' })
import { computed, onMounted, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { ChevronRight, FolderOpen } from '@lucide/vue'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import PathText from '../ui/PathText.vue'
import SettingsCard from '../ui/SettingsCard.vue'
import SettingsNotice from '../ui/SettingsNotice.vue'
type Worktree = { path: string; branch: string; exists: boolean }
type Project = {
  name: string
  path: string
  exists: boolean
  channels: {
    name: string
    mode: string
    archived: boolean
    current: boolean
    worktree: Worktree | null
  }[]
}
const rows = ref<Project[]>([]),
  error = ref('')
async function reveal(path: string) {
  try {
    await invoke('settings_reveal_workspace', { path })
  } catch (e) {
    error.value = String(e)
  }
}
// The project with the current channel comes first. Archived channels go last. Both sorts are stable.
const projects = computed(() =>
  rows.value
    .map((project) => ({
      project,
      channels: [...project.channels].sort((a, b) => Number(a.archived) - Number(b.archived)),
    }))
    .sort(
      (a, b) =>
        Number(b.project.channels.some((c) => c.current)) -
        Number(a.project.channels.some((c) => c.current)),
    ),
)
const collapsed = reactive(new Set<string>())
function setOpen(path: string, open: boolean) {
  if (open) collapsed.delete(path)
  else collapsed.add(path)
}
const chip = 'rounded-sm border px-1.5 text-[10.5px] leading-4'
onMounted(async () => {
  try {
    rows.value = await invoke<Project[]>('settings_workspaces')
  } catch (e) {
    error.value = String(e)
  }
})
</script>
<template>
  <div class="space-y-4">
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <Collapsible
      v-for="{ project, channels } in projects"
      :key="project.path"
      :open="!collapsed.has(project.path)"
      @update:open="setOpen(project.path, $event)"
    >
      <SettingsCard flush :rule="!collapsed.has(project.path)">
        <template #title>
          <CollapsibleTrigger
            class="group/trigger flex max-w-full items-center gap-1.5 rounded-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            <ChevronRight
              aria-hidden="true"
              class="size-3.5 shrink-0 text-muted-foreground transition-transform group-data-[state=open]/trigger:rotate-90 motion-reduce:transition-none"
            />
            <span class="truncate font-sans text-sm font-medium">{{ project.name }}</span>
          </CollapsibleTrigger>
        </template>
        <template #actions>
          <span class="min-w-0 text-muted-foreground"><PathText :path="project.path" /></span>
          <span class="shrink-0 text-muted-foreground">
            {{ channels.length }} {{ channels.length === 1 ? 'channel' : 'channels' }}
          </span>
          <span v-if="!project.exists" :class="[chip, 'shrink-0 border-danger/50 text-danger']">
            missing
          </span>
          <Button
            size="icon-xs"
            variant="ghost"
            :aria-label="`Reveal ${project.name}`"
            :title="`Reveal ${project.name}`"
            @click="reveal(project.path)"
          >
            <FolderOpen />
          </Button>
        </template>
        <CollapsibleContent>
          <p v-if="!channels.length" class="px-3 py-2 font-mono text-xs text-muted-foreground">
            no channels
          </p>
          <ul v-else class="flex flex-col font-mono text-xs">
            <li
              v-for="channel in channels"
              :key="channel.name"
              class="grid h-8 grid-cols-[minmax(0,3fr)_minmax(0,2fr)_9rem_1.5rem] items-center gap-x-2 border-b border-dashed border-border px-3 last:border-b-0 hover:bg-muted/40"
            >
              <!-- prettier-ignore -->
              <span class="sr-only">{{ channel.name }} · {{ channel.mode }}{{ channel.archived ? ' · Archived' : ''
              }}{{ channel.current ? ' · Current' : '' }}{{ channel.worktree ? ` · ${channel.worktree.branch} · ${shortPath(channel.worktree.path)}` : ''
              }}{{ channel.worktree && !channel.worktree.exists ? ' · Missing' : '' }}</span>
              <span aria-hidden="true" class="flex min-w-0 items-baseline gap-2">
                <span class="truncate" :title="channel.name">{{ channel.name }}</span>
                <span
                  v-if="channel.worktree && channel.worktree.branch !== channel.name"
                  class="min-w-0 shrink-[3] truncate text-muted-foreground"
                  :title="channel.worktree.branch"
                >
                  {{ channel.worktree.branch }}
                </span>
              </span>
              <span aria-hidden="true" class="min-w-0 text-muted-foreground">
                <PathText v-if="channel.worktree" :path="channel.worktree.path" />
                <span v-else class="block truncate">{{ channel.mode.toLowerCase() }}</span>
              </span>
              <span aria-hidden="true" class="flex justify-end gap-1">
                <span v-if="channel.current" :class="[chip, 'border-primary/50 text-primary']">
                  current
                </span>
                <span
                  v-if="channel.archived"
                  :class="[chip, 'border-border text-muted-foreground']"
                >
                  archived
                </span>
                <span
                  v-if="channel.worktree && !channel.worktree.exists"
                  :class="[chip, 'border-danger/50 text-danger']"
                >
                  missing
                </span>
              </span>
              <Button
                v-if="channel.worktree"
                size="icon-xs"
                variant="ghost"
                :aria-label="`Reveal ${channel.name}`"
                :title="`Reveal ${channel.name}`"
                @click="reveal(channel.worktree.path)"
              >
                <FolderOpen />
              </Button>
            </li>
          </ul>
        </CollapsibleContent>
      </SettingsCard>
    </Collapsible>
  </div>
</template>
