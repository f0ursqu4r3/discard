<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  getChannelDetails,
  getAgents,
  getProjectMembers,
  selectCanonicalBranch,
  selectCanonicalProjectChannel,
  setProjectMainAgent,
  switchChannel,
  updateChannelTopic,
} from '@/bridge/api'
import type { AgentEntry } from '@/bridge/api'
import type { ChannelDetailsDto, ChannelMemberDto } from '@/bridge/types'
import { Button } from '../ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card'
import { Textarea } from '../ui/textarea'

const props = defineProps<{
  projectId: string
  channelId: string
  generalRunning: boolean
}>()
const emit = defineEmits<{ mainAgentChanged: []; canonicalChanged: [] }>()
const details = ref<ChannelDetailsDto | null>(null)
const projectMembers = ref<ChannelMemberDto[]>([])
const agents = ref<AgentEntry[]>([])
const topicDraft = ref('')
const loading = ref(true)
const savingTopic = ref(false)
const savingMainAgent = ref(false)
const selectingCanonical = ref(false)
const error = ref('')
const staleTopic = ref(false)
let requestId = 0
let contextEpoch = 0
let topicOperation = 0
let mainAgentOperation = 0
let canonicalOperation = 0
let stopDetailsEvent: UnlistenFn | undefined
let disposed = false

type OperationContext = { projectId: string; channelId: string; epoch: number }
function captureContext(): OperationContext {
  return { projectId: props.projectId, channelId: props.channelId, epoch: contextEpoch }
}
function isCurrentContext(context: OperationContext) {
  return (
    !disposed &&
    context.epoch === contextEpoch &&
    context.projectId === props.projectId &&
    context.channelId === props.channelId
  )
}

async function load(options: { discardDraft?: boolean; forceStale?: boolean } = {}) {
  const currentRequest = ++requestId
  const projectId = props.projectId
  const channelId = props.channelId
  if (
    details.value &&
    (details.value.channel.projectId !== projectId || details.value.channel.id !== channelId)
  ) {
    details.value = null
    topicDraft.value = ''
    staleTopic.value = false
  }
  loading.value = true
  error.value = ''
  try {
    const [nextDetails, nextMembers, nextAgents] = await Promise.all([
      getChannelDetails(projectId, channelId),
      getProjectMembers(projectId),
      getAgents(),
    ])
    if (
      disposed ||
      currentRequest !== requestId ||
      projectId !== props.projectId ||
      channelId !== props.channelId
    )
      return
    if (nextDetails.channel.id !== channelId || nextDetails.channel.projectId !== projectId)
      throw new Error('Channel details did not match the requested project and channel.')
    const currentDetails = details.value
    const sameContext =
      currentDetails?.channel.id === channelId && currentDetails.channel.projectId === projectId
    const currentDraft = topicDraft.value
    const draftIsDirty =
      !!currentDetails && sameContext && currentDraft !== currentDetails.topic.text
    const versionChanged =
      !!currentDetails && sameContext && nextDetails.topic.version !== currentDetails.topic.version
    const wasStale = !!currentDetails && sameContext && staleTopic.value
    const keepDraft = draftIsDirty && !options.discardDraft
    details.value = nextDetails
    projectMembers.value = nextMembers
    agents.value = nextAgents
    topicDraft.value = keepDraft ? currentDraft : nextDetails.topic.text
    staleTopic.value = options.forceStale || (keepDraft && (versionChanged || wasStale))
  } catch (cause) {
    if (!disposed && currentRequest === requestId) error.value = String(cause)
  } finally {
    if (!disposed && currentRequest === requestId) loading.value = false
  }
}

watch(
  () => [props.projectId, props.channelId],
  () => {
    contextEpoch++
    topicOperation++
    mainAgentOperation++
    canonicalOperation++
    savingTopic.value = false
    savingMainAgent.value = false
    selectingCanonical.value = false
    void load()
  },
  { immediate: true, flush: 'sync' },
)
async function refreshFromEvent() {
  const context = captureContext()
  await load()
  if (staleTopic.value && isCurrentContext(context)) {
    error.value = 'The topic changed while you were editing. Reload the latest topic before saving.'
  }
}
async function refreshPreservingDraft() {
  const context = captureContext()
  await load()
  if (staleTopic.value && isCurrentContext(context)) {
    error.value = 'Details changed while you were editing. Reload the latest topic before saving.'
  }
}
onMounted(async () => {
  try {
    const stop = await listen<string>('discard://channel-details', (event) => {
      if (event.payload === props.channelId) void refreshFromEvent()
    })
    if (disposed) stop()
    else stopDetailsEvent = stop
  } catch (cause) {
    if (!disposed) error.value = String(cause)
  }
})
onUnmounted(() => {
  disposed = true
  requestId++
  stopDetailsEvent?.()
})

async function saveTopic() {
  const context = captureContext()
  const current = details.value
  const projectId = context.projectId
  const channelId = context.channelId
  if (current?.channel.projectId !== projectId || current.channel.id !== channelId) return
  if (!current || savingTopic.value || current.channel.archived || staleTopic.value) return
  const operation = ++topicOperation
  savingTopic.value = true
  error.value = ''
  try {
    await updateChannelTopic(projectId, channelId, current.topic.version, topicDraft.value)
    if (!isCurrentContext(context)) return
    await load({ discardDraft: true })
  } catch (cause) {
    if (!isCurrentContext(context)) return
    const message = String(cause)
    if (message.includes('Topic changed.')) {
      await load({ forceStale: true })
      if (!isCurrentContext(context)) return
      staleTopic.value = true
      error.value = 'The topic changed elsewhere. Reload the latest topic before saving again.'
    } else {
      error.value = message
    }
  } finally {
    if (operation === topicOperation) savingTopic.value = false
  }
}

function cancelTopic() {
  topicDraft.value = details.value?.topic.text ?? ''
  staleTopic.value = false
  error.value = ''
}

async function changeMainAgent(event: Event) {
  const agentId = (event.target as HTMLSelectElement).value
  const context = captureContext()
  const currentChannelId = context.channelId
  const currentProjectId = context.projectId
  if (
    !details.value ||
    details.value.channel.id !== currentChannelId ||
    details.value.channel.projectId !== currentProjectId
  )
    return
  if (!agentId || savingMainAgent.value || props.generalRunning) return
  const operation = ++mainAgentOperation
  savingMainAgent.value = true
  error.value = ''
  let commandError = ''
  try {
    await setProjectMainAgent(currentProjectId, agentId)
  } catch (cause) {
    commandError = String(cause)
  } finally {
    if (isCurrentContext(context)) {
      await refreshPreservingDraft()
      if (isCurrentContext(context)) {
        if (commandError) error.value = commandError
        emit('mainAgentChanged')
      }
    }
    if (operation === mainAgentOperation) savingMainAgent.value = false
  }
}

async function makeCanonical() {
  const channel = details.value?.channel
  const context = captureContext()
  const projectId = context.projectId
  if (
    !channel ||
    channel.projectId !== projectId ||
    channel.id !== props.channelId ||
    channel.kind !== 'archived_legacy' ||
    (!channel.branch && channel.name.toLocaleLowerCase() !== 'general') ||
    selectingCanonical.value
  )
    return
  const operation = ++canonicalOperation
  selectingCanonical.value = true
  error.value = ''
  let commandError = ''
  try {
    if (channel.branch) await selectCanonicalBranch(projectId, channel.branch, channel.id)
    else await selectCanonicalProjectChannel(projectId, channel.id)
    if (!isCurrentContext(context)) return
    await switchChannel(channel.id)
    if (!isCurrentContext(context)) return
  } catch (cause) {
    commandError = String(cause)
  } finally {
    if (isCurrentContext(context)) {
      await refreshPreservingDraft()
      if (isCurrentContext(context)) {
        if (commandError) error.value = commandError
        emit('canonicalChanged')
      }
    }
    if (operation === canonicalOperation) selectingCanonical.value = false
  }
}

function authorName(authorKind: string, authorId: string | null) {
  if (authorKind === 'user') return 'You'
  return projectMembers.value.find((member) => member.id === authorId)?.name ?? 'Agent'
}
</script>

<template>
  <Card data-ui-id="project-channel-details" size="sm">
    <CardHeader>
      <CardTitle class="text-sm">Channel details</CardTitle>
    </CardHeader>
    <CardContent v-if="loading && !details" role="status">Loading channel details…</CardContent>
    <CardContent v-else-if="details" class="flex flex-col gap-4">
      <div class="flex flex-col gap-1 text-xs">
        <p class="font-semibold">{{ details.channel.name }}</p>
        <p v-if="details.channel.branch" class="text-muted-foreground">
          Branch · {{ details.channel.branch }}
        </p>
        <p v-else-if="details.channel.kind === 'general'" class="text-muted-foreground">
          Project-wide channel
        </p>
        <p class="text-muted-foreground">
          {{
            details.channel.archived
              ? details.channel.kind === 'archived_legacy'
                ? 'Archived history'
                : 'Archived channel'
              : 'Active channel'
          }}
        </p>
        <Button
          v-if="
            details.channel.kind === 'archived_legacy' &&
            (details.channel.branch || details.channel.name.toLocaleLowerCase() === 'general')
          "
          data-ui-id="select-canonical-channel"
          size="sm"
          variant="outline"
          :disabled="selectingCanonical"
          @click="makeCanonical"
        >
          {{ selectingCanonical ? 'Selecting…' : 'Use as canonical channel' }}
        </Button>
        <p
          v-else-if="details.channel.kind === 'archived_legacy'"
          class="text-xs text-muted-foreground"
        >
          This history has no supported branch or general channel to select.
        </p>
      </div>

      <section class="flex flex-col gap-2">
        <label for="channel-topic" class="text-xs font-semibold">Topic</label>
        <Textarea
          id="channel-topic"
          v-model="topicDraft"
          aria-label="Channel topic"
          :disabled="details.channel.archived || savingTopic"
          rows="3"
        />
        <div class="flex justify-end gap-2">
          <Button
            v-if="staleTopic"
            data-ui-id="reload-topic"
            size="sm"
            variant="ghost"
            @click="load({ discardDraft: true })"
          >
            Use latest
          </Button>
          <Button
            data-ui-id="cancel-topic"
            size="sm"
            variant="ghost"
            :disabled="savingTopic"
            @click="cancelTopic"
          >
            Cancel
          </Button>
          <Button
            data-ui-id="save-topic"
            size="sm"
            :disabled="
              loading ||
              savingTopic ||
              details.channel.archived ||
              staleTopic ||
              topicDraft === details.topic.text
            "
            @click="saveTopic"
          >
            {{ savingTopic ? 'Saving…' : 'Save topic' }}
          </Button>
        </div>
        <p v-if="details.channel.archived" class="text-xs text-muted-foreground">
          Archived history is read-only.
        </p>
      </section>

      <p v-if="error" role="alert" class="text-xs text-destructive">{{ error }}</p>

      <section v-if="details.topicHistory.length" class="flex flex-col gap-2">
        <h3 class="text-xs font-semibold">Topic history</h3>
        <ol class="flex flex-col gap-2">
          <li
            v-for="revision in [...details.topicHistory].reverse()"
            :key="revision.version"
            class="border-l-2 border-border pl-2 text-xs"
          >
            <p class="text-muted-foreground">
              Version {{ revision.version }} ·
              {{ authorName(revision.authorKind, revision.authorId) }}
            </p>
            <p class="whitespace-pre-wrap">{{ revision.text }}</p>
          </li>
        </ol>
      </section>

      <section v-if="details.mainAgentId" class="flex flex-col gap-2">
        <label for="project-main-agent" class="text-xs font-semibold">Project main agent</label>
        <select
          id="project-main-agent"
          data-ui-id="project-main-agent"
          :value="details.mainAgentId"
          :disabled="loading || savingMainAgent || generalRunning"
          class="h-9 rounded-md border border-input bg-background px-2 text-sm"
          @change="changeMainAgent"
        >
          <option
            v-for="agent in agents"
            :key="agent.id"
            :value="agent.id"
            :disabled="agent.archived"
          >
            {{ agent.name }} · {{ agent.provider }} · {{ agent.model
            }}{{ agent.archived ? ' · Archived' : '' }}
          </option>
        </select>
        <p v-if="generalRunning" class="text-xs text-muted-foreground">
          The main agent cannot change while the general channel is running.
        </p>
      </section>

      <section class="flex flex-col gap-2">
        <h3 class="text-xs font-semibold">Participants · {{ details.members.length }}</h3>
        <ul class="flex flex-col gap-1 text-xs">
          <li v-for="member in details.members" :key="member.id" class="flex justify-between gap-2">
            <span class="truncate">{{ member.name }} · {{ member.provider }}</span>
            <span
              v-if="member.id === details.channel.agentId"
              class="shrink-0 text-muted-foreground"
            >
              Responds here
            </span>
            <span
              v-else-if="member.id === details.mainAgentId"
              class="shrink-0 text-muted-foreground"
            >
              Project main
            </span>
            <span v-else-if="member.archived" class="shrink-0 text-muted-foreground">Archived</span>
          </li>
        </ul>
        <p class="text-xs text-muted-foreground">
          Membership is saved. Responses use the agent assigned to this channel.
        </p>
      </section>
    </CardContent>
    <CardContent v-else-if="error" role="alert" class="text-xs text-destructive">
      {{ error }}
    </CardContent>
  </Card>
</template>
