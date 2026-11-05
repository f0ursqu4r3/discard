<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { useMediaQuery } from '@vueuse/core'
import { ScanEye, LoaderCircle, GitPullRequest, ListChecks } from '@lucide/vue'
import { toast } from 'vue-sonner'
import { useStore } from './bridge/store'
import { useWebShortcuts } from './bridge/shortcuts'
import {
  setMemberListVisible,
  setPanels,
  toggleBranch,
  sendMessage,
  cancelTurn,
  runAction,
  switchChannel,
  openBranchChannel,
  chooseProjectFolder,
  getProjects,
  selectProject,
  removeProject,
  type ProjectEntry,
} from './bridge/api'
import ChannelList from './components/ChannelList.vue'
import ChannelHeader from './components/ChannelHeader.vue'
import MessageList from './components/MessageList.vue'
import Composer from './components/Composer.vue'
import Notices from './components/Notices.vue'
import ApprovalCard from './components/ApprovalCard.vue'
import DiffBlock from './components/DiffBlock.vue'
import ToolCallBlock from './components/ToolCallBlock.vue'
import { thinkingLine } from './components/messages'
import { navigationWidth, panelWidths, savePanelWidths } from './components/layout'
import PluginElement from './plugin/PluginElement.vue'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from './components/ui/resizable'
import { Tabs, TabsList, TabsTrigger } from './components/ui/tabs'
import { Sheet, SheetContent, SheetTitle, SheetDescription } from './components/ui/sheet'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from './components/ui/empty'
import { Button } from './components/ui/button'
import { Badge } from './components/ui/badge'
import { Toaster } from './components/ui/sonner'
import CreateChannelDialog from './components/workspace/CreateChannelDialog.vue'
import NavigationPanel from './components/workspace/NavigationPanel.vue'
import ContextPanel from './components/workspace/ContextPanel.vue'
import WorkspacePageView from './components/workspace/WorkspacePage.vue'
import BrandMark from './components/workspace/BrandMark.vue'
import UiInspector from './components/workspace/UiInspector.vue'
import type { WorkspacePage } from './components/workspace/types'
import type { TranscriptPosition } from './components/workspace/transcript'
const store = useStore()
const projects = ref<ProjectEntry[]>([])
async function refreshProjects() {
  projects.value = await getProjects()
}
watch(
  () => store.snapshot?.projectId,
  () => {
    void refreshProjects().catch((error) => toast.error(String(error)))
  },
  { immediate: true },
)
useWebShortcuts()
const drafts = reactive<Record<string, string>>({})
const transcriptPositions = reactive<Record<string, TranscriptPosition>>({})
const page = ref<WorkspacePage>('conversation')
const conversationTab = ref('conversation')
const newChannelOpen = ref(false),
  switching = ref(false),
  sending = ref(false)
const inspector = ref(false)
const isWide = useMediaQuery('(min-width: 1280px)')
const isCompact = useMediaQuery('(max-width: 639px)')
const navigationOpen = ref(false),
  contextOpen = ref(false)
const showContextPanel = computed(() => isWide.value && !!store.snapshot?.memberListVisible)
const railWidth = computed(
  () => 72 * (store.snapshot?.theme.spacingScale ?? 1) * (store.snapshot?.theme.fontScale ?? 1),
)
const currentLayout = ref<number[]>([])
const heading = computed(() =>
  page.value === 'conversation'
    ? store.snapshot?.channel
    : page.value === 'direct-messages'
      ? store.snapshot?.agent
      : {
          overview: 'Project overview',
          agents: 'Project agents',
          memory: 'Project memory',
          artifacts: 'Artifacts',
          'project-settings': 'Project settings',
        }[page.value],
)
const changes = computed(() =>
  store.history.flatMap((message) =>
    message.blocks.filter(
      (block) =>
        block.type === 'diff' ||
        (block.type === 'code' && ['diff', 'patch'].includes(block.language || '')),
    ),
  ),
)
const tools = computed(() =>
  [
    ...store.history.flatMap((message) => message.blocks),
    ...(store.snapshot?.liveBlocks ?? []),
  ].filter((block) => block.type === 'tool_call'),
)
function navigate(value: WorkspacePage) {
  page.value = value
  navigationOpen.value = false
  if (value === 'conversation') conversationTab.value = 'conversation'
}
async function action(id: string) {
  try {
    if (id === 'import_project' || id === 'change_project_folder') {
      await chooseProjectFolder(id === 'change_project_folder')
      await store.refresh()
      await refreshProjects()
      return
    }
    if (id.startsWith('select_project:')) {
      await selectProject(id.slice('select_project:'.length))
      await store.refresh()
      navigate('conversation')
      return
    }
    if (id === 'remove_project' && store.snapshot) {
      await removeProject(store.snapshot.projectId)
      await store.refresh()
      await refreshProjects()
      navigate('conversation')
      return
    }
    await runAction(id)
  } catch (error) {
    toast.error('Could not complete action', { description: String(error) })
  }
}
async function toggleMembers() {
  if (!isWide.value) {
    contextOpen.value = !contextOpen.value
    return
  }
  try {
    await setMemberListVisible(!store.snapshot!.memberListVisible)
  } catch {
    toast.error('Could not change panel visibility')
  }
}
async function savePanels() {
  await nextTick()
  const snapshot = store.snapshot
  if (
    !snapshot ||
    isCompact.value ||
    !savePanelWidths(showContextPanel.value, currentLayout.value.length)
  )
    return
  const [channel, member] = panelWidths(
    currentLayout.value,
    railWidth.value,
    snapshot.panels.memberList,
  )
  try {
    await setPanels(channel, member)
  } catch {
    toast.error('Could not save panel sizes')
  }
}
function resizeFinished(dragging: boolean) {
  if (!dragging) void savePanels()
}
async function send(text: string) {
  const id = store.snapshot?.channelId
  if (!id || switching.value || sending.value) return
  sending.value = true
  try {
    await sendMessage(id, text)
    if (drafts[id]?.trim() === text) drafts[id] = ''
  } catch (error) {
    toast.error('Message was not sent', { description: String(error) })
  } finally {
    sending.value = false
  }
}
async function stop() {
  if (store.snapshot)
    try {
      await cancelTurn(store.snapshot.channelId)
    } catch {
      toast.error('Could not stop the agent')
    }
}
async function selectChannel(id: string) {
  if (switching.value) return
  switching.value = true
  try {
    await switchChannel(id)
    await store.refresh()
    navigate('conversation')
  } catch (error) {
    toast.error('Could not open channel', { description: String(error) })
  } finally {
    switching.value = false
  }
}
async function openBranch(key: string) {
  if (switching.value) return
  switching.value = true
  try {
    await openBranchChannel(key)
    await store.refresh()
    navigate('conversation')
  } catch (error) {
    toast.error('Could not open branch', { description: String(error) })
  } finally {
    switching.value = false
  }
}
async function channelCreated() {
  await store.refresh()
  navigate('conversation')
}
async function showApprovals() {
  const channels = store.snapshot?.channels ?? []
  const channel =
    channels.find(
      (channel) => channel.id === store.snapshot?.channelId && channel.pendingApprovals,
    ) ?? channels.find((channel) => channel.pendingApprovals)
  if (!channel) {
    toast.info('No pending approvals')
    return
  }
  await selectChannel(channel.id)
  await nextTick()
  document
    .querySelector<HTMLElement>('[data-ui-id="approval-card"]')
    ?.scrollIntoView({ block: 'nearest' })
}
watch([isWide, isCompact], () => {
  navigationOpen.value = false
  contextOpen.value = false
  currentLayout.value = []
})
watch(
  () => store.snapshot?.channelId,
  () => {
    conversationTab.value = 'conversation'
  },
)
</script>
<template>
  <main
    data-testid="app-root"
    data-ui-id="app-shell"
    class="flex h-dvh flex-col overflow-hidden bg-canvas text-sm text-foreground"
  >
    <header data-ui-id="app-toolbar" class="flex h-9 shrink-0 items-center gap-2 px-3">
      <BrandMark class="size-4" /><span class="text-xs font-semibold">Discard</span>
      <span
        class="pointer-events-none mx-auto hidden truncate text-xs text-muted-foreground sm:block"
        >{{ store.snapshot?.project
        }}<template v-if="store.snapshot"> — agent workspace</template></span
      >
      <span
        v-if="store.snapshot?.turnRunning"
        class="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground"
        ><LoaderCircle class="size-3 motion-safe:animate-spin" />Working</span
      >
      <Button
        data-ui-id="ui-inspector-toggle"
        :variant="inspector ? 'secondary' : 'ghost'"
        size="icon-xs"
        class="ml-auto sm:ml-0"
        aria-label="Toggle UI names"
        :aria-pressed="inspector"
        @click="inspector = !inspector"
        ><ScanEye
      /></Button>
    </header>
    <div v-if="store.snapshot?.projectId" class="flex min-h-0 flex-1 gap-1.5 px-1.5 pb-1.5">
      <NavigationPanel
        :projects="projects"
        v-if="isCompact"
        class="shrink-0"
        compact
        :snapshot="store.snapshot"
        :page="page"
        @navigate="navigate"
        @action="action"
        @toggle-branch="toggleBranch"
        :busy="switching"
        @select-channel="selectChannel"
        @open-branch="openBranch"
        @create-channel="newChannelOpen = true"
        @approvals="showApprovals"
      />
      <ResizablePanelGroup
        direction="horizontal"
        :keyboard-resize-by="1"
        @layout="currentLayout = $event"
      >
        <template v-if="!isCompact">
          <ResizablePanel
            :order="1"
            :default-size="navigationWidth(store.snapshot.panels.channelList, railWidth)"
            size-unit="px"
            :min-size="180 + railWidth"
            :max-size="400 + railWidth"
          >
            <NavigationPanel
              :projects="projects"
              :snapshot="store.snapshot"
              :page="page"
              @navigate="navigate"
              @action="action"
              @toggle-branch="toggleBranch"
              :busy="switching"
              @select-channel="selectChannel"
              @open-branch="openBranch"
              @create-channel="newChannelOpen = true"
              @approvals="showApprovals"
            />
          </ResizablePanel>
          <ResizableHandle
            data-ui-id="navigation-resizer"
            aria-label="Resize navigation"
            with-handle
            @dragging="resizeFinished"
            @keyup="savePanels"
          />
        </template>
        <ResizablePanel :order="2" :min-size="isCompact ? 0 : 320" size-unit="px">
          <section
            data-ui-id="main-panel"
            class="flex h-full min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-workspace"
          >
            <ChannelHeader
              :channel="heading || ''"
              :subtitle="
                page === 'direct-messages' ? 'Direct messages' : store.snapshot.projectPath
              "
              :direct-message="page === 'direct-messages'"
              :compact="isCompact"
              :member-list-visible="showContextPanel || contextOpen"
              @toggle-members="toggleMembers"
              @toggle-navigation="navigationOpen = true"
            >
              <PluginElement
                v-if="page === 'conversation' && store.snapshot.slots.channel_header"
                :element="store.snapshot.slots.channel_header.element"
                :plugin="store.snapshot.slots.channel_header.plugin"
              />
            </ChannelHeader>
            <template v-if="page === 'conversation'">
              <Tabs
                v-model="conversationTab"
                data-ui-id="conversation-tabs"
                class="flex h-11 shrink-0 flex-row items-center justify-start overflow-x-auto border-b border-border/60 px-3 sm:px-5"
              >
                <TabsList variant="line"
                  ><TabsTrigger value="conversation">Conversation</TabsTrigger
                  ><TabsTrigger value="changes"
                    >Changes<Badge v-if="changes.length" variant="secondary">{{
                      changes.length
                    }}</Badge></TabsTrigger
                  ><TabsTrigger value="run">Run</TabsTrigger
                  ><TabsTrigger value="review">Merge review</TabsTrigger></TabsList
                >
              </Tabs>
              <MessageList
                :conversation-id="store.snapshot.channelId"
                :position="transcriptPositions[store.snapshot.channelId]"
                @save-position="(id, position) => (transcriptPositions[id] = position)"
                v-if="conversationTab === 'conversation'"
                :key="store.snapshot.channelId"
                :messages="store.history"
                :live="store.snapshot.liveBlocks"
                :running="store.snapshot.turnRunning"
                :new-anchor-id="store.snapshot.newAnchorId"
                :agent="store.snapshot.agent"
                :prompts="store.snapshot.prompts"
              />
              <div
                v-else-if="conversationTab === 'changes'"
                data-ui-id="changes-view"
                class="min-h-0 flex-1 overflow-auto overscroll-contain p-5"
              >
                <div v-if="changes.length" class="flex flex-col gap-4">
                  <DiffBlock
                    v-for="(block, index) in changes"
                    :key="index"
                    :text="'text' in block ? block.text : ''"
                  />
                </div>
                <Empty v-else
                  ><EmptyHeader
                    ><EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia
                    ><EmptyTitle>No changes to display</EmptyTitle
                    ><EmptyDescription
                      >Diffs shared in this conversation appear here.</EmptyDescription
                    ></EmptyHeader
                  ></Empty
                >
              </div>
              <div
                v-else-if="conversationTab === 'run'"
                data-ui-id="run-view"
                class="min-h-0 flex-1 overflow-auto overscroll-contain p-5"
              >
                <div v-if="tools.length" class="flex flex-col gap-4">
                  <h2 class="text-lg font-semibold">Tool activity</h2>
                  <ToolCallBlock
                    v-for="(tool, index) in tools"
                    :key="index"
                    :name="tool.name"
                    :input="tool.input"
                    :output="tool.output"
                  />
                </div>
                <Empty v-else
                  ><EmptyHeader
                    ><EmptyMedia variant="icon"><ListChecks /></EmptyMedia
                    ><EmptyTitle>No tool activity</EmptyTitle
                    ><EmptyDescription
                      >Tool calls from this conversation appear here.</EmptyDescription
                    ></EmptyHeader
                  ></Empty
                >
              </div>
              <div
                v-else
                data-ui-id="merge-review-view"
                class="min-h-0 flex-1 overflow-auto overscroll-contain"
              >
                <Empty
                  ><EmptyHeader
                    ><EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia
                    ><EmptyTitle>No merge review</EmptyTitle
                    ><EmptyDescription
                      >Merge reviews are not available in this workspace yet.</EmptyDescription
                    ></EmptyHeader
                  ></Empty
                >
              </div>
            </template>
            <WorkspacePageView
              v-else
              :page="page"
              :snapshot="store.snapshot"
              :messages="store.history"
              @action="action"
              @conversation="navigate('conversation')"
            />
            <div
              data-ui-id="protected-notices"
              class="max-h-64 shrink-0 overflow-auto overscroll-contain"
            >
              <template v-if="page !== 'conversation' || conversationTab !== 'conversation'"
                ><ApprovalCard
                  :channel-id="store.snapshot.channelId"
                  v-for="prompt in store.snapshot.prompts.filter((p) => p.view === 'pending')"
                  :key="prompt.approvalId"
                  :prompt="prompt"
              /></template>
              <Notices
                :banners="store.snapshot.banners"
                :notice="store.snapshot.notice"
                :archived="store.snapshot.archived"
              />
            </div>
            <template v-if="page === 'conversation'">
              <p
                v-if="thinkingLine(store.snapshot.turnRunning, [store.snapshot.agent])"
                class="flex items-center gap-2 px-5 py-1 text-xs text-muted-foreground"
              >
                <LoaderCircle class="size-3 motion-safe:animate-spin" />{{
                  thinkingLine(store.snapshot.turnRunning, [store.snapshot.agent])
                }}
              </p>
              <Composer
                :key="store.snapshot.channelId"
                v-model:draft="drafts[store.snapshot.channelId]"
                :channel="store.snapshot.channel"
                :agent="store.snapshot.agent"
                :archived="store.snapshot.archived"
                :busy="
                  switching ||
                  sending ||
                  store.snapshot.archiveBusy ||
                  !!store.snapshot.banners.archive
                "
                :running="store.snapshot.turnRunning"
                @send="send"
                @stop="stop"
              />
            </template>
          </section>
        </ResizablePanel>
        <template v-if="showContextPanel">
          <ResizableHandle
            data-ui-id="context-resizer"
            aria-label="Resize context"
            with-handle
            @dragging="resizeFinished"
            @keyup="savePanels"
          />
          <ResizablePanel
            :order="3"
            :default-size="Math.max(240, store.snapshot.panels.memberList)"
            size-unit="px"
            :min-size="240"
            :max-size="400"
            ><div class="h-full overflow-hidden rounded-xl border border-border/60">
              <ContextPanel
                :direct-message="page === 'direct-messages'"
                :snapshot="store.snapshot"
                @action="action"
                @approvals="navigate('conversation')"
              /></div
          ></ResizablePanel>
        </template>
      </ResizablePanelGroup>
      <CreateChannelDialog
        v-model:open="newChannelOpen"
        :project="store.snapshot.project"
        :channel-id="store.snapshot.channelId"
        :agent="store.snapshot.agent"
        :provider="store.snapshot.provider"
        :has-git="store.snapshot.branches.length > 0"
        @created="channelCreated"
      />
      <Sheet v-model:open="navigationOpen"
        ><SheetContent side="left" class="gap-0" :show-close-button="false"
          ><SheetTitle class="sr-only">Workspace navigation</SheetTitle
          ><SheetDescription class="sr-only">Projects, channels, and agents</SheetDescription
          ><Button variant="ghost" class="shrink-0" @click="navigationOpen = false"
            >Close navigation</Button
          ><ChannelList
            :channels="store.snapshot.channels"
            :channel-id="store.snapshot.channelId"
            class="min-h-0 flex-1"
            :page="page"
            :project="store.snapshot.project"
            :project-path="store.snapshot.projectPath"
            :agent="store.snapshot.agent"
            :channel="store.snapshot.channel"
            :branches="store.snapshot.branches"
            :unread="store.snapshot.unread"
            :pending-approvals="store.snapshot.pendingApprovals"
            :target="{ type: 'channel', id: store.snapshot.channelId }"
            @navigate="navigate"
            @action="action"
            @toggle-branch="toggleBranch"
            :busy="switching"
            @select-channel="selectChannel"
            @open-branch="openBranch"
            @create-channel="newChannelOpen = true"
            @approvals="showApprovals" /></SheetContent
      ></Sheet>
      <Sheet v-model:open="contextOpen"
        ><SheetContent side="right" class="gap-0" :show-close-button="false"
          ><SheetTitle class="sr-only">Workspace context</SheetTitle
          ><SheetDescription class="sr-only">Channel context, members, and reviews</SheetDescription
          ><Button variant="ghost" class="shrink-0" @click="contextOpen = false"
            >Close context</Button
          ><ContextPanel
            class="min-h-0 flex-1"
            :direct-message="page === 'direct-messages'"
            :snapshot="store.snapshot"
            @action="action"
            @approvals="
              () => {
                navigate('conversation')
                contextOpen = false
              }
            " /></SheetContent
      ></Sheet>
    </div>
    <div
      v-else-if="store.snapshot"
      data-ui-id="empty-workspace"
      class="flex flex-1 flex-col items-center justify-center gap-4"
    >
      <p>No projects added</p>
      <Button @click="action('import_project')">Import project folder</Button>
    </div>
    <div v-else class="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
      <LoaderCircle class="size-4 motion-safe:animate-spin" />Opening workspace…
    </div>
    <UiInspector v-if="inspector" @close="inspector = false" />
    <Toaster
      :theme="store.snapshot?.theme.mode === 'light' ? 'light' : 'dark'"
      position="bottom-center"
      :duration="3000"
      close-button
    />
  </main>
</template>
