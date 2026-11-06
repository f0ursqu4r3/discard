<script setup lang="ts">
import { ref, watch } from 'vue'
import { createConversation, getAgents, type AgentEntry } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Input } from '../ui/input'
import { Button } from '../ui/button'
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{
  project: string
  projectId?: string | null
  channelId?: string
  agent?: string
  provider?: string
  hasGit?: boolean
}>()
const emit = defineEmits<{ created: [id: string] }>()
const name = ref(''),
  agentId = ref(''),
  error = ref(''),
  busy = ref(false),
  agents = ref<AgentEntry[]>([])
let context: string | null = null
watch(open, async (value) => {
  if (!value) return
  name.value = ''
  agentId.value = ''
  error.value = ''
  context = props.projectId ?? null
  try {
    agents.value = (await getAgents()).filter((a) => !a.archived)
  } catch (cause) {
    error.value = String(cause)
  }
})
async function submit() {
  if (busy.value || !name.value.trim() || !agentId.value) return
  busy.value = true
  error.value = ''
  try {
    const id = await createConversation(agentId.value, context, name.value.trim())
    open.value = false
    emit('created', id)
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog
    :open="open"
    @update:open="
      (value) => {
        if (!busy) open = value
      }
    "
  >
    <DialogContent
      data-ui-id="create-channel-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
    >
      <DialogHeader>
        <DialogTitle>New conversation</DialogTitle>
        <DialogDescription>{{ project }}. This conversation keeps this context.</DialogDescription>
      </DialogHeader>
      <form id="create-channel-form" class="space-y-3" @submit.prevent="submit">
        <label class="block" for="channel-name">Conversation name</label>
        <Input
          id="channel-name"
          v-model="name"
          data-ui-id="channel-name-input"
          :disabled="busy"
          required
        />
        <label class="block" for="conversation-agent">Agent</label>
        <select
          id="conversation-agent"
          v-model="agentId"
          :disabled="busy"
          required
          class="w-full rounded-md border bg-background p-2"
        >
          <option disabled value="">Choose an agent</option>
          <option v-for="agent in agents" :key="agent.id" :value="agent.id">
            {{ agent.name }} · {{ agent.provider }} · {{ agent.model }}
          </option>
        </select>
        <p v-if="!agents.length">
          Add an agent from Friends, or open an archived profile to restore it.
        </p>
        <p v-if="error" role="alert">{{ error }}</p>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          form="create-channel-form"
          data-ui-id="create-channel-submit"
          :disabled="busy || !name.trim() || !agentId"
        >
          {{ busy ? 'Creating…' : 'Create conversation' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
