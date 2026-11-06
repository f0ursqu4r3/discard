<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Plus, Search, Ellipsis } from '@lucide/vue'
import type { MemoryItem, MemoryView } from './types'
import MemoryProposal from './MemoryProposal.vue'
import { Button } from '../ui/button'
import { Input } from '../ui/input'
import { Textarea } from '../ui/textarea'
import { Badge } from '../ui/badge'
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from '../ui/dropdown-menu'

const props = defineProps<{
  projectId: string
  data: MemoryView
  loading: boolean
  busy: boolean
  loadError: string
}>()
const emit = defineEmits<{
  inspect: [item: MemoryItem]
  changed: []
  reload: [importEdits: boolean]
}>()
const id = useId()
const query = ref('')
const scope = ref('all')
const agent = ref('all')
const kind = ref('all')
const adding = ref(false)
const draft = ref('')
const newKind = ref('fact')
const saving = ref(false)
const error = ref('')
const notice = ref('')
const itemId = ref(crypto.randomUUID())
const kinds = ['fact', 'decision', 'instruction', 'preference', 'commitment']
const agents = computed(() => props.data.scopes.filter((s) => s.agentId))
const projectItems = computed(() => props.data.items.filter((i) => i.scope !== 'global'))
const filtered = computed(() =>
  props.data.items
    .filter((item) => {
      const domain = props.data.scopes.find((s) => s.scope === item.scope)
      return (
        (scope.value === 'all' || item.scope.split(':')[0] === scope.value) &&
        (agent.value === 'all' || domain?.agentId === agent.value) &&
        (kind.value === 'all' || item.kind === kind.value) &&
        `${item.content} ${item.kind} ${domain?.title ?? ''} ${item.sourceAgentName ?? ''}`
          .toLocaleLowerCase()
          .includes(query.value.trim().toLocaleLowerCase())
      )
    })
    .sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id)),
)
const hasFilters = computed(
  () =>
    !!query.value.trim() || scope.value !== 'all' || agent.value !== 'all' || kind.value !== 'all',
)
const groups = computed(() => {
  const scopes = [...new Set(filtered.value.map((i) => i.scope))]
  const rank = (value: string) => (value.startsWith('project:') ? 0 : value === 'global' ? 2 : 1)
  return scopes
    .sort((a, b) => rank(a) - rank(b) || title(a).localeCompare(title(b)))
    .map((scope) => ({
      scope,
      title: title(scope),
      items: filtered.value.filter((i) => i.scope === scope),
      description:
        scope === 'global'
          ? 'Applies to all agents in all projects. Changes here affect every project.'
          : scope.startsWith('project:')
            ? 'Available to all agents in this project.'
            : 'Available only to this agent in this project. Sharing requires a scope change.',
    }))
})
function title(scope: string) {
  if (scope === 'global') return 'Inherited global memory'
  if (scope.startsWith('project:')) return 'Shared project memory'
  return props.data.scopes.find((s) => s.scope === scope)?.title ?? scope
}
function clearFilters() {
  query.value = ''
  scope.value = agent.value = kind.value = 'all'
}
function cancel() {
  adding.value = false
  draft.value = ''
  error.value = ''
  itemId.value = crypto.randomUUID()
}
async function save() {
  if (saving.value || !draft.value.trim()) return
  saving.value = true
  error.value = ''
  try {
    const result = await invoke<{ warning: string | null }>('memory_create', {
      projectId: props.projectId,
      itemId: itemId.value,
      kind: newKind.value,
      content: draft.value.trim(),
    })
    notice.value = result?.warning ?? 'Memory saved.'
    cancel()
    clearFilters()
    emit('changed')
  } catch (cause) {
    error.value = String(cause)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="space-y-6" data-ui-id="project-memory">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-lg font-semibold">Saved project memory</h2>
        <p class="mt-1 text-sm text-muted-foreground">
          Facts, decisions, and instructions from this project’s conversations.
        </p>
        <p v-if="!loading && !loadError" class="mt-2 text-xs text-muted-foreground">
          {{ projectItems.length }} project items ·
          {{ data.items.length - projectItems.length }} inherited global items
        </p>
      </div>
      <div class="flex items-center gap-2">
        <Button size="sm" :disabled="busy || saving || adding" @click="adding = true">
          <Plus data-icon="inline-start" />
          Add memory
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger as-child>
            <Button variant="outline" size="sm" aria-label="Memory options"><Ellipsis /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem :disabled="busy || loading || saving" @select="emit('reload', false)">
              Reload memory
            </DropdownMenuItem>
            <DropdownMenuItem :disabled="busy || loading || saving" @select="emit('reload', true)">
              Import file edits and reload
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>

    <form
      v-if="adding"
      class="space-y-3 rounded-lg border border-border bg-card p-4"
      @submit.prevent="save"
    >
      <h3 class="font-medium">Add shared project memory</h3>
      <p class="text-sm text-muted-foreground">All agents in this project can use this memory.</p>
      <label :for="`${id}-new-kind`" class="block text-sm">Memory kind</label>
      <Select v-model="newKind" :disabled="saving">
        <SelectTrigger :id="`${id}-new-kind`"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="value in kinds" :key="value" :value="value" class="capitalize">
            {{ value }}
          </SelectItem>
        </SelectContent>
      </Select>
      <Textarea
        v-model="draft"
        aria-label="New memory text"
        placeholder="What should agents remember?"
        :disabled="saving"
        class="min-h-28"
      />
      <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
      <div class="flex gap-2">
        <Button type="submit" size="sm" :disabled="saving || !draft.trim()">
          {{ saving ? 'Saving…' : 'Save memory' }}
        </Button>
        <Button type="button" variant="ghost" size="sm" :disabled="saving" @click="cancel">
          Cancel
        </Button>
      </div>
    </form>

    <p v-if="notice" role="status" class="text-sm">{{ notice }}</p>
    <section v-if="data.proposals.length" aria-label="Pending memory changes" class="space-y-3">
      <h3 class="font-medium">
        Pending changes
        <Badge variant="secondary">{{ data.proposals.length }}</Badge>
      </h3>
      <MemoryProposal
        v-for="proposal in data.proposals"
        :key="proposal.id"
        :proposal="proposal"
        :project-id="projectId"
        @changed="emit('changed')"
      />
    </section>

    <div class="space-y-3">
      <div class="relative">
        <Search class="pointer-events-none absolute top-2.5 left-3 size-4 text-muted-foreground" />
        <Input
          v-model="query"
          aria-label="Search memory"
          placeholder="Search memory"
          class="pl-9"
        />
      </div>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div class="space-y-1">
          <label :for="`${id}-scope`" class="text-xs text-muted-foreground">Scope</label>
          <Select v-model="scope">
            <SelectTrigger :id="`${id}-scope`" class="w-full"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All scopes</SelectItem>
              <SelectItem value="project">Shared project</SelectItem>
              <SelectItem value="agent-project">Agent in this project</SelectItem>
              <SelectItem value="global">Inherited global</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div class="space-y-1">
          <label :for="`${id}-agent`" class="text-xs text-muted-foreground">Agent</label>
          <Select v-model="agent">
            <SelectTrigger :id="`${id}-agent`" class="w-full"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All agents</SelectItem>
              <SelectItem v-for="entry in agents" :key="entry.agentId!" :value="entry.agentId!">
                {{ entry.title }}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div class="space-y-1">
          <label :for="`${id}-kind`" class="text-xs text-muted-foreground">Kind</label>
          <Select v-model="kind">
            <SelectTrigger :id="`${id}-kind`" class="w-full"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All kinds</SelectItem>
              <SelectItem v-for="value in kinds" :key="value" :value="value" class="capitalize">
                {{ value }}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <Button v-if="hasFilters" variant="ghost" size="sm" @click="clearFilters">
        Clear filters
      </Button>
    </div>

    <p v-if="loading" role="status" class="text-sm text-muted-foreground">Loading memory…</p>
    <template v-else-if="!loadError">
      <div
        v-if="hasFilters && !filtered.length"
        role="status"
        class="rounded-lg border border-dashed border-border p-6 text-center"
      >
        <p class="font-medium">No matching memory</p>
        <p class="mt-1 text-sm text-muted-foreground">Change the search or clear the filters.</p>
      </div>
      <div
        v-else-if="!hasFilters && !projectItems.length"
        class="rounded-lg border border-dashed border-border p-6"
      >
        <p class="font-medium">No project memory yet.</p>
        <p class="mt-2 text-sm text-muted-foreground">
          Save project facts, decisions, or instructions here. Agents can also propose memories
          during conversations.
        </p>
      </div>
      <section
        v-for="group in groups"
        :key="group.scope"
        :aria-label="group.title"
        class="space-y-3"
      >
        <div>
          <h3 class="flex items-center gap-2 font-medium">
            {{ group.title }}
            <Badge variant="secondary">{{ group.items.length }}</Badge>
          </h3>
          <p class="mt-1 text-xs text-muted-foreground">{{ group.description }}</p>
        </div>
        <article
          v-for="item in group.items"
          :key="item.id"
          class="rounded-lg border border-border bg-card p-4"
        >
          <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <Badge variant="outline" class="capitalize">{{ item.kind }}</Badge>
            <span>Updated {{ new Date(item.updatedAt).toLocaleString() }}</span>
            <span v-if="item.sourceAgentName">Source agent: {{ item.sourceAgentName }}</span>
          </div>
          <p class="mt-3 whitespace-pre-wrap wrap-break-word">{{ item.content }}</p>
          <Button
            size="sm"
            variant="ghost"
            class="mt-2"
            :disabled="busy"
            @click="emit('inspect', item)"
          >
            Inspect
          </Button>
        </article>
      </section>
    </template>
    <p class="text-xs text-muted-foreground">
      Current instructions take priority. Agents select applicable memory before each response.
    </p>
  </div>
</template>
