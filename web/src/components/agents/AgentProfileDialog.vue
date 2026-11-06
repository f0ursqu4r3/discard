<script setup lang="ts">
import { ref, watch } from 'vue'
import { getAgents, type AgentEntry } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import AgentAvatar from '@/components/agents/AgentAvatar.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import AgentProfileEditor from './AgentProfileEditor.vue'
const props = defineProps<{ agentId: string | null }>()
const emit = defineEmits<{ close: []; message: [id: string]; busy: [value: boolean] }>()
const agent = ref<AgentEntry | null>(null),
  editing = ref(false),
  busy = ref(false),
  loading = ref(false),
  error = ref(''),
  saved = ref(false)
let generation = 0
watch(busy, (value) => emit('busy', value), { flush: 'sync' })
watch(
  () => props.agentId,
  async (id) => {
    const current = ++generation
    agent.value = null
    editing.value = false
    error.value = ''
    saved.value = false
    if (!id) return
    loading.value = true
    try {
      const rows = await getAgents()
      if (current !== generation) return
      agent.value = rows.find((a) => a.id === id) ?? null
      if (!agent.value)
        error.value = 'This agent is no longer available. Kept conversations remain readable.'
    } catch (cause) {
      if (current === generation) error.value = String(cause)
    } finally {
      if (current === generation) loading.value = false
    }
  },
  { immediate: true },
)
function updated(value: AgentEntry) {
  agent.value = value
  editing.value = false
  saved.value = true
}
function edit() {
  editing.value = true
  saved.value = false
}
</script>
<template>
  <Dialog :open="!!agentId" @update:open="!$event && !busy && emit('close')">
    <!-- Mount the portal on open so the profile and its overlay sit above an existing sheet. -->
    <DialogContent
      v-if="agentId"
      class="max-h-[85dvh] overflow-y-auto sm:max-w-lg"
      @interact-outside="busy && $event.preventDefault()"
      @escape-key-down="busy && $event.preventDefault()"
    >
      <DialogHeader>
        <DialogTitle>{{ editing ? 'Edit profile' : 'Agent profile' }}</DialogTitle>
        <DialogDescription>
          {{
            editing
              ? 'Changes apply to the next response.'
              : 'Get to know your agent and start a conversation.'
          }}
        </DialogDescription>
      </DialogHeader>
      <p v-if="loading" role="status">Loading profile…</p>
      <Alert v-else-if="error" variant="destructive" role="alert">
        <AlertDescription>{{ error }}</AlertDescription>
      </Alert>
      <template v-else-if="agent">
        <AgentProfileEditor
          v-if="editing"
          :key="agent.id"
          :agent="agent"
          @busy="busy = $event"
          @saved="updated"
          @cancel="editing = false"
          @deleted="emit('close')"
        />
        <div v-else class="flex flex-col gap-5" data-ui-id="agent-profile-view">
          <div class="flex items-center gap-4">
            <AgentAvatar :name="agent.name" :avatar="agent.avatar" class="size-16" />
            <div class="min-w-0">
              <h2 class="truncate text-xl font-semibold">{{ agent.name }}</h2>
              <Badge v-if="agent.archived" variant="secondary">Archived</Badge>
            </div>
          </div>
          <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
            <dt class="text-muted-foreground">Provider</dt>
            <dd>{{ agent.provider }}</dd>
            <dt class="text-muted-foreground">Model</dt>
            <dd class="break-all">{{ agent.model }}</dd>
          </dl>
          <div>
            <h3 class="mb-2 text-sm font-semibold">Personality</h3>
            <p class="whitespace-pre-wrap text-sm text-muted-foreground">
              {{ agent.systemPrompt || 'No personality added yet.' }}
            </p>
          </div>
          <p v-if="saved" role="status" class="text-sm">
            Saved. Changes apply to the next response.
          </p>
          <div class="flex gap-2">
            <Button v-if="!agent.archived" @click="emit('message', agent.id)">Message</Button>
            <Button variant="outline" @click="edit">Edit profile</Button>
          </div>
          <p v-if="agent.archived" class="text-sm text-muted-foreground">
            Edit this profile to restore the agent.
          </p>
        </div>
      </template>
    </DialogContent>
  </Dialog>
</template>
