<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { ChevronDown, Plus, Power, X, RotateCcw, SquareTerminal } from '@lucide/vue'
import { Button } from '../ui/button'
import { activeTheme } from '@/bridge/theme'
import { shortPath } from '@/settings/paths'
import {
  decodeBytes,
  terminalClose,
  terminalCreate,
  terminalDetach,
  terminalKill,
  terminalOpen,
  terminalResize,
  terminalRestart,
  terminalTabs,
  terminalWrite,
  type TerminalEvent,
  type TerminalOpened,
  type TerminalTab,
} from '@/bridge/terminal'
import { terminalFont, terminalTheme } from './terminal-theme'
import { terminalKey } from './terminal-keys'
import {
  applyEvent,
  canAddTab,
  canKill,
  canRestart,
  clampHeight,
  connecting,
  failed,
  loadHeight,
  nextActive,
  opened,
  pickActive,
  recallActive,
  rememberActive,
  saveHeight,
  statusLine,
  tabName,
  TAB_LIMIT_REASON,
  type SessionView,
} from './terminal-state'

const props = defineProps<{
  channelId: string
  /** The shown shortcut of Toggle Terminal. The terminal passes this key to the app. */
  toggleShortcut?: string
}>()
const emit = defineEmits<{ hide: [] }>()

const panel = ref<HTMLElement | null>(null)
const host = ref<HTMLElement | null>(null)
const view = ref<SessionView>(connecting())
const tabs = ref<TerminalTab[]>([])
const activeId = ref<string | null>(null)
// The text of a refused action. It shows in the status strip until the next open.
const refusal = ref('')
const height = ref(loadHeight())
const dragging = ref(false)

let term: Terminal | null = null
let fit: FitAddon | null = null
let observers: ResizeObserver[] = []
// The session that this view shows. Input and resize go there.
let attached: string | null = null
let generation = 0
// Opens run one at a time, so a later open of a channel always replaces the stream of an earlier one.
let queue: Promise<unknown> = Promise.resolve()
// Keystrokes go out one call at a time, so the shell sees them in order.
let writes: Promise<unknown> = Promise.resolve()
let frame = 0

const status = computed(() => refusal.value || statusLine(view.value))
const activeTab = computed(() => tabs.value.find((tab) => tab.sessionId === activeId.value))
const activeCwd = computed(() => activeTab.value?.cwd || view.value.cwd)
const cwdText = computed(() => (activeCwd.value ? shortPath(activeCwd.value) : ''))
const tabsFull = computed(() => !canAddTab(tabs.value.length))
const theme = computed(() => terminalTheme(activeTheme.value))
const font = computed(() => terminalFont(activeTheme.value))
const ignore = () => {}

function parentHeight(): number {
  return panel.value?.parentElement?.clientHeight ?? 0
}

function applyLook() {
  if (!term) return
  term.options.theme = theme.value
  term.options.fontFamily = font.value.fontFamily
  term.options.fontSize = font.value.fontSize
  refit()
}

function refit() {
  try {
    fit?.fit()
  } catch {
    // The host has no size yet, for example while the panel is hidden.
  }
}

/** The error text of a session that the backend no longer holds. Keep it in step with terminal.rs. */
const ENDED = 'This terminal session has ended.'
const isGone = (error: unknown) => String(error).includes(ENDED)

/**
 * Bring the strip in step with the backend: drop tabs it no longer has, refresh the others, add tabs that
 * another window opened. The active tab stays even when it is gone, so Restart can replace it in place.
 */
async function reconcile() {
  const channelId = props.channelId
  let list: TerminalTab[]
  try {
    list = await terminalTabs(channelId)
  } catch {
    return
  }
  if (props.channelId !== channelId) return
  const known = new Map(list.map((tab) => [tab.sessionId, tab]))
  const shown = new Set(tabs.value.map((tab) => tab.sessionId))
  const merged: TerminalTab[] = []
  for (const tab of tabs.value) {
    const fresh = known.get(tab.sessionId)
    if (fresh) merged.push(fresh)
    else if (tab.sessionId === activeId.value) merged.push({ ...tab, running: false })
  }
  tabs.value = [...merged, ...list.filter((tab) => !shown.has(tab.sessionId))]
}

function deliver(event: TerminalEvent) {
  if (event.kind === 'output') {
    term?.write(decodeBytes(event.data))
    return
  }
  view.value = applyEvent(view.value, event)
  if (event.kind === 'exit' || event.kind === 'closed') markEnded(activeId.value)
  // The app closed the session, and it can have closed the other tabs of the channel too.
  if (event.kind === 'closed' && event.reason !== KILLED) void reconcile()
}

const KILLED = 'The session was killed.'

function markEnded(id: string | null) {
  tabs.value = tabs.value.map((tab) => (tab.sessionId === id ? { ...tab, running: false } : tab))
}

function tabOf(result: TerminalOpened): TerminalTab {
  return {
    sessionId: result.sessionId,
    number: result.number,
    cwd: result.cwd,
    running: result.running,
    exitCode: result.exitCode,
    signal: result.signal,
  }
}

function setActive(id: string) {
  activeId.value = id
  rememberActive(props.channelId, id)
}

type Opener = (
  cols: number,
  rows: number,
  onEvent: (event: TerminalEvent) => void,
) => Promise<TerminalOpened>

/**
 * Show one session. `open` attaches it: it opens, creates, or restarts. `onOpened` records the result in the tab list.
 * Returns the error when the open failed.
 */
async function attach(
  run: number,
  open: Opener,
  onOpened?: (result: TerminalOpened, current: boolean) => void,
  focusTerminal = true,
): Promise<{ error: unknown } | null> {
  const current = term
  if (!current || run !== generation) return null
  current.reset()
  view.value = connecting()
  refusal.value = ''
  let early: TerminalEvent[] | null = []
  refit()
  try {
    const result = await open(current.cols, current.rows, (event) => {
      if (early) early.push(event)
      else if (run === generation) deliver(event)
    })
    if (run !== generation) {
      // The backend holds the session, so the tab list keeps it. Only the active tab streams here.
      onOpened?.(result, false)
      // A newer attach took over. A session that is not the active tab must stop streaming here.
      if (activeId.value !== result.sessionId) void terminalDetach(result.sessionId).catch(ignore)
      return null
    }
    onOpened?.(result, true)
    attached = result.sessionId
    const bytes = decodeBytes(result.scrollback)
    if (bytes.length) current.write(bytes)
    view.value = opened(view.value, result)
    const pending = early
    early = null
    pending.forEach(deliver)
    if (focusTerminal) current.focus()
    return null
  } catch (error) {
    if (run === generation) {
      view.value = failed(view.value, error)
      if (isGone(error)) void reconcile()
    }
    return { error }
  }
}

const openTab =
  (id: string): Opener =>
  (cols, rows, onEvent) =>
    terminalOpen(id, cols, rows, onEvent)

async function load(channelId: string, run: number) {
  const current = term
  if (!current || run !== generation) return
  view.value = connecting()
  let list: TerminalTab[]
  try {
    list = await terminalTabs(channelId)
  } catch (error) {
    if (run === generation) view.value = failed(view.value, error)
    return
  }
  if (run !== generation) return
  if (list.length === 0) {
    await attach(
      run,
      (cols, rows, onEvent) => terminalCreate(channelId, cols, rows, onEvent),
      (result, current) => {
        if (!current) return
        tabs.value = [tabOf(result)]
        setActive(result.sessionId)
      },
    )
    return
  }
  tabs.value = list
  const id = pickActive(list, recallActive(channelId))!
  setActive(id)
  await attach(run, openTab(id))
}

function start(channelId: string) {
  const run = ++generation
  attached = null
  tabs.value = []
  activeId.value = null
  refusal.value = ''
  queue = queue.then(() => load(channelId, run)).catch(ignore)
}

/** Stop the stream of the shown session. The session keeps running. */
function detachCurrent() {
  if (attached) void terminalDetach(attached).catch(ignore)
  attached = null
}

function selectTab(id: string, focusTerminal = true) {
  if (id === activeId.value) return
  detachCurrent()
  setActive(id)
  const run = ++generation
  queue = queue.then(() => attach(run, openTab(id), undefined, focusTerminal)).catch(ignore)
}

function addTab() {
  if (!canAddTab(tabs.value.length)) return
  const channelId = props.channelId
  const previous = activeId.value
  detachCurrent()
  const run = ++generation
  queue = queue
    .then(async () => {
      const failure = await attach(
        run,
        (cols, rows, onEvent) => terminalCreate(channelId, cols, rows, onEvent),
        (result, current) => {
          if (props.channelId !== channelId) return
          tabs.value = [...tabs.value, tabOf(result)]
          if (current) setActive(result.sessionId)
        },
      )
      // The backend refused the tab. Show the tab that was open and tell the user why.
      if (failure && previous && run === generation) {
        await attach(run, openTab(previous))
        await reconcile()
        refusal.value = String(failure.error)
      }
    })
    .catch(ignore)
}

/** Left and Right move the active tab. The strip wraps at its ends. */
function moveTab(event: KeyboardEvent, id: string) {
  const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
  if (!step) return
  event.preventDefault()
  const list = tabs.value
  const index = list.findIndex((tab) => tab.sessionId === id)
  const target = list[(index + step + list.length) % list.length]
  if (!target) return
  selectTab(target.sessionId, false)
  void nextTick(() => {
    const buttons = panel.value?.querySelectorAll<HTMLElement>('[data-testid="terminal-tab"]')
    buttons?.[(index + step + list.length) % list.length]?.focus()
  })
}

async function closeTab(id: string) {
  try {
    await terminalClose(id)
  } catch (error) {
    // A refused close keeps the tab. A session that is already gone has no tab to keep.
    if (!isGone(error)) {
      refusal.value = String(error)
      return
    }
  }
  // The person can select another tab while the close runs. That choice stays.
  const wasActive = id === activeId.value
  const next = wasActive ? nextActive(tabs.value, id) : null
  if (wasActive) {
    generation++
    attached = null
  }
  tabs.value = tabs.value.filter((tab) => tab.sessionId !== id)
  if (!wasActive) return
  if (!next) {
    activeId.value = null
    emit('hide')
    return
  }
  setActive(next)
  const run = ++generation
  queue = queue.then(() => attach(run, openTab(next))).catch(ignore)
}

// Opening again moves the session to this window. The other window gets the moved notice.
function showHere() {
  const id = activeId.value
  if (!id) return
  const run = ++generation
  attached = null
  queue = queue.then(() => attach(run, openTab(id))).catch(ignore)
}

function restart() {
  const id = activeId.value
  if (!id) {
    start(props.channelId)
    return
  }
  const run = ++generation
  const channelId = props.channelId
  attached = null
  queue = queue
    .then(() =>
      attach(
        run,
        async (cols, rows, onEvent) => {
          try {
            return await terminalRestart(id, cols, rows, onEvent)
          } catch (error) {
            // The app closed the session. Start a new shell in its place.
            if (!isGone(error)) throw error
            return terminalCreate(channelId, cols, rows, onEvent)
          }
        },
        (result, current) => {
          if (props.channelId !== channelId) return
          tabs.value = tabs.value.some((tab) => tab.sessionId === id)
            ? tabs.value.map((tab) => (tab.sessionId === id ? tabOf(result) : tab))
            : [...tabs.value, tabOf(result)]
          if (current) setActive(result.sessionId)
        },
      ),
    )
    .catch(ignore)
}

async function kill() {
  const id = activeId.value
  if (!id) return
  try {
    await terminalKill(id)
  } catch (error) {
    refusal.value = String(error)
    return
  }
  if (activeId.value !== id) return
  view.value = { ...view.value, phase: 'closed', message: KILLED }
  markEnded(id)
}

function startDrag(event: PointerEvent) {
  dragging.value = true
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture?.(event.pointerId)
  const startY = event.clientY
  const startHeight = height.value
  const move = (next: PointerEvent) => {
    height.value = clampHeight(startHeight + (startY - next.clientY), parentHeight())
  }
  const stop = () => {
    dragging.value = false
    target.removeEventListener('pointermove', move)
    target.removeEventListener('pointerup', stop)
    target.removeEventListener('pointercancel', stop)
    saveHeight(height.value)
  }
  target.addEventListener('pointermove', move)
  target.addEventListener('pointerup', stop)
  target.addEventListener('pointercancel', stop)
}

function resizeFromKeyboard(event: KeyboardEvent) {
  const step = event.key === 'ArrowUp' ? 16 : event.key === 'ArrowDown' ? -16 : 0
  if (!step) return
  event.preventDefault()
  height.value = clampHeight(height.value + step, parentHeight())
  saveHeight(height.value)
}

onMounted(() => {
  if (!host.value) return
  height.value = clampHeight(height.value, parentHeight())
  const created = new Terminal({
    allowProposedApi: false,
    cursorBlink: true,
    scrollback: 5000,
    theme: theme.value,
    fontFamily: font.value.fontFamily,
    fontSize: font.value.fontSize,
  })
  const fitAddon = new FitAddon()
  created.loadAddon(fitAddon)
  created.open(host.value)
  term = created
  fit = fitAddon
  created.attachCustomKeyEventHandler((event) => {
    const action = terminalKey(event, {
      hasSelection: created.hasSelection(),
      toggleShortcut: props.toggleShortcut ?? '',
      mac: /mac/i.test(navigator.platform),
    })
    if (action === 'copy') void navigator.clipboard.writeText(created.getSelection()).catch(ignore)
    else if (action === 'paste')
      void navigator.clipboard
        .readText()
        .then((text) => created.paste(text))
        .catch(ignore)
    return action === 'default'
  })
  created.onData((data) => {
    const id = attached
    if (!id || view.value.phase !== 'running') return
    writes = writes
      .then(() => terminalWrite(id, data))
      .catch((error) => {
        if (isGone(error)) void reconcile()
      })
  })
  created.onResize(({ cols, rows }) => {
    if (attached && view.value.phase === 'running')
      void terminalResize(attached, cols, rows).catch(ignore)
  })
  const hostObserver = new ResizeObserver(() => {
    cancelAnimationFrame(frame)
    frame = requestAnimationFrame(refit)
  })
  hostObserver.observe(host.value)
  observers = [hostObserver]
  const container = panel.value?.parentElement
  if (container) {
    const containerObserver = new ResizeObserver(() => {
      height.value = clampHeight(height.value, parentHeight())
    })
    containerObserver.observe(container)
    observers.push(containerObserver)
  }
  start(props.channelId)
})

watch(
  () => props.channelId,
  (next) => {
    detachCurrent()
    start(next)
  },
)
watch([theme, font], applyLook)

onBeforeUnmount(() => {
  generation++
  cancelAnimationFrame(frame)
  observers.forEach((observer) => observer.disconnect())
  observers = []
  const id = attached ?? activeId.value
  if (id) void terminalDetach(id).catch(ignore)
  term?.dispose()
  term = null
  fit = null
})

defineExpose({ restart, kill, view })
</script>

<template>
  <section
    ref="panel"
    data-ui-id="terminal-panel"
    data-testid="terminal-panel"
    aria-label="Terminal"
    class="relative flex shrink-0 flex-col border-t border-border/60 bg-workspace"
    :style="{ height: `${height}px` }"
  >
    <div
      data-testid="terminal-resizer"
      data-ui-id="terminal-resizer"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize terminal"
      :aria-valuenow="height"
      tabindex="0"
      class="absolute inset-x-0 -top-1 z-10 h-2 cursor-row-resize touch-none outline-none after:absolute after:inset-x-0 after:top-1 after:h-px after:bg-transparent hover:after:bg-primary/60 focus-visible:after:bg-primary"
      :class="{ 'after:bg-primary': dragging }"
      @pointerdown.prevent="startDrag"
      @keydown="resizeFromKeyboard"
    />
    <div
      data-testid="terminal-tabs"
      role="tablist"
      aria-label="Terminal tabs"
      class="flex h-8 shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border/60 px-2 text-xs"
    >
      <div
        v-for="tab in tabs"
        :key="tab.sessionId"
        class="flex h-6 shrink-0 items-center rounded hover:bg-muted"
        :class="tab.sessionId === activeId ? 'bg-muted text-foreground' : 'text-muted-foreground'"
      >
        <button
          type="button"
          data-testid="terminal-tab"
          role="tab"
          :tabindex="tab.sessionId === activeId ? 0 : -1"
          :aria-selected="tab.sessionId === activeId"
          class="h-6 rounded-l pr-1 pl-2 outline-none focus-visible:ring-1 focus-visible:ring-primary"
          @click="selectTab(tab.sessionId)"
          @keydown="moveTab($event, tab.sessionId)"
        >
          <span :class="{ 'opacity-60': !tab.running }">{{ tabName(tab.number) }}</span>
        </button>
        <button
          type="button"
          data-testid="terminal-tab-close"
          class="mr-1 rounded p-0.5 hover:bg-border"
          :aria-label="`Close ${tabName(tab.number)}`"
          @click="closeTab(tab.sessionId)"
        >
          <X class="size-3" aria-hidden="true" />
        </button>
      </div>
      <Button
        data-testid="terminal-new-tab"
        variant="ghost"
        size="icon-xs"
        aria-label="New terminal tab"
        :disabled="tabsFull"
        :title="tabsFull ? TAB_LIMIT_REASON : 'New terminal tab'"
        @click="addTab"
      >
        <Plus />
      </Button>
    </div>
    <header class="flex h-8 shrink-0 items-center gap-1.5 border-b border-border/60 px-3 text-xs">
      <SquareTerminal class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span
        data-testid="terminal-cwd"
        class="min-w-0 flex-1 truncate font-mono text-muted-foreground"
        :title="activeCwd"
      >
        {{ cwdText }}
      </span>
      <Button
        v-if="canRestart(view)"
        data-testid="terminal-restart"
        variant="ghost"
        size="xs"
        aria-label="Restart terminal"
        @click="restart"
      >
        <RotateCcw />
        Restart
      </Button>
      <Button
        v-if="canKill(view)"
        data-testid="terminal-kill"
        variant="ghost"
        size="xs"
        aria-label="Kill terminal session"
        @click="kill"
      >
        <Power />
        Kill
      </Button>
      <Button
        data-testid="terminal-hide"
        variant="ghost"
        size="icon-xs"
        aria-label="Hide terminal"
        title="Hide terminal"
        @click="emit('hide')"
      >
        <ChevronDown />
      </Button>
    </header>
    <div class="relative min-h-0 flex-1" :style="{ backgroundColor: theme.background }">
      <div ref="host" data-testid="terminal-host" class="absolute inset-0 px-2 py-1" />
      <div
        v-if="status"
        class="pointer-events-none absolute inset-x-2 bottom-1 flex items-center gap-2 rounded bg-muted/90 px-2 py-1 text-xs text-muted-foreground"
      >
        <p data-testid="terminal-status" role="status" class="min-w-0 flex-1 truncate">
          {{ status }}
        </p>
        <Button
          v-if="view.phase === 'moved'"
          data-testid="terminal-show-here"
          variant="ghost"
          size="xs"
          class="pointer-events-auto"
          @click="showHere"
        >
          Show Here
        </Button>
      </div>
    </div>
  </section>
</template>
