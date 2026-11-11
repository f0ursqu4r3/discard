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
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Sparkles } from '@lucide/vue'
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
import { Badge } from '@/components/ui/badge'
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
  personalityGeneration = 0,
  disposed = false
const personalityOpen = ref(false),
  personalityDescription = ref(''),
  personalityRunning = ref(false),
  personalityError = ref(''),
  previousPersonality = ref<string | null>(null)
const personalityField = ref<HTMLElement | null>(null)
async function openPersonality() {
  personalityOpen.value = true
  personalityError.value = ''
  await nextTick()
  personalityField.value?.querySelector<HTMLInputElement>('#agent-personality-description')?.focus()
}
function cancelPersonality() {
  personalityGeneration++
  personalityRunning.value = false
  personalityOpen.value = false
  personalityError.value = ''
}
async function generatePersonality() {
  if (
    busy.value ||
    personalityRunning.value ||
    !validModel.value ||
    !personalityDescription.value.trim()
  )
    return
  const current = ++personalityGeneration
  const { provider, model, reasoningEffort, name, systemPrompt } = draft.value
  personalityRunning.value = true
  personalityError.value = ''
  try {
    const result = await invoke<string>('settings_agent_generate_personality', {
      provider,
      model,
      reasoningEffort,
      name,
      description: personalityDescription.value,
      current: systemPrompt,
    })
    if (disposed || current !== personalityGeneration) return
    previousPersonality.value = systemPrompt
    draft.value.systemPrompt = result
    personalityOpen.value = false
    personalityDescription.value = ''
    personalityRunning.value = false
    await nextTick()
    if (!disposed && current === personalityGeneration) {
      personalityField.value?.querySelector<HTMLTextAreaElement>('#agent-prompt')?.focus()
    }
  } catch (cause) {
    if (!disposed && current === personalityGeneration) personalityError.value = String(cause)
  } finally {
    if (!disposed && current === personalityGeneration) personalityRunning.value = false
  }
}
function undoPersonality() {
  if (previousPersonality.value === null) return
  draft.value.systemPrompt = previousPersonality.value
  previousPersonality.value = null
}
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
    personalityGeneration++
    personalityRunning.value = false
    personalityError.value = ''
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
const autorunUnavailable = computed(() => props.agent.sandboxAutorunUnavailable ?? null)
const autorunPlanned = computed(
  () => !!autorunUnavailable.value && props.agent.sandboxAutorunPlanned,
)
const autorunOn = computed(
  () => draft.value.workspaceWrite && !autorunUnavailable.value && !!draft.value.sandboxAutorun,
)
const enablesAutorun = computed(
  () => autorunOn.value && !(props.agent.workspaceWrite && props.agent.sandboxAutorun),
)
// While file edits are off, the backend clears autorun on save. Do not send the stale value.
function draftPayload() {
  const { sandboxAutorun, ...rest } = draft.value
  return draft.value.workspaceWrite ? { ...rest, sandboxAutorun } : rest
}
watch(
  draft,
  async () => {
    const current = ++generation
    review.value = null
    confirm.value = false
    try {
      const result = await invoke<Review>('settings_agent_review', {
        id: props.agent.id,
        draft: draftPayload(),
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
  personalityGeneration++
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
      draft: draftPayload(),
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
        <div ref="personalityField" class="flex flex-col gap-2">
          <Textarea
            id="agent-prompt"
            v-model="draft.systemPrompt"
            :disabled="busy || personalityRunning"
            @update:model-value="previousPersonality = null"
            placeholder="How should this agent communicate and work?"
          />
          <div class="flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              :disabled="busy || personalityRunning || !validModel"
              @click="openPersonality"
            >
              <Sparkles aria-hidden="true" />
              Generate
            </Button>
            <Button
              v-if="previousPersonality !== null"
              type="button"
              variant="ghost"
              size="sm"
              :disabled="busy || personalityRunning"
              @click="undoPersonality"
            >
              Undo
            </Button>
          </div>
          <FieldDescription v-if="!validModel">
            Choose a model to generate a personality.
          </FieldDescription>
          <div v-if="personalityOpen" class="flex flex-col gap-2">
            <FieldLabel for="agent-personality-description" class="sr-only">
              Describe the personality
            </FieldLabel>
            <div class="flex flex-wrap gap-2">
              <Input
                id="agent-personality-description"
                v-model="personalityDescription"
                class="min-w-48 flex-1"
                placeholder="For example: terse senior reviewer who asks before large changes"
                maxlength="2000"
                @keydown.enter.stop.prevent="generatePersonality"
                @keydown.esc.stop.prevent="cancelPersonality"
              />
              <Button
                type="button"
                :disabled="
                  busy || !validModel || !personalityDescription.trim() || personalityRunning
                "
                :aria-busy="personalityRunning"
                @click="generatePersonality"
              >
                {{ personalityRunning ? 'Generating…' : 'Generate' }}
              </Button>
              <Button type="button" variant="ghost" @click="cancelPersonality">Cancel</Button>
            </div>
          </div>
          <p v-if="personalityError" role="alert" class="text-sm text-destructive">
            {{ personalityError }}
          </p>
        </div>
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
            <FieldLabel for="agent-sandbox-autorun">
              Run sandboxed commands without asking
              <!-- TODO: Enable when the provider passes the autorun sandbox check. A missing provider is a state block, not planned work. -->
              <Badge v-if="autorunPlanned" variant="secondary">Soon</Badge>
            </FieldLabel>
            <FieldDescription id="agent-sandbox-autorun-description">
              Shell commands in the worktree run without approval. The sandbox blocks writes outside
              the worktree and network access. It does not block reads. It cannot block Keychain
              access.
              <span v-if="autorunUnavailable" class="block">{{ autorunUnavailable }}</span>
              <span v-else-if="!draft.workspaceWrite" class="block">Turn on file edits first.</span>
            </FieldDescription>
          </FieldContent>
          <Switch
            id="agent-sandbox-autorun"
            :model-value="autorunOn"
            :disabled="busy || !draft.workspaceWrite || !!autorunUnavailable"
            aria-describedby="agent-sandbox-autorun-description"
            @update:model-value="draft.sandboxAutorun = $event === true"
          />
        </Field>
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel for="agent-delegation">Delegation</FieldLabel>
            <FieldDescription id="agent-delegation-description">
              This agent can start read-only sub-agents in threads.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="agent-delegation"
            :model-value="!!draft.delegation"
            :disabled="busy"
            aria-describedby="agent-delegation-description"
            @update:model-value="draft.delegation = $event === true"
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
        <p v-if="enablesAutorun">
          Autorun commands can read your macOS Keychain. Discard cannot block this. Do not turn on
          autorun if your Keychain holds tokens this agent must not see.
        </p>
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
