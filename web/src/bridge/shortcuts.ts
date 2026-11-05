import { onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { runAction } from './api'
type Binding = [string, string]
/** Match a core chord to a browser keyboard event. */
export function matchesChord(chord: string, event: KeyboardEvent, mac: boolean): boolean {
  const minus = chord.endsWith('--')
  const parts = (minus ? chord.slice(0, -2) : chord).split('-')
  const key = minus ? '-' : parts.pop()!
  const mods = new Set(parts)
  const cmd = mods.has('cmd')
  const ctrl = mods.has('ctrl') || (cmd && !mac)
  const meta = cmd && mac
  const names: Record<string, string> = {
    escape: 'escape',
    space: ' ',
    up: 'arrowup',
    down: 'arrowdown',
    left: 'arrowleft',
    right: 'arrowright',
  }
  return (
    event.key.toLowerCase() === (names[key] ?? key) &&
    event.ctrlKey === ctrl &&
    event.metaKey === meta &&
    event.altKey === mods.has('alt') &&
    event.shiftKey === mods.has('shift')
  )
}
/** Register shortcuts that native menus cannot carry. */
export function useWebShortcuts() {
  let bindings: Binding[] = []
  let unlisten: UnlistenFn | undefined
  let changed = false
  const keydown = (event: KeyboardEvent) => {
    if (event.repeat || event.isComposing) return
    const match = bindings.find(([, chord]) =>
      matchesChord(chord, event, /mac/i.test(navigator.platform)),
    )
    if (match) {
      event.preventDefault()
      void runAction(match[0])
    }
  }
  onMounted(async () => {
    window.addEventListener('keydown', keydown)
    try {
      unlisten = await listen<Binding[]>('discard://shortcuts', (event) => {
        changed = true
        bindings = event.payload
      })
      const initial = await invoke<Binding[]>('web_shortcuts')
      if (!changed) bindings = initial
    } catch {
      /* The bridge may be absent in a browser preview. */
    }
  })
  onUnmounted(() => {
    window.removeEventListener('keydown', keydown)
    unlisten?.()
  })
}
