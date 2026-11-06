<script setup lang="ts">
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from '@/components/ui/select'
import { computed, nextTick, ref, useId, watch } from 'vue'
import { Field, FieldLabel } from '../ui/field'
import { invoke } from '@tauri-apps/api/core'
import { useMemory } from './useMemory'
import type { MemoryItem } from './types'
import MemoryProposalCard from './MemoryProposal.vue'
import ProjectMemoryContent from './ProjectMemoryContent.vue'
import MemoryCaptureStatus from './MemoryCaptureStatus.vue'
import { Button } from '../ui/button'
const props = defineProps<{
  mode?: 'project'
  channelId?: string
  agentId?: string
  projectId?: string | null
  revision?: number
  running?: boolean
}>()
const projectMode = computed(() => props.mode === 'project')
const projectContext = computed(() => (projectMode.value ? (props.projectId ?? '') : null))
const {
  data,
  error: loadError,
  loading,
  reload,
} = useMemory(
  () => (projectMode.value ? null : (props.channelId ?? null)),
  () =>
    projectMode.value ? !!props.projectId : props.channelId === undefined || !!props.channelId,
  projectContext,
)
const selected = ref<MemoryItem | null>(null)
const inspector = ref<HTMLElement | null>(null)
const draft = ref('')
const target = ref('')
const filter = ref('all')
const filterId = useId()
const targetId = useId()
const error = ref('')
const busy = ref(false)
let generation = 0
function inspect(item: MemoryItem) {
  selected.value = item
  draft.value = item.content
  target.value = item.scope
  error.value = ''
  if (projectMode.value) {
    void nextTick(() => {
      if (selected.value?.id !== item.id) return
      inspector.value?.focus({ preventScroll: true })
      inspector.value?.scrollIntoView?.({ block: 'start' })
    })
  }
}
function date(value: number) {
  return new Date(value).toLocaleString()
}
async function run(action: () => Promise<unknown>, refresh = true) {
  const current = generation
  busy.value = true
  error.value = ''
  try {
    await action()
    if (current !== generation) return false
    if (refresh) await reload()
    return current === generation
  } catch (cause) {
    if (current === generation) error.value = String(cause)
    return false
  } finally {
    if (current === generation) busy.value = false
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
        channelId: projectMode.value ? null : (props.channelId ?? null),
        ...(projectMode.value ? { projectId: props.projectId } : {}),
      }),
    )
  ) {
    const current = data.value.items.find((i) => i.id === item.id)
    if (current) inspect(current)
    else selected.value = null
  }
}
async function refresh(importEdits = false) {
  const ticket = generation
  if (importEdits && !(await run(() => invoke('memory_reconcile')))) return
  if (!importEdits) await reload()
  if (ticket !== generation) return
  const current = data.value.items.find((i) => i.id === selected.value?.id)
  if (current) inspect(current)
  else selected.value = null
}
watch([() => props.channelId, projectContext], () => {
  ++generation
  busy.value = false
  selected.value = null
  error.value = ''
  filter.value = 'all'
})
watch([() => props.revision, () => props.running], () => {
  void reload()
})
</script>
<template>
  <div class="space-y-4 p-3 text-sm" data-ui-id="memory-browser">
    <ProjectMemoryContent
      v-if="projectMode && projectId"
      :key="projectId"
      :project-id="projectId"
      :data="data"
      :loading="loading"
      :busy="busy"
      :load-error="loadError"
      @inspect="inspect"
      @changed="reload"
      @reload="refresh"
    />
    <template v-if="!projectMode">
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
    </template>
    <p v-if="error || loadError" role="alert" class="text-destructive">{{ error || loadError }}</p>
    <p v-for="warning in data.warnings ?? []" :key="warning" role="status">{{ warning }}</p>
    <section
      v-if="selected"
      class="space-y-3 rounded border border-border p-3"
      ref="inspector"
      tabindex="-1"
      aria-label="Memory inspector"
    >
      <div v-if="projectMode" class="flex items-center justify-between gap-3">
        <h3 class="font-medium">Memory details</h3>
        <Button size="sm" variant="ghost" @click="selected = null">Close details</Button>
      </div>
      <p v-if="projectMode" class="text-sm text-muted-foreground">
        {{ data.scopes.find((scope) => scope.scope === selected!.scope)?.title }}
      </p>
      <p v-if="selected.scope === 'global'" class="text-sm text-muted-foreground">
        Changes to this item affect all agents in all projects.
      </p>
      <p v-if="selected.sourceAgentName" class="text-xs text-muted-foreground">
        Source agent: {{ selected.sourceAgentName }}
      </p>
      <p v-if="selected.origin" class="text-xs text-muted-foreground">
        Last change: {{ selected.origin }}
      </p>
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
      <p v-else class="text-xs text-muted-foreground">
        {{
          selected.sourceConversation
            ? 'Source conversation is unavailable.'
            : 'No source conversation was recorded.'
        }}
      </p>
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
