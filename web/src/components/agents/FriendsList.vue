<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getAgents, type AgentEntry } from '@/bridge/api'
import { Users, UserPlus } from '@lucide/vue'
import { Avatar, AvatarImage, AvatarFallback } from '@/components/ui/avatar'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription, EmptyMedia } from '@/components/ui/empty'
import { serverInitials } from '@/components/layout'
const emit = defineEmits<{ action: [id: string] }>()
const agents = ref<AgentEntry[]>([]),
  query = ref(''),
  filter = ref('active'),
  error = ref(''),
  loading = ref(true)
let stop: UnlistenFn | undefined,
  disposed = false,
  generation = 0
const visible = computed(() =>
  agents.value.filter(
    (a) =>
      a.archived === (filter.value === 'archived') &&
      a.name.toLocaleLowerCase().includes(query.value.trim().toLocaleLowerCase()),
  ),
)
async function reload() {
  const current = ++generation
  try {
    const rows = await getAgents()
    if (!disposed && current === generation) {
      agents.value = rows
      error.value = ''
    }
  } catch (cause) {
    if (!disposed && current === generation) error.value = String(cause)
  } finally {
    if (!disposed && current === generation) loading.value = false
  }
}
onMounted(async () => {
  try {
    const unlisten = await listen('discard://agents', () => void reload())
    if (disposed) {
      unlisten()
      return
    }
    stop = unlisten
    await reload()
  } catch (cause) {
    if (!disposed) {
      error.value = String(cause)
      loading.value = false
    }
  }
})
onUnmounted(() => {
  disposed = true
  stop?.()
})
</script>
<template>
  <section data-ui-id="friends-list" class="flex min-h-0 flex-1 flex-col">
    <div class="flex flex-wrap items-center gap-4 border-b border-border/60 px-5 py-3">
      <h2 class="flex items-center gap-2 font-semibold">
        <Users class="size-5" />
        Agents
      </h2>
      <Tabs v-model="filter">
        <TabsList variant="line">
          <TabsTrigger value="active">All agents</TabsTrigger>
          <TabsTrigger value="archived">Archived</TabsTrigger>
        </TabsList>
      </Tabs>
      <Button class="ml-auto" size="sm" @click="emit('action', 'add_agent')">
        <UserPlus data-icon="inline-start" />
        Add agent
      </Button>
    </div>
    <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-5">
      <Input v-model="query" aria-label="Search agents" placeholder="Search agents" />
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription>
          {{ error }}
          <Button variant="outline" size="sm" @click="reload">Retry</Button>
        </AlertDescription>
      </Alert>
      <p v-else-if="loading" role="status">Loading agents…</p>
      <template v-else>
        <p class="text-xs font-semibold text-muted-foreground">
          {{ filter === 'archived' ? 'Archived' : 'All agents' }} — {{ visible.length }}
        </p>
        <ul v-if="visible.length" class="flex flex-col divide-y divide-border/60">
          <li v-for="agent in visible" :key="agent.id" class="flex items-center gap-3 py-3">
            <button
              class="flex min-w-0 flex-1 items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :aria-label="
                agent.archived ? `View ${agent.name}'s profile` : `Open chat with ${agent.name}`
              "
              @click="
                emit(
                  'action',
                  `${agent.archived ? 'view_agent_profile' : 'message_agent'}:${agent.id}`,
                )
              "
            >
              <Avatar class="size-10">
                <AvatarImage v-if="agent.avatar" :src="agent.avatar" />
                <AvatarFallback>{{ serverInitials(agent.name) }}</AvatarFallback>
              </Avatar>
              <span class="min-w-0">
                <span class="block truncate font-medium">{{ agent.name }}</span>
                <span class="block truncate text-sm text-muted-foreground">
                  {{ agent.provider }} · {{ agent.model }}
                </span>
              </span>
            </button>
            <Button
              variant="outline"
              size="sm"
              @click="emit('action', `view_agent_profile:${agent.id}`)"
            >
              View profile
            </Button>
            <Button
              v-if="!agent.archived"
              variant="ghost"
              size="sm"
              :aria-label="`DM ${agent.name}`"
              @click="emit('action', `message_agent:${agent.id}`)"
            >
              DM
            </Button>
          </li>
        </ul>
        <Empty v-else>
          <EmptyHeader>
            <EmptyMedia variant="icon"><Users /></EmptyMedia>
            <EmptyTitle>
              {{
                query
                  ? 'No matching agents'
                  : filter === 'archived'
                    ? 'No archived agents'
                    : 'Your agent list starts here'
              }}
            </EmptyTitle>
            <EmptyDescription>
              {{
                query
                  ? 'Try another name.'
                  : filter === 'archived'
                    ? 'Archived profiles appear here. Open a profile to restore an agent.'
                    : 'Add an agent, then open their profile or send a message.'
              }}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      </template>
    </div>
  </section>
</template>
