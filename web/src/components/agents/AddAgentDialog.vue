<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { AgentEntry } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from '@/components/ui/select'
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{
  'update:open': [open: boolean]
  created: [id: string]
  connections: []
}>()
type Draft = Omit<AgentEntry, 'id' | 'archived'>
const draft = ref<Draft | null>(null)
const providers = ref<{ id: string; setup: string }[]>([])
const loading = ref(false),
  busy = ref(false),
  error = ref('')
type Model = { id: string; name: string; description: string }
const models = ref<Model[]>([]),
  modelsLoading = ref(false),
  modelsError = ref('')
let generation = 0,
  modelGeneration = 0
onUnmounted(() => {
  generation++
  modelGeneration++
})
async function loadModels() {
  const current = ++modelGeneration
  const provider = draft.value?.provider
  models.value = []
  modelsError.value = ''
  if (draft.value) draft.value.model = ''
  if (!props.open || !provider) {
    modelsLoading.value = false
    return
  }
  modelsLoading.value = true
  try {
    const result = await invoke<Model[]>('settings_provider_models', { provider })
    if (current === modelGeneration) models.value = result
  } catch (cause) {
    if (current === modelGeneration) modelsError.value = String(cause)
  } finally {
    if (current === modelGeneration) modelsLoading.value = false
  }
}
watch(() => [props.open, draft.value?.provider], loadModels, { flush: 'sync' })
const ready = computed(() => providers.value.filter((p) => p.setup === 'Ready'))
const canCreate = computed(
  () =>
    !!draft.value?.name.trim() &&
    !modelsLoading.value &&
    models.value.some((model) => model.id === draft.value?.model) &&
    ready.value.some((p) => p.id === draft.value?.provider),
)
watch(
  () => props.open,
  async (open) => {
    const current = ++generation
    if (!open) return
    draft.value = null
    error.value = ''
    loading.value = true
    try {
      const [defaults, connections] = await Promise.all([
        invoke<Draft>('settings_agent_defaults'),
        invoke<{ id: string; setup: string }[]>('settings_providers'),
      ])
      if (current !== generation) return
      providers.value = connections
      draft.value = {
        ...defaults,
        provider: connections.find((p) => p.setup === 'Ready')?.id ?? '',
      }
    } catch (cause) {
      if (current === generation) error.value = String(cause)
    } finally {
      if (current === generation) loading.value = false
    }
  },
  { immediate: true },
)
async function create() {
  if (!draft.value || !canCreate.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const id = await invoke<string>('settings_agent_create', { draft: { ...draft.value } })
    emit('created', id)
    emit('update:open', false)
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog :open="open" @update:open="!busy && emit('update:open', $event)">
    <DialogContent
      class="sm:max-w-md"
      @interact-outside="busy && $event.preventDefault()"
      @escape-key-down="busy && $event.preventDefault()"
    >
      <DialogHeader>
        <DialogTitle>Add agent</DialogTitle>
        <DialogDescription>
          Add someone to your agent list. You can personalize their profile later.
        </DialogDescription>
      </DialogHeader>
      <p v-if="loading" role="status">Loading connections…</p>
      <form v-else-if="draft" class="flex flex-col gap-5" @submit.prevent="create">
        <FieldGroup>
          <Field>
            <FieldLabel for="new-agent-name">Name</FieldLabel>
            <Input id="new-agent-name" v-model="draft.name" :disabled="busy" required />
          </Field>
          <Field>
            <FieldLabel for="new-agent-provider">Provider</FieldLabel>
            <Select v-model="draft.provider" :disabled="busy || !ready.length" required>
              <SelectTrigger id="new-agent-provider" class="w-full">
                <SelectValue placeholder="Choose a provider" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem v-for="provider in ready" :key="provider.id" :value="provider.id">
                    {{ provider.id }}
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel for="new-agent-model">Model</FieldLabel>
            <Select
              v-model="draft.model"
              :disabled="busy || modelsLoading || !models.length"
              required
            >
              <SelectTrigger id="new-agent-model" class="w-full" :aria-busy="modelsLoading">
                <SelectValue :placeholder="modelsLoading ? 'Loading models…' : 'Choose a model'" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem
                    v-for="model in models"
                    :key="model.id"
                    :value="model.id"
                    :title="model.description"
                  >
                    {{ model.name }}
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
            <FieldDescription v-if="draft.model">{{ draft.model }}</FieldDescription>
            <p v-if="modelsLoading" role="status" class="text-sm text-muted-foreground">
              Loading available models…
            </p>
            <div
              v-else-if="draft.provider && (modelsError || !models.length)"
              class="flex flex-col items-start gap-2"
            >
              <p :role="modelsError ? 'alert' : 'status'" class="text-sm text-muted-foreground">
                {{ modelsError || 'This provider returned no available models.' }}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                :disabled="busy"
                @click="loadModels"
              >
                Retry models
              </Button>
            </div>
          </Field>
        </FieldGroup>
        <div v-if="!ready.length" class="flex flex-col items-start gap-2">
          <p class="text-sm text-muted-foreground">Connect a provider before adding an agent.</p>
          <Button type="button" variant="outline" @click="emit('connections')">
            Connect a provider
          </Button>
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            :disabled="busy"
            @click="emit('update:open', false)"
          >
            Cancel
          </Button>
          <Button type="submit" :disabled="busy || !canCreate">
            {{ busy ? 'Adding…' : 'Add agent' }}
          </Button>
        </DialogFooter>
      </form>
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription>{{ error }}</AlertDescription>
      </Alert>
    </DialogContent>
  </Dialog>
</template>
