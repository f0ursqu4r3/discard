import { matchesDisplayChord } from '@/bridge/terminal'
import { isWebShortcut } from '@/bridge/shortcuts'

/**
 * What a key does while the terminal has focus.
 * - `default`: the shell gets the key.
 * - `app`: the terminal ignores the key, so the app or the system handles it.
 * - `copy`: copy the selection.
 * - `paste`: paste the clipboard.
 */
export type TerminalKey = 'default' | 'app' | 'copy' | 'paste'

export function terminalKey(
  event: KeyboardEvent,
  options: { hasSelection: boolean; toggleShortcut: string; mac: boolean },
): TerminalKey {
  if (event.type !== 'keydown') return 'default'
  if (matchesDisplayChord(options.toggleShortcut, event) || isWebShortcut(event)) return 'app'
  const key = event.key.toLowerCase()
  if (options.mac) {
    if (!event.metaKey) return 'default'
    if (key === 'c' && !event.shiftKey && !event.altKey && !event.ctrlKey && options.hasSelection)
      return 'copy'
    // Every other command key belongs to the app or the system. Cmd+V pastes through the paste event.
    return 'app'
  }
  // Ctrl+C must reach the shell, so copy and paste use Ctrl+Shift.
  if (event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey) {
    if (key === 'c') return options.hasSelection ? 'copy' : 'default'
    if (key === 'v') return 'paste'
  }
  return 'default'
}
