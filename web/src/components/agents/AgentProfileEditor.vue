<script setup lang="ts">
import { hashedSlot } from './agent-icon'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from '@/components/ui/select'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getAgents, type AgentEntry } from '@/bridge/api'
import {
  Field,
  FieldContent,
  FieldGroup,
  FieldLabel,
  FieldDescription,
  FieldSet,
  FieldLegend,
} from '@/components/ui/field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import AgentIconPicker from './AgentIconPicker.vue'
import ReasoningSelect from './ReasoningSelect.vue'
import type { ProviderModel } from '@/bridge/api'
import { Textarea } from '@/components/ui/textarea'
import { Switch } from '@/components/ui/switch'
import { Alert, AlertDescription } from '@/components/ui/alert'
const props = defineProps<{ agent: AgentEntry }>()
const emit = defineEmits<{
  saved: [agent: AgentEntry]
  deleted: []
  cancel: []
  busy: [value: boolean]
}>()
type Review = {
  changes: { field: string; before: string; after: string }[]
  choices: { policy: string; enabled: boolean; reason: string | null }[]
  blocked: string | null
  needsConfirm: boolean
}
const draft = ref({ ...props.agent, reasoningEffort: props.agent.reasoningEffort ?? null }),
  providers = ref<{ id: string; setup: string }[]>([])
const review = ref<Review | null>(null),
  busy = ref(false),
  error = ref(''),
  confirm = ref(false),
  deleting = ref(false)
const deleteConversations = ref(false),
  deletePrivateMemory = ref(false)
let generation = 0,
  modelGeneration = 0,
  disposed = false
type Model = ProviderModel
const models = ref<Model[]>([]),
  modelsLoading = ref(false),
  modelsError = ref('')
const availableModels = computed(() => {
  if (
    draft.value.provider === props.agent.provider &&
    props.agent.model &&
    !models.value.some((model) => model.id === props.agent.model)
  ) {
    return [
      { id: props.agent.model, name: props.agent.model, description: 'Saved model' },
      ...models.value,
    ]
  }
  return models.value
})
const validModel = computed(() =>
  availableModels.value.some((model) => model.id === draft.value.model),
)
const reasoningModel = computed(() => models.value.find((model) => model.id === draft.value.model))
watch(
  () => [draft.value.provider, draft.value.model],
  () => {
    draft.value.reasoningEffort = null
  },
  { flush: 'sync' },
)
async function loadModels() {
  const current = ++modelGeneration
  const provider = draft.value.provider
  models.value = []
  modelsError.value = ''
  modelsLoading.value = true
  try {
    const result = await invoke<Model[]>('settings_provider_models', { provider })
    if (!disposed && current === modelGeneration) models.value = result
  } catch (cause) {
    if (!disposed && current === modelGeneration) modelsError.value = String(cause)
  } finally {
    if (!disposed && current === modelGeneration) modelsLoading.value = false
  }
}
watch(
  () => draft.value.provider,
  (_provider, previous) => {
    if (previous !== undefined) draft.value.model = ''
    void loadModels()
  },
  { immediate: true, flush: 'sync' },
)
watch(busy, (value) => emit('busy', value), { flush: 'sync' })
watch(
  draft,
  async (value) => {
    const current = ++generation
    review.value = null
    confirm.value = false
    try {
      const result = await invoke<Review>('settings_agent_review', {
        id: props.agent.id,
        draft: { ...value },
      })
      if (!disposed && current === generation) review.value = result
    } catch (cause) {
      if (!disposed && current === generation) error.value = String(cause)
    }
  },
  { deep: true, immediate: true },
)
onMounted(async () => {
  try {
    providers.value = await invoke('settings_providers')
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  generation++
  modelGeneration++
  emit('busy', false)
})
async function currentAgent() {
  const rows = await getAgents()
  const agent = rows.find((a) => a.id === props.agent.id)
  if (!agent) throw new Error('This agent is no longer available.')
  emit('saved', agent)
}
async function save(confirmed = false) {
  if (
    busy.value ||
    !review.value ||
    review.value.blocked ||
    !draft.value.name.trim() ||
    !validModel.value
  )
    return
  busy.value = true
  error.value = ''
  try {
    await invoke('settings_agent_save', {
      id: props.agent.id,
      draft: { ...draft.value },
      confirmed,
    })
    await currentAgent()
  } catch (cause) {
    if (String(cause) === 'Confirmation is required') confirm.value = true
    else error.value = String(cause)
  } finally {
    busy.value = false
  }
}
async function archive() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await invoke('settings_agent_archive', { id: props.agent.id, archived: !props.agent.archived })
    await currentAgent()
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
async function remove() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await invoke('settings_agent_delete', {
      id: props.agent.id,
      deleteConversations: deleteConversations.value,
      deletePrivateMemory: deletePrivateMemory.value,
    })
    emit('deleted')
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <form class="flex flex-col gap-5" @submit.prevent="save(false)">
    <FieldGroup>
      <Field>
        <FieldLabel for="agent-name">Name</FieldLabel>
        <Input id="agent-name" v-model="draft.name" :disabled="busy" required />
      </Field>
      <AgentIconPicker
        :fallback-color="`ansi-${hashedSlot(agent.id)}`"
        v-model="draft.avatar"
        :name="draft.name"
        :disabled="busy"
      />
      <Field>
        <FieldLabel for="agent-provider">Provider</FieldLabel>
        <Select v-model="draft.provider" :disabled="busy" required>
          <SelectTrigger id="agent-provider" class="w-full">
            <SelectValue placeholder="Choose a provider" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem
                v-for="provider in providers"
                :key="provider.id"
                :value="provider.id"
                :disabled="provider.setup !== 'Ready' && provider.id !== props.agent.provider"
              >
                {{ provider.id }} · {{ provider.setup }}
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      </Field>
      <Field>
        <FieldLabel for="agent-model">Model ID</FieldLabel>
        <Select
          v-model="draft.model"
          :disabled="busy || modelsLoading || !availableModels.length"
          required
        >
          <SelectTrigger id="agent-model" class="w-full" :aria-busy="modelsLoading">
            <SelectValue :placeholder="modelsLoading ? 'Loading models…' : 'Choose a model'" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem
                v-for="model in availableModels"
                :key="model.id"
                :value="model.id"
                :title="model.description"
              >
                {{ model.name }}
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        <FieldDescription>
          <span v-if="draft.model">{{ draft.model }}.</span>
          Changes apply to the next response.
        </FieldDescription>
        <p v-if="modelsLoading" role="status" class="text-sm text-muted-foreground">
          Loading available models…
        </p>
        <div v-else-if="modelsError || !models.length" class="flex flex-col items-start gap-2">
          <p :role="modelsError ? 'alert' : 'status'" class="text-sm text-muted-foreground">
            {{ modelsError || 'This provider returned no available models.' }}
          </p>
          <Button type="button" variant="outline" size="sm" :disabled="busy" @click="loadModels">
            Retry models
          </Button>
        </div>
      </Field>
      <ReasoningSelect
        v-model="draft.reasoningEffort"
        :model="reasoningModel"
        :loading="modelsLoading"
        :disabled="busy || !draft.model"
      />
      <Field>
        <FieldLabel for="agent-prompt">Personality (optional)</FieldLabel>
        <Textarea
          id="agent-prompt"
          v-model="draft.systemPrompt"
          :disabled="busy"
          placeholder="How should this agent communicate and work?"
        />
      </Field>
    </FieldGroup>
    <details>
      <summary class="cursor-pointer text-sm font-medium">Tool permissions</summary>
      <FieldGroup class="pt-4">
        <FieldSet>
          <FieldLegend>Approval policy</FieldLegend>
          <label
            v-for="choice in review?.choices"
            :key="choice.policy"
            class="flex items-start gap-2 text-sm"
          >
            <input
              v-model="draft.policy"
              type="radio"
              :value="choice.policy"
              :disabled="busy || !choice.enabled"
            />
            <span>
              {{ choice.policy === 'ask' ? 'Ask before tool calls' : 'Deny all' }}
              <span v-if="choice.reason" class="block text-muted-foreground">
                {{ choice.reason }}
              </span>
            </span>
          </label>
        </FieldSet>
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel for="agent-workspace-write">Edit files in branch channels</FieldLabel>
            <FieldDescription id="agent-workspace-write-description">
              Lets this agent edit files and run approved commands in branch and worktree channels.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="agent-workspace-write"
            v-model="draft.workspaceWrite"
            :disabled="busy"
            aria-describedby="agent-workspace-write-description"
          />
        </Field>
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel for="agent-plugin-write">Allow plugin writes</FieldLabel>
            <FieldDescription id="agent-plugin-write-description">
              Works only in a builder session. Start Discard with
              <code class="font-mono">discard --builder</code>
              . Normal chats cannot write plugins.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="agent-plugin-write"
            v-model="draft.pluginWrite"
            :disabled="busy"
            aria-describedby="agent-plugin-write-description"
          />
        </Field>
      </FieldGroup>
    </details>
    <Alert v-if="review?.blocked" role="alert">
      <AlertDescription>{{ review.blocked }}</AlertDescription>
    </Alert>
    <Alert v-if="error" variant="destructive" role="alert">
      <AlertDescription>{{ error }}</AlertDescription>
    </Alert>
    <Alert v-if="confirm" data-protected="true">
      <AlertDescription class="flex flex-col items-start gap-3">
        <p>This lets the agent do more without asking.</p>
        <Button type="button" variant="destructive" :disabled="busy" @click="save(true)">
          Confirm and save
        </Button>
      </AlertDescription>
    </Alert>
    <div class="flex justify-end gap-2">
      <Button type="button" variant="outline" :disabled="busy" @click="emit('cancel')">
        Cancel
      </Button>
      <Button
        type="submit"
        :disabled="
          busy || !draft.name.trim() || !validModel || !!review?.blocked || !review?.changes.length
        "
      >
        Save profile
      </Button>
    </div>
    <FieldSet class="border-t border-border pt-4">
      <FieldLegend>Manage agent</FieldLegend>
      <div class="flex flex-wrap gap-2">
        <Button type="button" variant="outline" :disabled="busy" @click="archive">
          {{ agent.archived ? 'Restore agent' : 'Archive agent' }}
        </Button>
        <Button type="button" variant="destructive" :disabled="busy" @click="deleting = true">
          Delete agent
        </Button>
      </div>
      <Alert v-if="deleting" data-protected="true">
        <AlertDescription class="flex flex-col gap-3">
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
          <div class="flex gap-2">
            <Button type="button" variant="destructive" :disabled="busy" @click="remove">
              Delete permanently
            </Button>
            <Button type="button" variant="outline" :disabled="busy" @click="deleting = false">
              Cancel deletion
            </Button>
          </div>
        </AlertDescription>
      </Alert>
    </FieldSet>
  </form>
</template>
