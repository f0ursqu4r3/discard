<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getAgents, type AgentEntry } from '@/bridge/api'
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
  FieldSet,
  FieldLegend,
} from '@/components/ui/field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
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
const draft = ref({ ...props.agent }),
  providers = ref<{ id: string; setup: string }[]>([])
const review = ref<Review | null>(null),
  busy = ref(false),
  error = ref(''),
  confirm = ref(false),
  deleting = ref(false)
const deleteConversations = ref(false),
  deletePrivateMemory = ref(false)
let generation = 0,
  disposed = false
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
    !draft.value.model.trim()
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
      <Field>
        <FieldLabel for="agent-avatar">Avatar URL (optional)</FieldLabel>
        <Input
          id="agent-avatar"
          :model-value="draft.avatar ?? ''"
          :disabled="busy"
          @update:model-value="draft.avatar = String($event) || null"
        />
      </Field>
      <Field>
        <FieldLabel for="agent-provider">Provider</FieldLabel>
        <select
          id="agent-provider"
          v-model="draft.provider"
          :disabled="busy"
          required
          class="w-full rounded-md border border-input bg-background p-2 text-sm"
        >
          <option
            v-for="provider in providers"
            :key="provider.id"
            :value="provider.id"
            :disabled="provider.setup !== 'Ready' && provider.id !== props.agent.provider"
          >
            {{ provider.id }} · {{ provider.setup }}
          </option>
        </select>
      </Field>
      <Field>
        <FieldLabel for="agent-model">Model ID</FieldLabel>
        <Input id="agent-model" v-model="draft.model" :disabled="busy" required />
        <FieldDescription>
          Use the exact model ID. Changes apply to the next response.
        </FieldDescription>
      </Field>
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
          <FieldLabel for="agent-plugin-write">Allow plugin writes</FieldLabel>
          <Switch id="agent-plugin-write" v-model="draft.pluginWrite" :disabled="busy" />
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
          busy ||
          !draft.name.trim() ||
          !draft.model.trim() ||
          !!review?.blocked ||
          !review?.changes.length
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
