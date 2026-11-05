<script setup lang="ts">
import { useStore } from './bridge/store'
import { useWebShortcuts } from './bridge/shortcuts'
import { setMemberListVisible, setPanels, toggleBranch } from './bridge/api'
import ServerRail from './components/ServerRail.vue'
import ChannelList from './components/ChannelList.vue'
import ChannelHeader from './components/ChannelHeader.vue'
import MemberList from './components/MemberList.vue'
import MessageList from './components/MessageList.vue'
import Composer from './components/Composer.vue'
import Notices from './components/Notices.vue'
import { thinkingLine } from './components/messages'
import { panelWidths, savePanelWidths, settingsTarget } from './components/layout'
import PluginElement from './plugin/PluginElement.vue'
import { sendMessage, cancelTurn, runAction } from './bridge/api'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from './components/ui/resizable'
const store = useStore()
useWebShortcuts()
function savePanels(sizes: number[]) {
  if (!savePanelWidths(!!store.snapshot?.memberListVisible, sizes.length)) return
  const [channel, member] = panelWidths(sizes)
  void setPanels(channel, member)
}
</script>
<template>
  <main data-testid="app-root" class="flex h-screen overflow-hidden bg-background text-foreground">
    <template v-if="store.snapshot">
      <ServerRail :project="store.snapshot.project" :unread="store.snapshot.unread" :pending-approvals="store.snapshot.pendingApprovals" :review-count="store.snapshot.banners.reviewCount" :target="{ type: 'server', id: store.snapshot.projectId, name: store.snapshot.project, path: store.snapshot.projectPath }" @settings="runAction(settingsTarget(store.snapshot.banners.reviewCount)[0])" />
      <div class="min-w-0 flex-1">
        <ResizablePanelGroup direction="horizontal" @layout="savePanels">
          <ResizablePanel :order="1" :default-size="store.snapshot.panels.channelList" size-unit="px" :min-size="180" :max-size="400"><ChannelList :project="store.snapshot.project" :channel="store.snapshot.channel" :unread="store.snapshot.unread" :pending-approvals="store.snapshot.pendingApprovals" :branches="store.snapshot.branches" :target="{ type: 'channel', id: store.snapshot.channelId }" @toggle-branch="toggleBranch" /></ResizablePanel>
          <ResizableHandle />
          <ResizablePanel :order="2" :min-size="20">
            <section class="flex h-full flex-col">
              <ChannelHeader :channel="store.snapshot.channel" :member-list-visible="store.snapshot.memberListVisible" @toggle-members="setMemberListVisible(!store.snapshot!.memberListVisible)"><PluginElement v-if="store.snapshot.slots.channel_header" :element="store.snapshot.slots.channel_header.element" :plugin="store.snapshot.slots.channel_header.plugin" /><strong v-else class="block truncate border-b border-border px-4 py-3"># {{ store.snapshot.channel }}</strong></ChannelHeader>
              <MessageList :messages="store.history" :live="store.snapshot.liveBlocks" :running="store.snapshot.turnRunning" :new-anchor-id="store.snapshot.newAnchorId" :agent="store.snapshot.agent" :prompts="store.snapshot.prompts" />
              <Notices :banners="store.snapshot.banners" :notice="store.snapshot.notice" :archived="store.snapshot.archived" />
              <p v-if="thinkingLine(store.snapshot.turnRunning, [store.snapshot.agent])" class="px-4 py-1 text-sm text-muted-foreground">{{ thinkingLine(store.snapshot.turnRunning, [store.snapshot.agent]) }}</p>
              <Composer :channel="store.snapshot.channel" :archived="store.snapshot.archived" :busy="store.snapshot.archiveBusy || !!store.snapshot.banners.archive" :running="store.snapshot.turnRunning" @send="sendMessage" @stop="cancelTurn" />
            </section>
          </ResizablePanel>
          <template v-if="store.snapshot.memberListVisible">
            <ResizableHandle />
            <ResizablePanel :order="3" :default-size="store.snapshot.panels.memberList" size-unit="px" :min-size="180" :max-size="400"><div v-if="store.snapshot.slots.member_list" class="h-full overflow-auto"><PluginElement :element="store.snapshot.slots.member_list.element" :plugin="store.snapshot.slots.member_list.plugin" /></div><MemberList v-else :agent="store.snapshot.agent" :status="store.snapshot.agentStatus" :provider="store.snapshot.provider" :target="{ type: 'agent', id: store.snapshot.agentId, name: store.snapshot.agent }" /></ResizablePanel>
          </template>
        </ResizablePanelGroup>
      </div>
    </template>
  </main>
</template>
