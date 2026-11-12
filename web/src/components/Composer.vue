<script setup lang="ts">
defineOptions({ name: 'MessageComposer' })
import { computed, nextTick, onUnmounted, ref, useId, watch } from 'vue'
import { useEventListener, useTextareaAutosize } from '@vueuse/core'
import {
  chooseAttachments,
  importAttachmentFiles,
  getAgents,
  getProjectMembers,
  runCommand,
  type AgentEntry,
} from '../bridge/api'
import { mentionQuery, mentionToken } from './mentions'
import AttachmentCard from './AttachmentCard.vue'
import type { AttachmentDto } from '../bridge/types'
import { Send, Square, AtSign, Paperclip, LoaderCircle, ChevronRight, Reply, X } from '@lucide/vue'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupTextarea } from './ui/input-group'
import { Checkbox } from './ui/checkbox'
import { Label } from './ui/label'
import { Button } from './ui/button'
import { messageSourceLink, type PendingReply } from './reply'
import { maySend, mayStop } from './composer'
import { commandLine, parseSlash } from './slash'
const props = defineProps<{
  /** The id of the channel or thread that this composer serves. Commands run against it. */
  channelId: string
  archived: boolean
  busy: boolean
  running: boolean
  channel: string
  directMessage?: boolean
  projectId?: string | null
  agent?: string
  draft?: string
  attachments?: AttachmentDto[]
  dropTarget?: HTMLElement | null
  /** Replaces the placeholder and the accessible name of the input. */
  placeholder?: string
  /** Shows the "Also post to" checkbox. The thread composer sets it to the parent channel name. */
  alsoPostLabel?: string
  /** A message can be sent while a turn runs, and Send shows next to Stop. A steer of a sub-agent needs it. */
  sendWhileRunning?: boolean
  /** The reply that waits for this composer. It shows as a chip above the input. */
  reply?: PendingReply | null
}>()
const emit = defineEmits<{
  send: [text: string, attachments?: AttachmentDto[], alsoPost?: boolean]
  stop: []
  cancelReply: []
  'update:draft': [text: string]
  'update:attachments': [attachments: AttachmentDto[]]
}>()
const placeholder = computed(
  () =>
    props.placeholder ??
    (props.directMessage ? `Message ${props.agent || 'agent'}` : `Message #${props.channel}`),
)
// The box applies to one message. It clears on every send.
const alsoPost = ref(false)
const alsoPostId = useId()
const localAttachments = ref<AttachmentDto[]>([])
const importing = ref(false)
const attachmentError = ref('')
// The last command result. It clears on a dismiss, a message, or a channel change.
const commandResult = ref<{ line: string; output: string; error: boolean } | null>(null)
const commandRunning = ref(false)
let commandRun = 0
let disposed = false
onUnmounted(() => {
  disposed = true
})
const files = computed({
  get: () => props.attachments ?? localAttachments.value,
  set: (value: AttachmentDto[]) => {
    localAttachments.value = value
    emit('update:attachments', value)
  },
})
const canAttach = computed(() => !props.archived && !props.busy && !importing.value)
async function addAttachments(importFiles: () => Promise<AttachmentDto[]>) {
  if (props.archived || props.busy || importing.value) return
  importing.value = true
  attachmentError.value = ''
  try {
    const added = await importFiles()
    if (disposed || !added.length) return
    if (files.value.length + added.length > 8) throw new Error('Attach up to 8 files per message.')
    if ([...files.value, ...added].reduce((total, file) => total + file.size, 0) > 20 * 1024 * 1024)
      throw new Error('Attachments must total 20 MB or less.')
    files.value = [...files.value, ...added]
  } catch (error) {
    if (!disposed) attachmentError.value = String(error)
  } finally {
    if (!disposed) importing.value = false
  }
}
function attach() {
  void addAttachments(chooseAttachments)
}
function importFiles(incoming: File[]) {
  if (!incoming.length) return
  void addAttachments(async () => {
    if (files.value.length + incoming.length > 8)
      throw new Error('Attach up to 8 files per message.')
    if (
      [...files.value, ...incoming].reduce((total, file) => total + file.size, 0) >
      20 * 1024 * 1024
    )
      throw new Error('Attachments must total 20 MB or less.')
    return importAttachmentFiles(incoming)
  })
}
const dragDepth = ref(0)
const dragging = computed(() => dragDepth.value > 0 && canAttach.value)
function hasFiles(event: DragEvent) {
  return Array.from(event.dataTransfer?.types ?? []).includes('Files')
}
function dragEnter(event: DragEvent) {
  if (!hasFiles(event)) return
  event.preventDefault()
  dragDepth.value++
}
function dragOver(event: DragEvent) {
  if (!hasFiles(event)) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = canAttach.value ? 'copy' : 'none'
}
function dragLeave(event: DragEvent) {
  if (hasFiles(event)) dragDepth.value = Math.max(0, dragDepth.value - 1)
}
function dropFiles(event: DragEvent) {
  dragDepth.value = 0
  if (!hasFiles(event)) return
  event.preventDefault()
  event.stopPropagation()
  if (!canAttach.value) return
  const items = Array.from(event.dataTransfer?.items ?? [])
  if (items.some((item) => item.webkitGetAsEntry?.()?.isDirectory)) {
    attachmentError.value = 'Attach files individually. Folders are not supported.'
    return
  }
  importFiles(Array.from(event.dataTransfer?.files ?? []))
}
function pasteImages(event: ClipboardEvent) {
  const clipboard = event.clipboardData
  if (!clipboard) return
  const images = Array.from(clipboard.files).filter((file) => file.type.startsWith('image/'))
  if (!images.length) return
  event.preventDefault()
  importFiles(images)
}
// Prevent file drops outside the active drop target from navigating away from the app.
useEventListener(window, 'dragover', (event: DragEvent) => {
  if (!hasFiles(event) || event.defaultPrevented) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'none'
})
useEventListener(window, 'drop', (event: DragEvent) => {
  if (hasFiles(event)) event.preventDefault()
  dragDepth.value = 0
})
function removeAttachment(id: string) {
  files.value = files.value.filter((file) => file.id !== id)
  attachmentError.value = ''
}
const localDraft = ref('')
const text = computed({
  get: () => props.draft ?? localDraft.value,
  set: (value) => {
    localDraft.value = value
    emit('update:draft', value)
  },
})
const isCommand = computed(() => parseSlash(text.value) !== null)
watch(
  () => props.channelId,
  () => {
    // Drop the result of a command that still runs for the old channel.
    commandRun++
    commandRunning.value = false
    commandResult.value = null
  },
)
const host = ref<HTMLElement | null>(null)
const dropZone = computed(() => props.dropTarget ?? host.value)
useEventListener(dropZone, 'dragenter', dragEnter)
useEventListener(dropZone, 'dragover', dragOver)
useEventListener(dropZone, 'dragleave', dragLeave)
useEventListener(dropZone, 'drop', dropFiles)
// Measure on mount, draft changes, and width changes; do not depend on WebKit's
// first content-sizing layout being refreshed by a pointer interaction.
useTextareaAutosize({
  element: computed(() => host.value?.querySelector('textarea') ?? null),
  input: text,
})
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
async function focusEnd() {
  await nextTick()
  if (disposed) return
  const input = host.value?.querySelector('textarea')
  if (!input || input.disabled) return
  input.focus()
  input.setSelectionRange(text.value.length, text.value.length)
  caret.value = text.value.length
  dismissed.value = true
}
defineExpose({ focusEnd })
function cancelReply() {
  emit('cancelReply')
  void focusEnd()
}
function blur(event: FocusEvent) {
  if (!host.value?.contains(event.relatedTarget as Node | null)) dismissed.value = true
}
async function runSlash(command: NonNullable<ReturnType<typeof parseSlash>>) {
  if (commandRunning.value) return
  const line = commandLine(command)
  if (files.value.length) {
    commandResult.value = { line, output: 'A command cannot have attachments.', error: true }
    return
  }
  const run = ++commandRun
  const sent = text.value
  commandRunning.value = true
  try {
    const output = await runCommand(props.channelId, command.name, command.args)
    if (disposed || run !== commandRun) return
    // Keep a draft that the user typed while the command ran.
    if (text.value === sent) text.value = ''
    commandResult.value = output ? { line, output, error: false } : null
  } catch (error) {
    if (disposed || run !== commandRun) return
    commandResult.value = { line, output: String(error), error: true }
  } finally {
    if (!disposed && run === commandRun) commandRunning.value = false
  }
}
function send() {
  if (
    props.archived ||
    props.busy ||
    importing.value ||
    !maySend(false, text.value, files.value.length)
  )
    return
  const command = parseSlash(text.value)
  if (command) {
    void runSlash(command)
    return
  }
  if (props.running && !props.sendWhileRunning) return
  // A command that still runs must not show its result after this message.
  commandRun++
  commandRunning.value = false
  commandResult.value = null
  if (props.alsoPostLabel !== undefined) {
    emit('send', text.value.trim(), [...files.value], alsoPost.value)
    alsoPost.value = false
  } else if (files.value.length) emit('send', text.value.trim(), [...files.value])
  else emit('send', text.value.trim())
  if (props.attachments === undefined) files.value = []
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
  // The mention list takes Escape first. Without it, Escape cancels the reply.
  if (event.key === 'Escape' && props.reply) {
    event.preventDefault()
    emit('cancelReply')
    return
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
      <p v-else-if="loadError" role="status" class="px-2 py-2 text-sm">
        {{ loadError }}
      </p>
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
    <p v-if="attachmentError" role="alert" class="mb-2 text-sm text-destructive">
      {{ attachmentError }}
    </p>
    <InputGroup
      class="relative h-auto flex-col items-stretch gap-2 rounded-lg p-2 has-[[data-slot=input-group-control]:focus-visible]:border-input has-[[data-slot=input-group-control]:focus-visible]:ring-0"
      :class="dragging ? 'border-primary ring-2 ring-primary/30' : ''"
    >
      <Teleport v-if="dragging" :to="dropTarget ?? 'body'" :disabled="!dropTarget">
        <div
          role="status"
          class="pointer-events-none absolute inset-0 z-30 flex items-center justify-center rounded-lg border-2 border-dashed border-primary bg-background/90 text-sm font-medium text-primary"
        >
          Drop files to attach
        </div>
      </Teleport>
      <div
        v-if="reply"
        data-ui-id="reply-chip"
        class="flex min-w-0 items-center gap-1 rounded-sm bg-muted/50 py-0.5 pr-0.5 pl-2 text-xs text-muted-foreground"
      >
        <a
          :href="messageSourceLink(reply.source)"
          class="flex min-w-0 flex-1 items-center gap-1 rounded-sm outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
        >
          <Reply class="size-3 shrink-0" aria-hidden="true" />
          <span class="shrink-0">
            Replying to
            <strong class="font-medium text-foreground">{{ reply.author }}</strong>
          </span>
          <span class="min-w-0 truncate">{{ reply.excerpt }}</span>
        </a>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Cancel reply"
          title="Cancel reply"
          @click="cancelReply"
        >
          <X aria-hidden="true" />
        </Button>
      </div>
      <div
        v-if="commandResult"
        data-ui-id="command-result"
        class="flex min-w-0 items-start gap-1 rounded-sm bg-muted/50 py-0.5 pr-0.5 pl-2 text-xs"
      >
        <div class="min-w-0 flex-1">
          <p class="truncate font-mono text-muted-foreground">{{ commandResult.line }}</p>
          <p
            :role="commandResult.error ? 'alert' : 'status'"
            class="max-h-48 overflow-y-auto break-words whitespace-pre-wrap"
            :class="commandResult.error ? 'text-destructive' : 'text-foreground'"
          >
            {{ commandResult.output }}
          </p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Dismiss command result"
          title="Dismiss command result"
          @click="commandResult = null"
        >
          <X aria-hidden="true" />
        </Button>
      </div>
      <div v-if="files.length" aria-label="Attachments" class="flex gap-2 overflow-x-auto pb-1">
        <AttachmentCard
          v-for="file in files"
          :key="file.id"
          :attachment="file"
          removable
          :disabled="busy || archived || importing"
          @remove="removeAttachment"
        />
      </div>
      <div class="flex min-w-0 items-end gap-2">
        <InputGroupAddon align="inline-start" class="shrink-0 p-0 has-[>button]:ml-0">
          <InputGroupButton
            size="icon-sm"
            aria-label="Attach files"
            title="Attach files"
            :disabled="archived || busy || importing || files.length >= 8"
            :aria-busy="importing"
            @click="attach"
          >
            <LoaderCircle v-if="importing" class="motion-safe:animate-spin" />
            <Paperclip v-else />
          </InputGroupButton>
        </InputGroupAddon>
        <span aria-hidden="true" class="flex h-8 shrink-0 items-center text-primary">
          <ChevronRight class="size-3.5" />
        </span>
        <InputGroupTextarea
          v-model="text"
          data-ui-id="composer-input"
          class="h-8 max-h-40 min-h-8 min-w-0 field-sizing-fixed overflow-y-auto px-1 py-1.5 leading-5"
          :rows="1"
          :disabled="archived || busy"
          :aria-label="placeholder"
          :placeholder="placeholder"
          :aria-expanded="!!query"
          :aria-controls="query ? listId : undefined"
          :aria-activedescendant="activeId"
          aria-autocomplete="list"
          @input="updateCaret"
          @paste="pasteImages"
          @click="updateCaret"
          @keyup.left="updateCaret"
          @keyup.right="updateCaret"
          @keyup.home="updateCaret"
          @keyup.end="updateCaret"
          @keydown="keydown"
        />
        <InputGroupAddon align="inline-end" class="shrink-0 gap-1 p-0 has-[>button]:mr-0">
          <InputGroupButton
            size="icon-sm"
            variant="ghost"
            aria-label="Mention an agent"
            :disabled="archived || busy || directMessage"
            :title="
              directMessage ? 'Mentions are available in project channels' : 'Mention an agent'
            "
            @click="insertMention"
          >
            <AtSign aria-hidden="true" class="size-4 text-muted-foreground" />
          </InputGroupButton>
          <InputGroupButton
            v-if="mayStop(running)"
            data-ui-id="composer-stop"
            size="sm"
            class="h-8"
            variant="destructive"
            @click="emit('stop')"
          >
            <Square data-icon="inline-start" />
            Stop
          </InputGroupButton>
          <InputGroupButton
            v-if="!mayStop(running) || sendWhileRunning || isCommand"
            data-ui-id="composer-send"
            size="icon-sm"
            variant="default"
            :disabled="
              commandRunning || !maySend(archived || busy || importing, text, files.length)
            "
            :aria-busy="commandRunning"
            aria-label="Send message"
            @click="send"
          >
            <LoaderCircle v-if="commandRunning" class="motion-safe:animate-spin" />
            <Send v-else />
            <span class="sr-only">Send</span>
          </InputGroupButton>
        </InputGroupAddon>
      </div>
    </InputGroup>
    <div v-if="alsoPostLabel !== undefined" class="mt-2 flex items-center gap-2 px-1">
      <Checkbox
        :id="alsoPostId"
        v-model="alsoPost"
        data-ui-id="composer-also-post"
        :disabled="archived || busy"
      />
      <Label :for="alsoPostId" class="text-xs font-normal text-muted-foreground">
        Also post to {{ alsoPostLabel }}
      </Label>
    </div>
    <p
      v-if="archived && !directMessage && alsoPostLabel === undefined"
      class="mt-2 text-xs text-muted-foreground"
    >
      Channel archived
    </p>
  </section>
</template>
