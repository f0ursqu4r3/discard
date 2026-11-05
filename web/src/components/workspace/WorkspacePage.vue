<script setup lang="ts">
import { computed, ref } from 'vue'
import { Bot, FileText, FolderGit2, GitBranch, MessagesSquare, Settings, Cpu } from '@lucide/vue'
import type { MessageDto, SnapshotDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import BrandMark from './BrandMark.vue'
import MemberList from '../MemberList.vue'
import { Button } from '../ui/button'
import { Badge } from '../ui/badge'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '../ui/card'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription, EmptyContent } from '../ui/empty'
defineOptions({ name: 'WorkspacePageView' })
const props = defineProps<{ page: WorkspacePage; snapshot: SnapshotDto; messages: MessageDto[] }>()
const emit = defineEmits<{ action: [id: string]; conversation: [] }>()
const dmTab = ref('conversation')
const branchCount = computed(() => props.snapshot.branches.filter(row => row.kind === 'branch').length)
</script>
<template>
  <div v-if="page === 'direct-messages'" data-ui-id="agent-dm" class="flex min-h-0 flex-1 flex-col">
    <Tabs v-model="dmTab" class="flex min-h-0 flex-1 flex-col gap-0">
      <div class="flex h-11 shrink-0 items-center overflow-x-auto border-b border-border/60 px-3 sm:px-5"><TabsList variant="line"><TabsTrigger value="conversation">Conversation</TabsTrigger><TabsTrigger value="personality">Personality</TabsTrigger><TabsTrigger value="memory">Memory</TabsTrigger><TabsTrigger value="settings">Settings</TabsTrigger></TabsList></div>
      <TabsContent value="conversation" class="m-0 flex min-h-0 flex-1 flex-col overflow-auto overscroll-contain"><Empty class="m-auto"><EmptyHeader><EmptyMedia variant="icon"><MessagesSquare /></EmptyMedia><EmptyTitle>Direct messages with {{ snapshot.agent }}</EmptyTitle><EmptyDescription>Direct messages are not available yet. Your project conversation stays in #{{ snapshot.channel }}.</EmptyDescription></EmptyHeader><EmptyContent><Button variant="outline" @click="emit('conversation')">Open #{{ snapshot.channel }}</Button></EmptyContent></Empty></TabsContent>
      <TabsContent value="personality" class="m-0 overflow-auto overscroll-contain p-6"><Card size="sm"><CardHeader><CardTitle>Personality &amp; role</CardTitle><CardDescription>Set how {{ snapshot.agent }} approaches their work.</CardDescription></CardHeader><CardContent><Button variant="outline" @click="emit('action', 'open_agent_settings')"><Settings data-icon="inline-start" />Edit agent instructions</Button></CardContent></Card></TabsContent>
      <TabsContent value="memory" class="m-0 overflow-auto overscroll-contain p-6"><Empty><EmptyHeader><EmptyMedia variant="icon"><Cpu /></EmptyMedia><EmptyTitle>Agent memory</EmptyTitle><EmptyDescription>Review stored notes in Settings → Memory.</EmptyDescription></EmptyHeader><EmptyContent><Button variant="outline" @click="emit('action', 'open_settings')">Open settings</Button></EmptyContent></Empty></TabsContent>
      <TabsContent value="settings" class="m-0 overflow-auto overscroll-contain p-6"><Card size="sm"><CardHeader><CardTitle>{{ snapshot.agent }}</CardTitle><CardDescription>Provider and model belong to this agent.</CardDescription></CardHeader><CardContent class="flex flex-col items-start gap-4"><Badge variant="secondary">{{ snapshot.provider }}</Badge><Button variant="outline" @click="emit('action', 'open_agent_settings')">Open agent settings</Button></CardContent></Card></TabsContent>
    </Tabs>
  </div>
  <div v-else data-ui-id="project-page" class="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 sm:p-6">
    <div v-if="page === 'overview'" class="mx-auto flex max-w-4xl flex-col gap-6">
      <div class="flex items-center gap-4"><BrandMark class="size-12" /><div class="min-w-0"><h2 class="text-xl font-semibold">{{ snapshot.project }}</h2><p class="truncate text-sm text-muted-foreground">{{ snapshot.projectPath }}</p></div></div>
      <div class="grid grid-cols-2 gap-3 lg:grid-cols-4"><Card v-for="stat in [{ label: 'Branches', value: branchCount, icon: GitBranch }, { label: 'Channel', value: 1, icon: MessagesSquare }, { label: 'Agent', value: 1, icon: Bot }, { label: 'Messages', value: messages.length, icon: FileText }]" :key="stat.label" size="sm"><CardHeader><CardDescription>{{ stat.label }}</CardDescription></CardHeader><CardContent class="flex items-center gap-2 text-xl font-semibold"><component :is="stat.icon" class="size-4 text-primary" />{{ stat.value }}</CardContent></Card></div>
      <Card size="sm"><CardHeader><CardTitle>Current work</CardTitle></CardHeader><CardContent><Button variant="ghost" class="w-full justify-start" @click="emit('conversation')"><FolderGit2 data-icon="inline-start" /><span class="truncate">{{ snapshot.channel }}</span><Badge variant="secondary" class="ml-auto">{{ snapshot.turnRunning ? 'Running' : 'Idle' }}</Badge></Button></CardContent></Card>
    </div>
    <div v-else-if="page === 'agents'" class="mx-auto flex max-w-3xl flex-col gap-5"><div class="flex items-center justify-between gap-4"><h2 class="text-xl font-semibold">Project agents</h2><Button variant="outline" size="sm" @click="emit('action', 'open_agent_settings')"><Settings data-icon="inline-start" />Manage agents</Button></div><MemberList :agent="snapshot.agent" :status="snapshot.agentStatus" :provider="snapshot.provider" :target="{ type: 'agent', id: snapshot.agentId, name: snapshot.agent }" @settings="emit('action', 'open_agent_settings')" /></div>
    <Empty v-else-if="page === 'memory'"><EmptyHeader><EmptyMedia variant="icon"><Cpu /></EmptyMedia><EmptyTitle>Project memory</EmptyTitle><EmptyDescription>Review project and agent notes in Settings → Memory.</EmptyDescription></EmptyHeader><EmptyContent><Button variant="outline" @click="emit('action', 'open_settings')">Open settings</Button></EmptyContent></Empty>
    <Empty v-else><EmptyHeader><EmptyMedia variant="icon"><FileText /></EmptyMedia><EmptyTitle>No saved artifacts</EmptyTitle><EmptyDescription>Code and file changes appear in the conversation.</EmptyDescription></EmptyHeader><EmptyContent><Button variant="outline" @click="emit('conversation')">Open conversation</Button></EmptyContent></Empty>
  </div>
</template>
