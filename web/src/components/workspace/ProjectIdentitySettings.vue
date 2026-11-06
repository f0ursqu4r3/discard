<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  getAgents,
  setProjectMainAgent,
  updateProjectProfile,
  type AgentEntry,
  type ProjectEntry,
} from '@/bridge/api'
import AgentIconPicker from '../agents/AgentIconPicker.vue'
import { Button } from '../ui/button'
import { Input } from '../ui/input'
import { Field, FieldLabel } from '../ui/field'
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card'

const props = defineProps<{ project: ProjectEntry; generalRunning?: boolean; busy?: boolean }>()
const emit = defineEmits<{ changed: [] }>()
const name = ref(props.project.name)
const avatar = ref(props.project.avatar ?? null)
const agents = ref<AgentEntry[]>([])
const loading = ref(true),
  saving = ref(false),
  changing = ref(false),
  error = ref('')
const dirty = computed(
  () => name.value.trim() !== props.project.name || avatar.value !== (props.project.avatar ?? null),
)
const disabled = computed(() => props.busy || saving.value || changing.value)
let disposed = false,
  stop: UnlistenFn | undefined,
  generation = 0
watch(
  () => props.project,
  (project, previous) => {
    if (
      project.id !== previous.id ||
      (name.value === previous.name && avatar.value === (previous.avatar ?? null))
    ) {
      name.value = project.name
      avatar.value = project.avatar ?? null
    }
  },
)
async function reloadAgents() {
  const request = ++generation
  try {
    const rows = await getAgents()
    if (!disposed && request === generation) agents.value = rows
  } catch (cause) {
    if (!disposed && request === generation) error.value = String(cause)
  } finally {
    if (!disposed && request === generation) loading.value = false
  }
}
onMounted(async () => {
  await reloadAgents()
  try {
    const unlisten = await listen('discard://agents', () => void reloadAgents())
    if (disposed) unlisten()
    else stop = unlisten
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  generation++
  stop?.()
})
async function save() {
  if (disabled.value || !name.value.trim() || !dirty.value) return
  saving.value = true
  error.value = ''
  try {
    await updateProjectProfile(props.project.id, name.value.trim(), avatar.value)
    if (!disposed) emit('changed')
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  } finally {
    saving.value = false
  }
}
async function changePrimary(event: Event) {
  const agentId = (event.target as HTMLSelectElement).value
  if (disabled.value || props.generalRunning || !agentId) return
  changing.value = true
  error.value = ''
  try {
    await setProjectMainAgent(props.project.id, agentId)
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  } finally {
    // A provider restart can fail after the new primary agent has been saved.
    if (!disposed) emit('changed')
    changing.value = false
  }
}
</script>
<template>
  <Card data-ui-id="project-identity-settings">
    <CardHeader><CardTitle>Identity</CardTitle></CardHeader>
    <CardContent class="flex flex-col gap-5">
      <form class="flex flex-col gap-5" @submit.prevent="save">
        <Field>
          <FieldLabel for="project-display-name">Name / Alias</FieldLabel>
          <Input
            id="project-display-name"
            v-model="name"
            required
            maxlength="100"
            :disabled="disabled"
          />
        </Field>
        <AgentIconPicker v-model="avatar" :name="name" entity="Project" :disabled="disabled" />
        <Button type="submit" class="self-start" :disabled="disabled || !dirty || !name.trim()">
          Save
        </Button>
      </form>
      <Field>
        <FieldLabel for="project-primary-agent">Primary / Moderator agent</FieldLabel>
        <select
          id="project-primary-agent"
          :value="project.mainAgentId ?? ''"
          :disabled="disabled || loading || generalRunning"
          class="h-9 rounded-md border border-input bg-background px-2 text-sm"
          @change="changePrimary"
        >
          <option value="" disabled>Select an agent</option>
          <option
            v-for="agent in agents"
            :key="agent.id"
            :value="agent.id"
            :disabled="agent.archived || !agent.model"
          >
            {{ agent.name }}{{ agent.archived ? ' · Archived' : '' }}
          </option>
        </select>
      </Field>
      <p v-if="generalRunning" class="text-xs text-muted-foreground">
        Wait for the general channel to finish before changing its agent.
      </p>
      <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
    </CardContent>
  </Card>
</template>
