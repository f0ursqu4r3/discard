<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getConversations, type ConversationEntry } from '@/bridge/api'
import { Bot, FileText, FolderGit2, GitBranch, MessagesSquare, Users } from '@lucide/vue'
import type { MessageDto, SnapshotDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  DialogClose,
} from '../ui/dialog'
import BrandMark from './BrandMark.vue'
import MemoryBrowser from '../memory/MemoryBrowser.vue'
import FriendsList from '../agents/FriendsList.vue'
import { Button } from '../ui/button'
import { Badge } from '../ui/badge'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '../ui/card'
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  EmptyDescription,
  EmptyContent,
} from '../ui/empty'
defineOptions({ name: 'WorkspacePageView' })
const props = defineProps<{
  page: WorkspacePage
  snapshot: SnapshotDto
  messages: MessageDto[]
}>()
const emit = defineEmits<{ action: [id: string]; conversation: [] }>()
const conversations = ref<ConversationEntry[]>([])
const error = ref('')
const projectAgents = computed(() => [
  ...new Map(conversations.value.map((c) => [c.agentId, c])).values(),
])
const currentProject = computed(
  () => props.snapshot.activeProjectId === props.snapshot.projectId && !!props.snapshot.channelId,
)
const stops: UnlistenFn[] = []
let disposed = false,
  generation = 0
async function reload() {
  const current = ++generation,
    id = props.snapshot.projectId
  try {
    const rows = await getConversations()
    if (disposed || current !== generation) return
    conversations.value = rows.filter((c) => c.projectId === id)
    error.value = ''
  } catch (cause) {
    if (!disposed && current === generation) error.value = String(cause)
  }
}
watch(() => props.snapshot.projectId, reload, { immediate: true })
onMounted(async () => {
  try {
    for (const event of ['discard://agents', 'discard://conversations']) {
      const unlisten = await listen(event, () => {
        void reload()
      })
      if (disposed) {
        unlisten()
        return
      }
      stops.push(unlisten)
    }
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  stops.forEach((stop) => stop())
})

const branchCount = computed(() =>
  currentProject.value ? props.snapshot.branches.filter((row) => row.kind === 'branch').length : 0,
)
</script>
<template>
  <FriendsList v-if="page === 'direct-messages'" @action="emit('action', $event)" />
  <div
    v-else
    data-ui-id="project-page"
    class="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 sm:p-6"
  >
    <div
      v-if="page === 'project-settings'"
      data-ui-id="project-settings"
      class="mx-auto flex max-w-3xl flex-col gap-5"
    >
      <div>
        <h2 class="text-xl font-semibold">{{ snapshot.project }}</h2>
        <p class="mt-1 text-sm text-muted-foreground">Project settings</p>
      </div>
      <Card>
        <CardHeader><CardTitle>Project</CardTitle></CardHeader>
        <CardContent>
          <dl class="flex flex-col gap-4 text-sm">
            <div>
              <dt class="text-muted-foreground">Name</dt>
              <dd class="mt-1">{{ snapshot.project }}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Folder</dt>
              <dd class="mt-1 break-all">{{ snapshot.projectPath }}</dd>
              <Button
                data-ui-id="change-project-folder"
                variant="outline"
                size="sm"
                class="mt-3"
                :disabled="snapshot.archiveBusy"
                @click="emit('action', 'change_project_folder')"
              >
                <FolderGit2 data-icon="inline-start" />
                Change folder
              </Button>
              <p class="mt-2 text-xs text-muted-foreground">
                Messages stay saved. Agent sessions reset for the new folder. Discard stays open.
              </p>
            </div>
          </dl>
        </CardContent>
      </Card>
      <Dialog>
        <DialogTrigger as-child>
          <Button
            data-ui-id="remove-project"
            variant="outline"
            class="self-start"
            :disabled="snapshot.archiveBusy"
          >
            Remove project
          </Button>
        </DialogTrigger>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove {{ snapshot.project }}?</DialogTitle>
            <DialogDescription>
              This removes the project from the rail. Files and message history stay saved. Import
              the folder again to restore it.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose as-child>
              <Button variant="ghost">Cancel</Button>
            </DialogClose>
            <DialogClose as-child>
              <Button variant="destructive" @click="emit('action', 'remove_project')">
                Remove project
              </Button>
            </DialogClose>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Card>
        <CardHeader>
          <CardTitle>Channel workspaces</CardTitle>
          <CardDescription>Each channel keeps its assigned agent and workspace.</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-col divide-y divide-border">
          <div
            v-for="channel in conversations"
            :key="channel.id"
            class="flex min-w-0 flex-wrap items-center justify-between gap-2 py-3 first:pt-0 last:pb-0"
          >
            <div class="min-w-0">
              <p class="truncate text-sm font-medium">#{{ channel.name }}</p>
              <p class="text-xs text-muted-foreground">
                {{ channel.agent }} · {{ channel.provider }}
              </p>
            </div>
            <Badge variant="secondary">
              {{ channel.workspaceMode === 'worktree' ? 'Worktree' : 'Project folder' }}
            </Badge>
          </div>
        </CardContent>
      </Card>
    </div>
    <div v-else-if="page === 'overview'" class="mx-auto flex max-w-4xl flex-col gap-6">
      <div class="flex items-center gap-4">
        <BrandMark class="size-12" />
        <div class="min-w-0">
          <h2 class="text-xl font-semibold">{{ snapshot.project }}</h2>
          <p class="truncate text-sm text-muted-foreground">
            {{ snapshot.projectPath }}
          </p>
        </div>
      </div>
      <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Card
          v-for="stat in [
            { label: 'Branches', value: branchCount, icon: GitBranch },
            {
              label: 'Conversations',
              value: conversations.length,
              icon: MessagesSquare,
            },
            { label: 'Agents', value: projectAgents.length, icon: Bot },
            {
              label: 'Current messages',
              value: currentProject ? messages.length : 0,
              icon: FileText,
            },
          ]"
          :key="stat.label"
          size="sm"
        >
          <CardHeader>
            <CardDescription>{{ stat.label }}</CardDescription>
          </CardHeader>
          <CardContent class="flex items-center gap-2 text-xl font-semibold">
            <component :is="stat.icon" class="size-4 text-primary" />
            {{ stat.value }}
          </CardContent>
        </Card>
      </div>
      <Card size="sm">
        <CardHeader><CardTitle>Current work</CardTitle></CardHeader>
        <CardContent>
          <Button
            v-if="currentProject"
            variant="ghost"
            class="w-full justify-start"
            @click="emit('conversation')"
          >
            <FolderGit2 data-icon="inline-start" />
            <span class="truncate">{{ snapshot.channel }}</span>
            <Badge variant="secondary" class="ml-auto">
              {{ snapshot.turnRunning ? 'Running' : 'Idle' }}
            </Badge>
          </Button>
          <p v-else>Choose a conversation in this project from the navigation.</p>
        </CardContent>
      </Card>
    </div>
    <div v-else-if="page === 'agents'" class="mx-auto flex max-w-3xl flex-col gap-5">
      <div class="flex items-center justify-between gap-4">
        <h2 class="text-xl font-semibold">Project agents</h2>
        <Button variant="outline" size="sm" @click="emit('action', 'show_friends')">
          <Users data-icon="inline-start" />
          View agents
        </Button>
      </div>
      <Card v-for="agent in projectAgents" :key="agent.agentId" size="sm">
        <CardHeader>
          <CardTitle>
            <Button
              v-if="!agent.deleted"
              variant="link"
              class="h-auto p-0"
              @click="emit('action', `view_agent_profile:${agent.agentId}`)"
            >
              {{ agent.agent }}
            </Button>
            <span v-else>{{ agent.agent }}</span>
          </CardTitle>
          <CardDescription>
            {{ agent.provider }} · {{ agent.model }}
            <span v-if="agent.deleted">· Deleted</span>
            <span v-else-if="agent.agentArchived">· Archived</span>
          </CardDescription>
        </CardHeader>
      </Card>
      <p v-if="!projectAgents.length">No agents have conversations in this project.</p>
      <p v-if="error" role="alert">{{ error }}</p>
    </div>
    <div v-else-if="page === 'memory'" class="overflow-auto p-3">
      <MemoryBrowser
        v-if="snapshot.channelId && snapshot.activeProjectId === snapshot.projectId"
        :channel-id="snapshot.channelId"
        :agent-id="snapshot.agentId"
        :project-id="snapshot.activeProjectId"
        :revision="snapshot.memoryRev"
        :running="snapshot.turnRunning"
      />
      <p v-else>
        Choose a conversation in this project to see its memory. Settings → Memory shows all
        domains.
      </p>
    </div>
    <Empty v-else>
      <EmptyHeader>
        <EmptyMedia variant="icon"><FileText /></EmptyMedia>
        <EmptyTitle>No saved artifacts</EmptyTitle>
        <EmptyDescription>Code and file changes appear in the conversation.</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button variant="outline" @click="emit('conversation')">Open conversation</Button>
      </EmptyContent>
    </Empty>
  </div>
</template>
