<script setup lang="ts">
import type { AgentRequest } from '@/bridge/api'
import { Button } from '../ui/button'
defineProps<{
  requests: AgentRequest[]
  /** A response runs. A decision continues the requesting agent, so the channel must be idle. */
  running: boolean
  /** The request that is being decided. */
  busy?: string | null
}>()
const emit = defineEmits<{ decide: [requestId: string, decision: 'add' | 'deny'] }>()
const buttonClass =
  'border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)] hover:bg-[var(--protected-bg)] hover:text-[var(--protected-fg)] focus-visible:ring-[var(--protected-border)]'
const stateLabel: Record<AgentRequest['state'], string> = {
  pending: 'Pending',
  added: 'Added to the project.',
  denied: 'Denied.',
  failed: 'Failed.',
  expired: 'Expired.',
}
</script>
<template>
  <div v-if="requests.length" class="flex flex-col gap-3 px-3 py-3 sm:px-6">
    <section
      v-for="request in requests"
      :key="request.id"
      data-ui-id="agent-request-approval"
      data-protected="true"
      class="flex flex-col gap-3 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-4 text-sm text-[var(--protected-fg)]"
      aria-label="Agent add request"
    >
      <p class="font-semibold">Add an agent to this project?</p>
      <dl class="grid min-w-0 gap-1">
        <dt class="font-medium">Requested by</dt>
        <dd class="break-words">{{ request.requestingAgentName }}</dd>
        <dt class="mt-2 font-medium">Agent to add</dt>
        <dd class="break-words">
          {{ request.agentName }}
          <span class="font-mono">
            · {{ request.provider }} · {{ request.model ?? 'default model' }}
          </span>
        </dd>
        <dt class="mt-2 font-medium">Reason, in the words of {{ request.requestingAgentName }}</dt>
        <dd>
          <!-- The reason is model text. Show it as plain text in a quote, never as HTML or as a Discard statement. -->
          <blockquote
            data-testid="agent-request-reason"
            class="whitespace-pre-wrap break-words border-l-2 border-[var(--protected-border)] pl-3 italic"
          >
            {{ request.reason }}
          </blockquote>
        </dd>
      </dl>
      <template v-if="request.state === 'pending'">
        <p>
          The agent joins as a project member with no end date. Restarting Discard cancels pending
          requests.
        </p>
        <p v-if="running">Wait for the current response to finish before deciding.</p>
        <div class="flex gap-2">
          <Button
            variant="outline"
            :class="buttonClass"
            :disabled="!!busy || running"
            @click="emit('decide', request.id, 'deny')"
          >
            Deny
          </Button>
          <Button
            variant="outline"
            :class="buttonClass"
            :disabled="!!busy || running"
            @click="emit('decide', request.id, 'add')"
          >
            {{ busy === request.id ? 'Working…' : 'Add to project' }}
          </Button>
        </div>
      </template>
      <p v-else role="status" class="break-words">
        {{ request.result || stateLabel[request.state] }}
      </p>
    </section>
  </div>
</template>
