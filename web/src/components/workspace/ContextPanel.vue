<script setup lang="ts">
import { computed, ref } from 'vue'
import { GitBranch, FolderGit2, Hash, Activity, ShieldCheck } from '@lucide/vue'
import type { SnapshotDto } from '@/bridge/types'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs'
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card'
import { Badge } from '../ui/badge'
import { Button } from '../ui/button'
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription } from '../ui/empty'
import MemberList from '../MemberList.vue'
import PluginElement from '@/plugin/PluginElement.vue'
const props = defineProps<{ snapshot: SnapshotDto; directMessage?: boolean }>()
const emit = defineEmits<{ action: [id: string]; approvals: [] }>()
const tab = ref('context')
const branch = computed(() => props.snapshot.branches.find((row) => row.current)?.label)
</script>
<template>
  <Tabs v-model="tab" data-ui-id="context-panel" class="flex h-full min-h-0 flex-col gap-0 bg-card">
    <div
      data-ui-id="context-header"
      class="flex h-16 shrink-0 items-center border-b border-border/60 px-3"
    >
      <TabsList class="w-full"
        ><TabsTrigger value="context">{{ directMessage ? 'Profile' : 'Context' }}</TabsTrigger
        ><TabsTrigger value="tasks">{{ directMessage ? 'Memory' : 'Tasks' }}</TabsTrigger
        ><TabsTrigger value="artifacts">{{
          directMessage ? 'Projects' : 'Artifacts'
        }}</TabsTrigger></TabsList
      >
    </div>
    <TabsContent value="context" class="m-0 min-h-0 flex-1 overflow-y-auto overscroll-contain p-3">
      <div class="flex flex-col gap-3">
        <Card v-if="!directMessage" data-ui-id="branch-context" size="sm"
          ><CardHeader
            ><CardTitle class="flex items-center gap-2 text-sm"
              ><GitBranch class="size-4 shrink-0 text-muted-foreground" /><span class="truncate">{{
                branch || snapshot.channel
              }}</span
              ><Badge variant="secondary" class="ml-auto">{{
                snapshot.archived ? 'Archived' : 'Active'
              }}</Badge></CardTitle
            ></CardHeader
          ><CardContent>
            <dl class="flex flex-col gap-3 text-xs">
              <div class="flex items-center gap-2">
                <dt class="flex items-center gap-1.5 text-muted-foreground">
                  <Hash class="size-3" />Channel
                </dt>
                <dd class="ml-auto truncate">{{ snapshot.channel }}</dd>
              </div>
              <div v-if="branch" class="flex items-center gap-2">
                <dt class="flex items-center gap-1.5 text-muted-foreground">
                  <GitBranch class="size-3" />Branch
                </dt>
                <dd class="ml-auto truncate">{{ branch }}</dd>
              </div>
              <div class="flex items-center gap-2">
                <dt class="flex items-center gap-1.5 text-muted-foreground">
                  <FolderGit2 class="size-3" />Repository
                </dt>
                <dd class="ml-auto min-w-0 truncate" :title="snapshot.projectPath">
                  {{ snapshot.project }}
                </dd>
              </div>
              <div class="flex items-center gap-2">
                <dt class="flex items-center gap-1.5 text-muted-foreground">
                  <Activity class="size-3" />Status
                </dt>
                <dd class="ml-auto">{{ snapshot.turnRunning ? 'Running' : 'Idle' }}</dd>
              </div>
            </dl>
          </CardContent></Card
        >
        <MemberList
          v-if="directMessage || !snapshot.slots.member_list"
          :agent="snapshot.agent"
          :status="snapshot.agentStatus"
          :provider="snapshot.provider"
          :target="{ type: 'agent', id: snapshot.agentId, name: snapshot.agent }"
          @settings="emit('action', 'open_agent_settings')"
        />
        <div
          v-if="!directMessage && snapshot.slots.member_list"
          data-ui-id="context-plugin-slot"
          class="max-h-96 overflow-auto overscroll-contain"
        >
          <PluginElement
            :element="snapshot.slots.member_list.element"
            :plugin="snapshot.slots.member_list.plugin"
          />
        </div>
        <Card v-if="!directMessage" data-ui-id="review-queue" size="sm"
          ><CardHeader
            ><CardTitle class="flex items-center gap-2 text-sm"
              ><ShieldCheck class="size-4 text-muted-foreground" />Review queue</CardTitle
            ></CardHeader
          ><CardContent class="flex flex-col gap-3 text-xs text-muted-foreground"
            ><p>
              {{
                snapshot.pendingApprovals
                  ? `${snapshot.pendingApprovals} pending approval${snapshot.pendingApprovals === 1 ? '' : 's'}`
                  : 'No pending approvals'
              }}
            </p>
            <Button
              v-if="snapshot.pendingApprovals"
              variant="outline"
              size="sm"
              @click="emit('approvals')"
              >View approvals</Button
            ><Button
              v-if="snapshot.banners.reviewCount"
              variant="outline"
              size="sm"
              @click="emit('action', 'review_plugins')"
              >Review {{ snapshot.banners.reviewCount }} plugin changes</Button
            ></CardContent
          ></Card
        >
      </div>
    </TabsContent>
    <TabsContent value="tasks" class="m-0 overflow-y-auto"
      ><Empty
        ><EmptyHeader
          ><EmptyTitle>{{ directMessage ? 'Agent memory' : 'No task list' }}</EmptyTitle
          ><EmptyDescription>{{
            directMessage
              ? 'Review stored notes in Settings → Memory.'
              : 'Task updates appear in the conversation.'
          }}</EmptyDescription></EmptyHeader
        ></Empty
      ></TabsContent
    >
    <TabsContent value="artifacts" class="m-0 overflow-y-auto"
      ><Empty
        ><EmptyHeader
          ><EmptyTitle>{{
            directMessage ? 'Project memberships' : 'No saved artifacts'
          }}</EmptyTitle
          ><EmptyDescription>{{
            directMessage
              ? 'Project invitations are not available yet.'
              : 'Code and changes appear in the conversation.'
          }}</EmptyDescription></EmptyHeader
        ></Empty
      ></TabsContent
    >
  </Tabs>
</template>
