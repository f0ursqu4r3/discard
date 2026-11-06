<script setup lang="ts">
defineOptions({ name: 'SettingsAgents' })
import { nextTick, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getAgents, type AgentEntry } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Switch } from '@/components/ui/switch'
type Review = {
  changes: { field: string; before: string; after: string }[]
  choices: { policy: string; enabled: boolean; reason: string | null }[]
  blocked: string | null
  needsConfirm: boolean
}
type Provider = { id: string; setup: string }
const agents = ref<AgentEntry[]>([])
const providers = ref<Provider[]>([])
const selected = ref<AgentEntry | null>(null)
const creating = ref(false),
  busy = ref(false),
  error = ref(''),
  saved = ref(false)
const review = ref<Review | null>(null),
  confirm = ref(false),
  deleting = ref(false)
const deleteConversations = ref(false),
  deletePrivateMemory = ref(false)
let generation = 0
async function reload() {
  ;[agents.value, providers.value] = await Promise.all([
    getAgents(),
    invoke<Provider[]>('settings_providers'),
  ])
}
function select(agent: AgentEntry) {
  generation++
  selected.value = { ...agent }
  creating.value = false
  deleting.value = false
  review.value = null
  confirm.value = false
  saved.value = false
  error.value = ''
}
async function create() {
  try {
    await reload()
    const draft = await invoke<Omit<AgentEntry, 'id' | 'archived'>>('settings_agent_defaults')
    creating.value = true
    selected.value = {
      ...draft,
      id: '',
      archived: false,
      provider: providers.value.find((p) => p.setup === 'Ready')?.id ?? '',
    }
    review.value = null
    deleting.value = false
    error.value = ''
  } catch (cause) {
    error.value = String(cause)
  }
}
watch(
  selected,
  async (agent) => {
    const current = ++generation
    confirm.value = false
    saved.value = false
    review.value = null
    if (!agent || creating.value) return
    try {
      const result = await invoke<Review>('settings_agent_review', { id: agent.id, draft: agent })
      if (current === generation) review.value = result
    } catch (cause) {
      if (current === generation) error.value = String(cause)
    }
  },
  { deep: true },
)
async function save(confirmed = false) {
  const draft = selected.value
  if (!draft || busy.value) return
  busy.value = true
  error.value = ''
  try {
    if (creating.value) {
      const id = await invoke<string>('settings_agent_create', { draft })
      await reload()
      const agent = agents.value.find((a) => a.id === id)
      if (agent) select(agent)
    } else {
      await invoke('settings_agent_save', { id: draft.id, draft, confirmed })
      await reload()
      const agent = agents.value.find((a) => a.id === draft.id)
      if (agent) select(agent)
    }
    await nextTick()
    review.value = null
    confirm.value = false
    saved.value = true
  } catch (cause) {
    if (String(cause) === 'Confirmation is required') confirm.value = true
    else error.value = String(cause)
  } finally {
    busy.value = false
  }
}
async function archive() {
  const agent = selected.value
  if (!agent || busy.value) return
  busy.value = true
  try {
    await invoke('settings_agent_archive', { id: agent.id, archived: !agent.archived })
    await reload()
    const updated = agents.value.find((a) => a.id === agent.id)
    if (updated) select(updated)
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
async function remove() {
  if (!selected.value || busy.value) return
  busy.value = true
  try {
    await invoke('settings_agent_delete', {
      id: selected.value.id,
      deleteConversations: deleteConversations.value,
      deletePrivateMemory: deletePrivateMemory.value,
    })
    selected.value = null
    deleting.value = false
    await reload()
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
onMounted(() => {
  void reload().catch((cause) => {
    error.value = String(cause)
  })
})
</script>
<template>
  <div class="space-y-4">
    <div class="flex flex-wrap gap-2">
      <Button :disabled="busy" @click="create">Create agent</Button>
      <Button
        v-for="agent in agents"
        :key="agent.id"
        variant="outline"
        :disabled="busy"
        @click="select(agent)"
      >
        {{ agent.name }} · {{ agent.provider }}
        <span v-if="agent.archived">· Archived</span>
      </Button>
    </div>
    <p v-if="!agents.length && !selected">
      Create an agent to start a direct chat. Connect a provider in CLI connections first.
    </p>
    <form v-if="selected" class="space-y-3" @submit.prevent="save(false)">
      <label class="block" for="agent-name">Name</label>
      <Input id="agent-name" v-model="selected.name" :disabled="busy" required />
      <label class="block" for="agent-avatar">Avatar URL (optional)</label>
      <Input
        id="agent-avatar"
        :model-value="selected.avatar ?? ''"
        :disabled="busy"
        @update:model-value="selected.avatar = String($event) || null"
      />
      <label class="block" for="agent-provider">Provider</label>
      <select
        id="agent-provider"
        v-model="selected.provider"
        :disabled="busy"
        required
        class="w-full rounded-md border bg-background p-2"
      >
        <option disabled value="">Choose a connected provider</option>
        <option
          v-for="provider in providers"
          :key="provider.id"
          :value="provider.id"
          :disabled="provider.setup !== 'Ready' && provider.id !== selected.provider"
        >
          {{ provider.id }} · {{ provider.setup }}
        </option>
      </select>
      <label class="block" for="agent-model">Model ID</label>
      <Input
        id="agent-model"
        v-model="selected.model"
        list="agent-models"
        :disabled="busy"
        required
      />
      <datalist id="agent-models">
        <option
          v-for="model in [
            ...new Set(
              agents
                .filter((a) => a.provider === selected?.provider)
                .map((a) => a.model)
                .filter(Boolean),
            ),
          ]"
          :key="model"
          :value="model"
        />
      </datalist>
      <p class="text-xs text-muted-foreground">
        Enter the exact model ID supported by this provider.
      </p>
      <label class="block" for="agent-prompt">Personality (optional)</label>
      <Textarea id="agent-prompt" v-model="selected.systemPrompt" :disabled="busy" />
      <p>Approval policy</p>
      <div class="flex gap-2">
        <template v-if="!creating">
          <div v-for="choice in review?.choices" :key="choice.policy">
            <Button
              type="button"
              :disabled="busy || !choice.enabled"
              :variant="selected.policy === choice.policy ? 'default' : 'outline'"
              @click="selected.policy = choice.policy"
            >
              {{ choice.policy === 'ask' ? 'Ask before tool calls' : 'Deny all' }}
            </Button>
            <p v-if="choice.reason">{{ choice.reason }}</p>
          </div>
        </template>
        <p v-else>Deny all. You can review tool permissions after creation.</p>
      </div>
      <div v-if="!creating" class="flex items-center gap-2">
        <span>plugin.write</span>
        <Switch aria-label="plugin.write" v-model="selected.pluginWrite" :disabled="busy" />
      </div>
      <p v-for="change in review?.changes" :key="change.field" class="whitespace-pre-wrap">
        {{ change.field }}: {{ change.before }} → {{ change.after }}
      </p>
      <p v-if="review?.blocked">{{ review.blocked }}</p>
      <Button
        type="submit"
        :disabled="
          busy ||
          !selected.name.trim() ||
          !selected.provider ||
          !selected.model.trim() ||
          !!review?.blocked ||
          (!creating && !review?.changes.length)
        "
      >
        Save
      </Button>
      <div
        v-if="confirm"
        data-protected="true"
        class="space-y-2 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
      >
        <p>This lets the agent do more without asking.</p>
        <Button type="button" variant="destructive" :disabled="busy" @click="save(true)">
          Confirm and save
        </Button>
      </div>
      <p v-if="saved" role="status">Saved. Changes apply to the next response.</p>
      <div v-if="!creating" class="flex gap-2 border-t pt-4">
        <Button type="button" variant="outline" :disabled="busy" @click="archive">
          {{ selected.archived ? 'Restore agent' : 'Archive agent' }}
        </Button>
        <Button type="button" variant="destructive" :disabled="busy" @click="deleting = true">
          Delete agent
        </Button>
      </div>
      <div
        v-if="deleting"
        data-protected="true"
        class="space-y-3 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
      >
        <p>
          Delete this agent permanently. Kept conversations remain readable. Shared global and
          project memory stays saved.
        </p>
        <label class="flex gap-2">
          <input v-model="deleteConversations" type="checkbox" :disabled="busy" />
          Delete conversations
        </label>
        <label class="flex gap-2">
          <input v-model="deletePrivateMemory" type="checkbox" :disabled="busy" />
          Delete private memory
        </label>
        <Button type="button" variant="destructive" :disabled="busy" @click="remove">
          Delete permanently
        </Button>
        <Button type="button" variant="outline" :disabled="busy" @click="deleting = false">
          Cancel
        </Button>
      </div>
    </form>
    <p
      v-if="error"
      role="alert"
      data-protected="true"
      class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
    >
      {{ error }}
    </p>
  </div>
</template>
