import { onMounted, onUnmounted, ref, type Ref } from 'vue'
import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** What the backend sends on the stream of a terminal session. */
export type TerminalEvent =
  | { kind: 'output'; data: string }
  | { kind: 'exit'; exitCode: number | null; signal: string | null }
  | { kind: 'moved' }
  | { kind: 'closed'; reason: string }

export interface TerminalOpened {
  sessionId: string
  cwd: string
  /** Output kept so far, base64. */
  scrollback: string
  running: boolean
  exitCode: number | null
  signal: string | null
}

export interface TerminalStatus {
  sessionId: string
  cwd: string
  running: boolean
  exitCode: number | null
  signal: string | null
}

/** Decode base64 output to raw bytes. xterm decodes UTF-8 across chunk borders. */
export function decodeBytes(data: string): Uint8Array {
  const text = atob(data)
  const bytes = new Uint8Array(text.length)
  for (let index = 0; index < text.length; index++) bytes[index] = text.charCodeAt(index)
  return bytes
}

/**
 * Open or re-attach the terminal of a channel. `onEvent` replaces the earlier listener of the session.
 * Events can arrive before the returned promise settles, so callers keep them until they draw the scrollback.
 */
export function terminalOpen(
  channelId: string,
  cols: number,
  rows: number,
  onEvent: (event: TerminalEvent) => void,
): Promise<TerminalOpened> {
  const stream = new Channel<TerminalEvent>()
  stream.onmessage = onEvent
  return invoke<TerminalOpened>('terminal_open', { channelId, cols, rows, onEvent: stream })
}

export function terminalWrite(channelId: string, data: string): Promise<void> {
  return invoke('terminal_write', { channelId, data })
}

export function terminalResize(channelId: string, cols: number, rows: number): Promise<void> {
  return invoke('terminal_resize', { channelId, cols, rows })
}

/** Stop the stream. The session keeps running and keeps its output. */
export function terminalDetach(channelId: string): Promise<void> {
  return invoke('terminal_detach', { channelId })
}

/** Kill the shell and drop the session. */
export function terminalClose(channelId: string): Promise<void> {
  return invoke('terminal_close', { channelId })
}

export function terminalStatus(channelId: string): Promise<TerminalStatus | null> {
  return invoke<TerminalStatus | null>('terminal_status', { channelId })
}

/** The shortcut text of an action, as the Keybindings page shows it. Empty when the action has none. */
export async function actionShortcut(actionId: string): Promise<string> {
  const rows = await invoke<{ id: string; chord: string; bound: boolean }[]>('settings_keymap_rows')
  const row = rows.find((entry) => entry.id === actionId)
  return row?.bound ? row.chord : ''
}

/** The shown shortcut of an action. It follows keymap changes. Empty when the action has none or the bridge is absent. */
export function useActionShortcut(actionId: string): Ref<string> {
  const shortcut = ref('')
  let stop: UnlistenFn | undefined
  let disposed = false
  async function reload() {
    try {
      const value = await actionShortcut(actionId)
      if (!disposed) shortcut.value = value
    } catch {
      /* The bridge may be absent in a browser preview. */
    }
  }
  onMounted(async () => {
    void reload()
    try {
      const unlisten = await listen('discard://shortcuts', reload)
      if (disposed) unlisten()
      else stop = unlisten
    } catch {
      /* The bridge may be absent in a browser preview. */
    }
  })
  onUnmounted(() => {
    disposed = true
    stop?.()
  })
  return shortcut
}

/** Match a displayed shortcut such as `Ctrl+\`` or `Cmd+Shift+K` to a keyboard event. */
export function matchesDisplayChord(display: string, event: KeyboardEvent): boolean {
  if (!display) return false
  const plus = display.endsWith('++')
  const parts = (plus ? display.slice(0, -2) : display).split('+')
  const key = plus ? '+' : parts.pop()!
  const mods = new Set(parts)
  return (
    event.key.toLowerCase() === key.toLowerCase() &&
    event.ctrlKey === mods.has('Ctrl') &&
    event.metaKey === mods.has('Cmd') &&
    event.altKey === mods.has('Alt') &&
    event.shiftKey === mods.has('Shift')
  )
}
