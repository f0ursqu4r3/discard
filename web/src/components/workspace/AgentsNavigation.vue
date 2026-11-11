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
import { Plus } from '@lucide/vue'
import BrailleSpinner from '../BrailleSpinner.vue'
import type { ChannelDto } from '@/bridge/types'
import AgentAvatar from '@/components/agents/AgentAvatar.vue'
import ThreadRow from '../ThreadRow.vue'
import { threadLinks } from '../threads'

const props = defineProps<{
  channels?: ChannelDto[]
  projectId: string | null
  project?: string
  channelId?: string
  busy?: boolean
  /** The thread panel shows this thread. */
  openThreadId?: string | null
}>()
const emit = defineEmits<{
  select: [id: string]
  action: [id: string]
  selectThread: [threadId: string]
  leaveThread: [threadId: string]
}>()
/** Followed threads of one conversation. Live state wins over what storage holds. */
function threadsOf(id: string) {
  return threadLinks(id, props.channels, conversations.value)
}
/** A direct message contact can have several conversations. */
function contactThreads(agentId: string) {
  return conversations.value
    .filter((c) => c.agentId === agentId && c.projectId === null)
    .flatMap((c) => threadsOf(c.id))
}
const agents = ref<AgentEntry[]>([]),
  conversations = ref<ConversationEntry[]>([])
const error = ref(''),
  opening = ref(false)
const stops: UnlistenFn[] = []
let disposed = false
const working = computed(
  () =>
    new Set(props.channels?.filter((channel) => channel.running).map((channel) => channel.agentId)),
)
const contacts = computed(() =>
  agents.value.filter((agent) =>
    conversations.value.some((c) => c.agentId === agent.id && c.projectId === null && c.inDmList),
  ),
)
const visible = computed(() =>
  props.projectId === null
    ? []
    : conversations.value.filter((c) => c.projectId === props.projectId),
)
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
    <Button variant="ghost" size="sm" class="justify-start" @click="emit('action', 'show_friends')">
      Agents
    </Button>
    <template v-if="projectId === null">
      <h2 class="section-label px-2 pt-4">Direct Messages</h2>
      <template v-for="agent in contacts" :key="agent.id">
        <Button
          :variant="
            conversations.some(
              (c) => c.id === channelId && c.agentId === agent.id && c.projectId === null,
            )
              ? 'secondary'
              : 'ghost'
          "
          size="sm"
          class="h-auto justify-start gap-2 py-2"
          :class="{
            'nav-active-edge': conversations.some(
              (c) => c.id === channelId && c.agentId === agent.id && c.projectId === null,
            ),
          }"
          :disabled="busy || opening"
          @click="open(agent.id)"
        >
          <AgentAvatar
            :agent-id="agent.id"
            :name="agent.name"
            :avatar="agent.avatar"
            class="size-7"
          />
          <span class="min-w-0 text-left">
            <span class="block truncate">
              {{ agent.name }}
              <span v-if="agent.archived">· Archived</span>
            </span>
            <span class="block truncate text-xs text-muted-foreground">
              {{ agent.provider }} · {{ agent.model }}
            </span>
          </span>
          <BrailleSpinner
            v-if="working.has(agent.id)"
            :agent-id="agent.id"
            :avatar="agent.avatar"
            class="ml-auto shrink-0"
          />
        </Button>
        <ThreadRow
          v-for="(link, index) in contactThreads(agent.id)"
          :key="link.id"
          :link="link"
          :is-last="index === contactThreads(agent.id).length - 1"
          :active="link.id === openThreadId"
          :busy="busy"
          @select="emit('selectThread', $event)"
          @leave="emit('leaveThread', $event)"
        />
      </template>
      <p v-if="!contacts.length" class="px-2 py-2 text-xs text-muted-foreground">
        Choose an agent in Agents and select DM.
      </p>
    </template>
    <div v-if="projectId !== null" class="mt-4 flex items-center justify-between px-2">
      <h2 class="section-label flex-1">Conversations</h2>
      <!-- TODO: Create project conversations only after branch selection is part of this flow. -->
      <Button variant="ghost" size="sm" disabled>
        <Plus />
        <span class="sr-only">New conversation</span>
        <span class="text-xs text-muted-foreground">Soon</span>
      </Button>
    </div>
    <template v-for="conversation in visible" :key="conversation.id">
      <Button
        :variant="conversation.id === channelId ? 'secondary' : 'ghost'"
        :class="{ 'nav-active-edge': conversation.id === channelId }"
        size="sm"
        class="h-auto justify-start py-2 text-left"
        :disabled="busy"
        @click="emit('select', conversation.id)"
      >
        <span class="min-w-0">
          <span class="block truncate font-mono text-[0.92em]">{{ conversation.name }}</span>
          <span class="block truncate text-xs text-muted-foreground">
            {{ conversation.agent }}
            <span v-if="conversation.deleted">· Read only</span>
            <span v-else-if="conversation.agentArchived || conversation.archived">· Archived</span>
          </span>
        </span>
        <BrailleSpinner
          v-if="channels?.some((channel) => channel.id === conversation.id && channel.running)"
          :agent-id="conversation.agentId"
          :avatar="conversation.avatar"
          class="ml-auto shrink-0"
        />
      </Button>
      <ThreadRow
        v-for="(link, index) in threadsOf(conversation.id)"
        :key="link.id"
        :link="link"
        :is-last="index === threadsOf(conversation.id).length - 1"
        :active="link.id === openThreadId"
        :busy="busy"
        @select="emit('selectThread', $event)"
        @leave="emit('leaveThread', $event)"
      />
    </template>
    <p v-if="projectId !== null && !visible.length" class="px-2 py-2 text-xs text-muted-foreground">
      No conversations in this project.
    </p>
    <p v-if="error" role="alert" class="px-2 py-2 text-xs">{{ error }}</p>
  </section>
</template>
