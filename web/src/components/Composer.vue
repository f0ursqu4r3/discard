<script setup lang="ts">
defineOptions({ name: 'MessageComposer' })
import { computed, nextTick, ref, useId, watch } from 'vue'
import { getAgents, getProjectMembers, type AgentEntry } from '../bridge/api'
import { mentionQuery, mentionToken } from './mentions'
import { Send, Square, AtSign } from '@lucide/vue'
import { InputGroup, InputGroupAddon, InputGroupTextarea } from './ui/input-group'
import { Button } from './ui/button'
import { maySend, mayStop } from './composer'
const props = defineProps<{
  archived: boolean
  busy: boolean
  running: boolean
  channel: string
  directMessage?: boolean
  projectId?: string | null
  agent?: string
  draft?: string
}>()
const emit = defineEmits<{
  send: [text: string]
  stop: []
  'update:draft': [text: string]
}>()
const placeholder = computed(() =>
  props.directMessage ? `Message ${props.agent || 'agent'}` : `Message #${props.channel}`,
)
const localDraft = ref('')
const text = computed({
  get: () => props.draft ?? localDraft.value,
  set: (value) => {
    localDraft.value = value
    emit('update:draft', value)
  },
})
const host = ref<HTMLElement | null>(null)
const caret = ref(0)
const dismissed = ref(false)
const agents = ref<AgentEntry[]>([])
const members = ref(new Set<string>())
const loading = ref(false)
const loadError = ref('')
const selected = ref(0)
const listId = useId()
const query = computed(() =>
  props.directMessage || props.archived || props.busy || dismissed.value
    ? null
    : mentionQuery(text.value, caret.value),
)
const candidates = computed(() =>
  agents.value
    .filter(
      (agent) =>
        !agent.archived &&
        agent.model &&
        agent.name.toLowerCase().startsWith(query.value?.query.toLowerCase() ?? ''),
    )
    .map((agent) => ({
      ...agent,
      ambiguous:
        agents.value.filter(
          (other) => !other.archived && other.name.toLowerCase() === agent.name.toLowerCase(),
        ).length > 1 ||
        agent.name.includes('"') ||
        agent.name.includes('\n'),
    }))
    .sort(
      (a, b) =>
        Number(members.value.has(b.id)) - Number(members.value.has(a.id)) ||
        a.name.localeCompare(b.name),
    ),
)
const selectable = computed(() => candidates.value.filter((agent) => !agent.ambiguous))
const activeId = computed(() =>
  query.value && selectable.value[selected.value]
    ? `${listId}-${selectable.value[selected.value]!.id}`
    : undefined,
)
let request = 0
async function loadAgents() {
  const token = ++request
  loading.value = true
  loadError.value = ''
  try {
    const [available, projectMembers] = await Promise.all([
      getAgents(),
      props.projectId ? getProjectMembers(props.projectId) : Promise.resolve([]),
    ])
    if (token !== request) return
    agents.value = available
    members.value = new Set(projectMembers.map((member) => member.id))
  } catch {
    if (token === request) {
      agents.value = []
      loadError.value = 'Could not load agents. Type an agent name or try again.'
    }
  } finally {
    if (token === request) loading.value = false
  }
}
watch(
  () => !!query.value,
  (open) => {
    if (open) void loadAgents()
  },
)
watch(
  () => props.projectId,
  () => {
    request++
    agents.value = []
    members.value = new Set()
    if (query.value) void loadAgents()
  },
)
watch(
  () => query.value?.query,
  () => {
    selected.value = 0
  },
)
function updateCaret(event: Event) {
  caret.value = (event.target as HTMLTextAreaElement).selectionStart
  dismissed.value = false
}
async function choose(agent: AgentEntry) {
  const match = query.value
  if (!match) return
  const token = mentionToken(agent.name) + (text.value[match.end] === ' ' ? '' : ' ')
  text.value = text.value.slice(0, match.start) + token + text.value.slice(match.end)
  dismissed.value = true
  await nextTick()
  const input = host.value?.querySelector('textarea')
  input?.focus()
  input?.setSelectionRange(match.start + token.length, match.start + token.length)
  caret.value = match.start + token.length
}
async function insertMention() {
  const input = host.value?.querySelector('textarea')
  if (!input) return
  const start = input.selectionStart
  const end = input.selectionEnd
  const token = (start && !/[\s(]/.test(text.value[start - 1]!) ? ' ' : '') + '@'
  text.value = text.value.slice(0, start) + token + text.value.slice(end)
  await nextTick()
  input.focus()
  input.setSelectionRange(start + token.length, start + token.length)
  caret.value = start + token.length
  dismissed.value = false
}
function blur(event: FocusEvent) {
  if (!host.value?.contains(event.relatedTarget as Node | null)) dismissed.value = true
}
function send() {
  if (props.running || !maySend(props.archived || props.busy, text.value)) return
  emit('send', text.value.trim())
  if (props.draft === undefined) text.value = ''
}
function keydown(event: KeyboardEvent) {
  if (event.isComposing) return
  if (query.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      dismissed.value = true
      return
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const count = selectable.value.length
      if (count)
        selected.value = (selected.value + (event.key === 'ArrowDown' ? 1 : -1) + count) % count
      return
    }
    if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') {
      const agent = selectable.value[selected.value]
      if (agent) {
        event.preventDefault()
        void choose(agent)
        return
      }
      // Do not send a partial name while the picker is loading or has no match.
      if (event.key === 'Enter') {
        event.preventDefault()
        return
      }
    }
  }
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
    event.preventDefault()
    send()
  }
}
</script>
<template>
  <section
    ref="host"
    @focusout="blur"
    data-ui-id="message-composer"
    aria-label="Message composer"
    class="relative mt-auto shrink-0 px-3 pt-2 pb-3"
  >
    <div
      v-if="query"
      class="absolute right-3 bottom-full left-3 z-20 mb-1 max-h-64 overflow-y-auto rounded-lg border bg-popover p-1 text-popover-foreground shadow-md"
    >
      <p class="px-2 py-1 text-xs text-muted-foreground">Mention one agent</p>
      <p v-if="loading" role="status" class="px-2 py-2 text-sm">Loading agents…</p>
      <p v-else-if="loadError" role="status" class="px-2 py-2 text-sm">{{ loadError }}</p>
      <p v-else-if="!candidates.length" role="status" class="px-2 py-2 text-sm">
        No matching agents
      </p>
      <div :id="listId" role="listbox" aria-label="Agents">
        <button
          v-for="candidate in candidates"
          :id="`${listId}-${candidate.id}`"
          :key="candidate.id"
          type="button"
          role="option"
          :aria-selected="activeId === `${listId}-${candidate.id}`"
          :disabled="candidate.ambiguous"
          :aria-disabled="candidate.ambiguous"
          tabindex="-1"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-2 py-2 text-left text-sm hover:bg-accent disabled:opacity-50"
          :class="
            activeId === `${listId}-${candidate.id}` ? 'bg-accent text-accent-foreground' : ''
          "
          @pointerdown.prevent
          @click="choose(candidate)"
        >
          <span>{{ candidate.name }}</span>
          <span class="text-xs text-muted-foreground">
            {{
              candidate.ambiguous
                ? 'Rename this agent to mention it'
                : members.has(candidate.id)
                  ? 'Project member'
                  : 'Add to channel'
            }}
          </span>
        </button>
      </div>
    </div>
    <InputGroup class="rounded-lg">
      <InputGroupTextarea
        v-model="text"
        data-ui-id="composer-input"
        class="max-h-40 min-h-12 py-3"
        :rows="1"
        :disabled="archived || busy"
        :aria-label="placeholder"
        :placeholder="placeholder"
        :aria-expanded="!!query"
        :aria-controls="query ? listId : undefined"
        :aria-activedescendant="activeId"
        aria-autocomplete="list"
        @input="updateCaret"
        @click="updateCaret"
        @keyup.left="updateCaret"
        @keyup.right="updateCaret"
        @keyup.home="updateCaret"
        @keyup.end="updateCaret"
        @keydown="keydown"
      />
      <InputGroupAddon align="inline-end" class="self-end pb-2">
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Mention an agent"
          :disabled="archived || busy || directMessage"
          :title="directMessage ? 'Mentions are available in project channels' : 'Mention an agent'"
          @click="insertMention"
        >
          <AtSign aria-hidden="true" class="size-4 text-muted-foreground" />
        </Button>
        <Button
          v-if="mayStop(running)"
          data-ui-id="composer-stop"
          size="sm"
          variant="destructive"
          @click="emit('stop')"
        >
          <Square data-icon="inline-start" />
          Stop
        </Button>
        <Button
          v-else
          data-ui-id="composer-send"
          size="icon-sm"
          :disabled="!maySend(archived || busy, text)"
          aria-label="Send message"
          @click="send"
        >
          <Send />
          <span class="sr-only">Send</span>
        </Button>
      </InputGroupAddon>
    </InputGroup>
    <p v-if="archived && !directMessage" class="mt-2 text-xs text-muted-foreground">
      Channel archived
    </p>
  </section>
</template>
