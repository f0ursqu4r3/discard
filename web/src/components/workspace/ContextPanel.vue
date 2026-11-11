<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { GitBranch, ShieldCheck } from '@lucide/vue'
import type { ProjectChannelDto, SnapshotDto } from '@/bridge/types'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs'
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card'
import { Badge } from '../ui/badge'
import { Button } from '../ui/button'
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription } from '../ui/empty'
import AgentSummary from '../agents/AgentSummary.vue'
import AutorunIndicator from './AutorunIndicator.vue'
import MemberList from '../MemberList.vue'
import MemoryBrowser from '../memory/MemoryBrowser.vue'
import PluginElement from '@/plugin/PluginElement.vue'
import ProjectChannelDetails from './ProjectChannelDetails.vue'
import { channelBranch } from './channel-branch'
import { selectedChannelApprovals } from '../threads'
const props = defineProps<{
  snapshot: SnapshotDto
  projectChannels?: ProjectChannelDto[]
  directMessage?: boolean
  profileRequest?: number
  agentAvatar?: string | null
}>()
const emit = defineEmits<{
  action: [id: string]
  approvals: []
  mainAgentChanged: []
  canonicalChanged: []
}>()
const tab = ref('context')
watch(
  () => [props.snapshot.agentId, props.profileRequest],
  () => {
    tab.value = 'context'
  },
)
const approvals = computed(() => selectedChannelApprovals(props.snapshot))
const branch = computed(() => channelBranch(props.snapshot, props.projectChannels))
const member = computed(() =>
  props.snapshot.turnRunning && props.snapshot.responder
    ? props.snapshot.responder
    : {
        agentId: props.snapshot.agentId,
        name: props.snapshot.agent,
        provider: props.snapshot.provider,
        avatar: props.agentAvatar,
      },
)
const generalRunning = computed(() => {
  const general = props.projectChannels?.find((channel) => channel.kind === 'general')
  return (
    !!general && !!props.snapshot.channels.find((channel) => channel.id === general.id)?.running
  )
})
</script>
<template>
  <Tabs v-model="tab" data-ui-id="context-panel" class="flex h-full min-h-0 flex-col gap-0 bg-card">
    <div
      data-ui-id="context-header"
      class="flex h-16 shrink-0 items-center border-b border-border/60 px-3"
    >
      <TabsList class="w-full">
        <TabsTrigger value="context">{{ directMessage ? 'Profile' : 'Context' }}</TabsTrigger>
        <TabsTrigger value="tasks">Memory</TabsTrigger>
        <!-- TODO: Implement project memberships and saved artifacts. -->
        <TabsTrigger value="artifacts" disabled>
          {{ directMessage ? 'Projects' : 'Artifacts' }} · Soon
        </TabsTrigger>
      </TabsList>
    </div>
    <TabsContent value="context" class="m-0 min-h-0 flex-1 overflow-y-auto overscroll-contain p-3">
      <AgentSummary
        v-if="directMessage"
        :key="snapshot.agentId"
        :agent-id="snapshot.agentId"
        :name="snapshot.agent"
        :status="snapshot.agentStatus"
        :autorun="snapshot.autorun"
        @profile="emit('action', `view_agent_profile:${$event}`)"
      />
      <div v-else class="flex flex-col gap-3">
        <ProjectChannelDetails
          v-if="snapshot.activeProjectId && snapshot.channelId"
          :project-id="snapshot.activeProjectId"
          :channel-id="snapshot.channelId"
          :general-running="generalRunning"
          @main-agent-changed="emit('mainAgentChanged')"
          @canonical-changed="emit('canonicalChanged')"
        />
        <Card v-if="!directMessage" data-ui-id="branch-context" size="sm">
          <CardHeader>
            <CardTitle class="flex items-center gap-2 text-sm">
              <GitBranch class="size-4 shrink-0 text-muted-foreground" />
              <span class="truncate">{{ branch || snapshot.channel }}</span>
              <Badge variant="secondary" class="ml-auto">
                {{ snapshot.archived ? 'Archived' : 'Active' }}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent>
            <dl class="flex flex-col gap-3 text-xs">
              <div class="flex items-baseline font-mono text-xs">
                <dt class="text-muted-foreground">channel</dt>
                <span aria-hidden="true" class="kv-leader" />
                <dd class="text-right truncate">{{ snapshot.channel }}</dd>
              </div>
              <div v-if="branch" class="flex items-baseline font-mono text-xs">
                <dt class="text-muted-foreground">branch</dt>
                <span aria-hidden="true" class="kv-leader" />
                <dd class="text-right truncate">{{ branch }}</dd>
              </div>
              <div class="flex items-baseline font-mono text-xs">
                <dt class="text-muted-foreground">repository</dt>
                <span aria-hidden="true" class="kv-leader" />
                <dd class="text-right min-w-0 truncate" :title="snapshot.projectPath">
                  {{ snapshot.project }}
                </dd>
              </div>
              <div class="flex items-baseline font-mono text-xs">
                <dt class="text-muted-foreground">status</dt>
                <span aria-hidden="true" class="kv-leader" />
                <dd class="flex flex-wrap items-center justify-end gap-x-2 gap-y-1 text-right">
                  <span>{{ snapshot.turnRunning ? 'Running' : 'Idle' }}</span>
                  <AutorunIndicator :autorun="snapshot.autorun" />
                </dd>
              </div>
            </dl>
          </CardContent>
        </Card>
        <MemberList
          :agent-id="member.agentId"
          v-if="directMessage || !snapshot.slots.member_list"
          :agent="member.name"
          :avatar="member.avatar"
          :status="snapshot.agentStatus"
          :provider="member.provider"
          :target="{
            type: 'agent',
            id: member.agentId,
            name: member.name,
          }"
          @settings="emit('action', `view_agent_profile:${member.agentId}`)"
        />
        <Card
          v-if="!directMessage && snapshot.slots.member_list"
          data-ui-id="context-plugin-slot"
          size="sm"
          class="max-h-96 overflow-auto overscroll-contain"
        >
          <CardContent>
            <PluginElement
              :element="snapshot.slots.member_list.element"
              :plugin="snapshot.slots.member_list.plugin"
            />
          </CardContent>
        </Card>
        <Card v-if="!directMessage" data-ui-id="review-queue" size="sm">
          <CardHeader>
            <CardTitle class="flex items-center gap-2 text-sm">
              <ShieldCheck class="size-4 text-muted-foreground" />
              Review queue
            </CardTitle>
          </CardHeader>
          <CardContent class="flex flex-col gap-3 text-xs text-muted-foreground">
            <p>
              {{
                approvals
                  ? `${approvals} pending approval${approvals === 1 ? '' : 's'}`
                  : 'No pending approvals'
              }}
            </p>
            <Button v-if="approvals" variant="outline" size="sm" @click="emit('approvals')">
              View approvals
            </Button>
            <Button
              v-if="snapshot.banners.reviewCount"
              variant="outline"
              size="sm"
              @click="emit('action', 'review_plugins')"
            >
              Review {{ snapshot.banners.reviewCount }} plugin changes
            </Button>
          </CardContent>
        </Card>
      </div>
    </TabsContent>
    <TabsContent value="tasks" class="m-0 overflow-y-auto">
      <MemoryBrowser
        :channel-id="snapshot.channelId"
        :agent-id="snapshot.agentId"
        :project-id="snapshot.activeProjectId"
        :revision="snapshot.memoryRev"
        :running="snapshot.turnRunning"
      />
    </TabsContent>
    <TabsContent value="artifacts" class="m-0 overflow-y-auto">
      <Empty>
        <EmptyHeader>
          <EmptyTitle>
            {{ directMessage ? 'Project memberships' : 'No saved artifacts' }}
          </EmptyTitle>
          <EmptyDescription>
            {{
              directMessage
                ? 'Project invitations are not available yet.'
                : 'Code and changes appear in the conversation.'
            }}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    </TabsContent>
  </Tabs>
</template>
