<script setup lang="ts">
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from '@/components/ui/select'
import { ref, useId, watch } from 'vue'
import { Field, FieldLabel } from '../ui/field'
import { invoke } from '@tauri-apps/api/core'
import { useMemory } from './useMemory'
import type { MemoryItem } from './types'
import MemoryProposalCard from './MemoryProposal.vue'
import MemoryCaptureStatus from './MemoryCaptureStatus.vue'
import { Button } from '../ui/button'
const props = defineProps<{
  channelId?: string
  agentId?: string
  projectId?: string | null
  revision?: number
  running?: boolean
}>()
const {
  data,
  error: loadError,
  loading,
  reload,
} = useMemory(
  () => props.channelId ?? null,
  () => props.channelId === undefined || !!props.channelId,
)
const selected = ref<MemoryItem | null>(null)
const draft = ref('')
const target = ref('')
const filter = ref('all')
const filterId = useId()
const targetId = useId()
const error = ref('')
const busy = ref(false)
function inspect(item: MemoryItem) {
  selected.value = item
  draft.value = item.content
  target.value = item.scope
  error.value = ''
}
function date(value: number) {
  return new Date(value).toLocaleString()
}
async function run(action: () => Promise<unknown>, refresh = true) {
  busy.value = true
  error.value = ''
  try {
    await action()
    if (refresh) await reload()
    return true
  } catch (cause) {
    error.value = String(cause)
    return false
  } finally {
    busy.value = false
  }
}
async function mutate(action: string) {
  const item = selected.value
  if (!item) return
  if (
    await run(() =>
      invoke('memory_mutate', {
        id: item.id,
        expectedVersion: item.version,
        action,
        content: draft.value,
        scope: target.value,
        channelId: props.channelId ?? null,
      }),
    )
  ) {
    const current = data.value.items.find((i) => i.id === item.id)
    if (current) inspect(current)
    else selected.value = null
  }
}
async function refresh(importEdits = false) {
  if (importEdits && !(await run(() => invoke('memory_reconcile')))) return
  if (!importEdits) await reload()
  const current = data.value.items.find((i) => i.id === selected.value?.id)
  if (current) inspect(current)
  else selected.value = null
}
watch(
  () => props.channelId,
  () => {
    selected.value = null
    error.value = ''
    filter.value = 'all'
  },
)
watch([() => props.revision, () => props.running], () => {
  void reload()
})
</script>
<template>
  <div class="space-y-4 p-3 text-sm" data-ui-id="memory-browser">
    <p class="font-medium">
      {{ channelId === undefined ? 'All saved memory' : 'Conversation memory' }}
    </p>
    <p class="text-xs text-muted-foreground">
      Current instructions take priority. Memory is refreshed before each response.
    </p>
    <div class="flex flex-wrap gap-2">
      <Button size="sm" variant="outline" :disabled="busy || loading" @click="refresh()">
        Reload memory
      </Button>
      <Button size="sm" variant="outline" :disabled="busy" @click="refresh(true)">
        Import file edits and reload
      </Button>
    </div>
    <p v-if="error || loadError" role="alert" class="text-destructive">
      {{ error || loadError }}
    </p>
    <p v-for="warning in data.warnings ?? []" :key="warning" role="status">{{ warning }}</p>
    <p v-if="channelId === ''">Choose a conversation to see applicable memory.</p>
    <MemoryCaptureStatus
      v-if="channelId"
      :channel-id="channelId"
      :paused="data.paused"
      @changed="reload"
    />
    <Field v-if="channelId === undefined">
      <FieldLabel :for="filterId">Domain</FieldLabel>
      <Select v-model="filter">
        <SelectTrigger :id="filterId" class="w-full">
          <SelectValue placeholder="All domains" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="all">All domains</SelectItem>
            <SelectItem v-for="scope in data.scopes" :key="scope.scope" :value="scope.scope">
              {{ scope.title }}
            </SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
    </Field>
    <MemoryProposalCard
      v-for="proposal in data.proposals"
      :key="proposal.id"
      :proposal="proposal"
      :channel-id="channelId"
      @changed="reload"
    />
    <p v-if="!loading && !data.items.length" class="text-muted-foreground">
      No saved memory in these domains.
    </p>
    <article
      v-for="item in data.items.filter((i) => filter === 'all' || i.scope === filter)"
      :key="item.id"
      class="space-y-2 rounded border border-border p-3"
    >
      <p class="text-xs text-muted-foreground">
        {{ item.scope }} · {{ item.kind }} · v{{ item.version }}
      </p>
      <p class="whitespace-pre-wrap wrap-break-word">{{ item.content }}</p>
      <Button size="sm" variant="outline" @click="inspect(item)">Inspect</Button>
    </article>
    <section
      v-if="selected"
      class="space-y-3 rounded border border-border p-3"
      aria-label="Memory inspector"
    >
      <p class="font-medium">
        {{ selected.kind }} · {{ selected.scope }} · v{{ selected.version }}
      </p>
      <p class="text-xs text-muted-foreground">
        Created {{ date(selected.createdAt) }} · Updated
        {{ date(selected.updatedAt) }}
      </p>
      <Button
        v-if="selected.sourceAvailable && selected.sourceConversation"
        size="sm"
        variant="outline"
        :disabled="busy"
        @click="
          run(
            () =>
              invoke('switch_channel', {
                channelId: selected!.sourceConversation,
              }),
            false,
          )
        "
      >
        Open source conversation
      </Button>
      <p v-else class="text-xs text-muted-foreground">Source conversation is unavailable.</p>
      <Button
        size="sm"
        variant="outline"
        :disabled="busy"
        @click="run(() => invoke('memory_reveal_item', { id: selected!.id }), false)"
      >
        Reveal item file
      </Button>
      <label class="block space-y-1">
        <span>Memory text</span>
        <textarea
          v-model="draft"
          class="block min-h-24 w-full rounded border border-input bg-background p-2"
        />
      </label>
      <Button size="sm" :disabled="busy || !draft.trim()" @click="mutate('edit')">Save edit</Button>
      <Field>
        <FieldLabel :for="targetId">Move to domain</FieldLabel>
        <Select v-model="target">
          <SelectTrigger :id="targetId" class="w-full">
            <SelectValue placeholder="Choose a domain" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem v-for="scope in data.scopes" :key="scope.scope" :value="scope.scope">
                {{ scope.title }}
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      </Field>
      <p class="text-xs text-muted-foreground">
        Global memory is shared with all agents. Project memory is shared within that project.
      </p>
      <Button
        size="sm"
        variant="outline"
        :disabled="busy || target === selected.scope"
        @click="mutate('move')"
      >
        Move
      </Button>
      <p class="text-xs text-muted-foreground">
        Forget removes saved memory from retrieval and pauses automatic capture. Transcript messages
        stay saved. Transcript deletion is separate.
      </p>
      <Button size="sm" variant="destructive" :disabled="busy" @click="mutate('forget')">
        Forget
      </Button>
      <details>
        <summary>Version history ({{ selected.history.length }})</summary>
        <article
          v-for="revision in selected.history"
          :key="revision.version"
          class="my-2 border-t border-border pt-2"
        >
          <p>
            v{{ revision.version }} · {{ revision.scope }} · {{ revision.origin }} ·
            {{ date(revision.updatedAt) }}{{ revision.deleted ? ' · Forgotten' : '' }}
          </p>
          <p class="whitespace-pre-wrap wrap-break-word">
            {{ revision.content }}
          </p>
        </article>
      </details>
    </section>
  </div>
</template>
