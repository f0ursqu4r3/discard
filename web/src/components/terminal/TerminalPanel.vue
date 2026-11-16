<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { ChevronDown, Plus, Power, RotateCcw, SquareTerminal } from '@lucide/vue'
import { Button } from '../ui/button'
import { activeTheme } from '@/bridge/theme'
import { shortPath } from '@/settings/paths'
import {
  decodeBytes,
  terminalClose,
  terminalDetach,
  terminalOpen,
  terminalResize,
  terminalWrite,
  type TerminalEvent,
} from '@/bridge/terminal'
import { terminalFont, terminalTheme } from './terminal-theme'
import { terminalKey } from './terminal-keys'
import {
  applyEvent,
  canKill,
  canRestart,
  clampHeight,
  connecting,
  failed,
  loadHeight,
  opened,
  saveHeight,
  statusLine,
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
const height = ref(loadHeight())
const dragging = ref(false)

let term: Terminal | null = null
let fit: FitAddon | null = null
let observers: ResizeObserver[] = []
// The channel whose session this view shows. Input and resize go there.
let attached: string | null = null
let generation = 0
// Opens run one at a time, so a later open of a channel always replaces the stream of an earlier one.
let queue: Promise<unknown> = Promise.resolve()
// Keystrokes go out one call at a time, so the shell sees them in order.
let writes: Promise<unknown> = Promise.resolve()
let frame = 0

const status = computed(() => statusLine(view.value))
const cwdText = computed(() => (view.value.cwd ? shortPath(view.value.cwd) : ''))
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

function deliver(event: TerminalEvent) {
  if (event.kind === 'output') term?.write(decodeBytes(event.data))
  else view.value = applyEvent(view.value, event)
}

async function attach(channelId: string, run: number) {
  const current = term
  if (!current || run !== generation) return
  current.reset()
  view.value = connecting()
  let early: TerminalEvent[] | null = []
  refit()
  try {
    const result = await terminalOpen(channelId, current.cols, current.rows, (event) => {
      if (early) early.push(event)
      else if (run === generation) deliver(event)
    })
    if (run !== generation) {
      // A newer attach took over. Another channel must stop streaming here.
      if (props.channelId !== channelId) void terminalDetach(channelId).catch(ignore)
      return
    }
    attached = channelId
    const bytes = decodeBytes(result.scrollback)
    if (bytes.length) current.write(bytes)
    view.value = opened(view.value, result)
    const pending = early
    early = null
    pending.forEach(deliver)
    current.focus()
  } catch (error) {
    if (run === generation) view.value = failed(view.value, error)
  }
}

function start(channelId: string) {
  const run = ++generation
  attached = null
  queue = queue.then(() => attach(channelId, run)).catch(ignore)
}

// Opening again moves the session to this window. The other window gets the moved notice.
function showHere() {
  start(props.channelId)
}

async function restart() {
  const id = props.channelId
  generation++
  attached = null
  await terminalClose(id).catch(ignore)
  start(id)
}

async function kill() {
  const id = props.channelId
  generation++
  attached = null
  await terminalClose(id).catch(ignore)
  view.value = { ...view.value, phase: 'closed', message: 'The session was killed.' }
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
    writes = writes.then(() => terminalWrite(id, data)).catch(ignore)
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
  (next, previous) => {
    if (previous) void terminalDetach(previous).catch(ignore)
    start(next)
  },
)
watch([theme, font], applyLook)

onBeforeUnmount(() => {
  generation++
  cancelAnimationFrame(frame)
  observers.forEach((observer) => observer.disconnect())
  observers = []
  const id = attached ?? props.channelId
  void terminalDetach(id).catch(ignore)
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
    <header class="flex h-8 shrink-0 items-center gap-1.5 border-b border-border/60 px-3 text-xs">
      <SquareTerminal class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span
        data-testid="terminal-cwd"
        class="min-w-0 flex-1 truncate font-mono text-muted-foreground"
        :title="view.cwd"
      >
        {{ cwdText }}
      </span>
      <!-- TODO: Add several terminal sessions per channel with a tab strip. The backend holds one session per channel. -->
      <Button
        data-testid="terminal-new-tab"
        variant="ghost"
        size="xs"
        disabled
        aria-label="New Terminal Tab (Soon)"
        title="New Terminal Tab · Soon"
      >
        <Plus />
        <span class="hidden sm:inline">New Terminal Tab · Soon</span>
      </Button>
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
