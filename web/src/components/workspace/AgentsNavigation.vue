<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  getAgents,
  getConversations,
  openAgentChat,
  type AgentEntry,
  type ConversationEntry,
} from '@/bridge/api'
import { Button } from '../ui/button'
import { Avatar, AvatarFallback, AvatarImage } from '../ui/avatar'
import { serverInitials } from '../layout'
import CreateChannelDialog from './CreateChannelDialog.vue'
const props = defineProps<{
  projectId: string | null
  project?: string
  channelId?: string
  busy?: boolean
}>()
const emit = defineEmits<{ select: [id: string]; action: [id: string] }>()
const agents = ref<AgentEntry[]>([]),
  conversations = ref<ConversationEntry[]>([])
const error = ref(''),
  opening = ref(false),
  creating = ref(false)
const stops: UnlistenFn[] = []
let disposed = false
const visible = computed(() => conversations.value.filter((c) => c.projectId === props.projectId))
async function reload() {
  try {
    ;[agents.value, conversations.value] = await Promise.all([getAgents(), getConversations()])
    error.value = ''
  } catch (cause) {
    error.value = String(cause)
  }
}
async function open(agentId: string) {
  if (opening.value) return
  opening.value = true
  try {
    const id = await openAgentChat(agentId)
    emit('select', id)
  } catch (cause) {
    error.value = String(cause)
  } finally {
    opening.value = false
  }
}
function created(id: string) {
  void reload()
  emit('select', id)
}
watch(() => props.projectId, reload)
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
    await reload()
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  stops.forEach((stop) => stop())
})
</script>
<template>
  <section class="flex flex-col gap-1" aria-label="Agents and conversations">
    <Button
      variant="outline"
      size="sm"
      class="justify-start"
      @click="emit('action', 'open_agent_settings')"
    >
      Manage agents
    </Button>
    <template v-if="projectId === null">
      <h2 class="px-2 pt-4 text-xs font-semibold text-muted-foreground">Agents</h2>
      <Button
        v-for="agent in agents"
        :key="agent.id"
        variant="ghost"
        size="sm"
        class="h-auto justify-start gap-2 py-2"
        :disabled="busy || opening"
        @click="open(agent.id)"
      >
        <Avatar class="size-7">
          <AvatarImage v-if="agent.avatar" :src="agent.avatar" />
          <AvatarFallback>{{ serverInitials(agent.name) }}</AvatarFallback>
        </Avatar>
        <span class="min-w-0 text-left">
          <span class="block truncate">
            {{ agent.name }}
            <span v-if="agent.archived">· Archived</span>
          </span>
          <span class="block truncate text-xs text-muted-foreground">
            {{ agent.provider }} · {{ agent.model }}
          </span>
        </span>
      </Button>
      <p v-if="!agents.length" class="px-2 py-2 text-xs text-muted-foreground">
        Create an agent in Manage agents.
      </p>
    </template>
    <div class="mt-4 flex items-center justify-between px-2">
      <h2 class="text-xs font-semibold text-muted-foreground">Conversations</h2>
      <Button
        variant="ghost"
        size="sm"
        :disabled="busy || !agents.some((a) => !a.archived)"
        @click="creating = true"
      >
        New
      </Button>
    </div>
    <Button
      v-for="conversation in visible"
      :key="conversation.id"
      :variant="conversation.id === channelId ? 'secondary' : 'ghost'"
      size="sm"
      class="h-auto justify-start py-2 text-left"
      :disabled="busy"
      @click="emit('select', conversation.id)"
    >
      <span class="min-w-0">
        <span class="block truncate">{{ conversation.name }}</span>
        <span class="block truncate text-xs text-muted-foreground">
          {{ conversation.agent }}
          <span v-if="conversation.deleted">· Read only</span>
          <span v-else-if="conversation.agentArchived || conversation.archived">· Archived</span>
        </span>
      </span>
    </Button>
    <p v-if="!visible.length" class="px-2 py-2 text-xs text-muted-foreground">
      No conversations in {{ projectId === null ? 'direct chats' : 'this project' }}.
    </p>
    <p v-if="error" role="alert" class="px-2 py-2 text-xs">{{ error }}</p>
    <CreateChannelDialog
      v-model:open="creating"
      :project="project || 'Direct chat'"
      :project-id="projectId"
      @created="created"
    />
  </section>
</template>
