import { ref, watch, type Ref } from 'vue'
import type { TerminalEvent, TerminalOpened } from '@/bridge/terminal'

export const MIN_HEIGHT = 120
export const MAX_HEIGHT_RATIO = 0.7
export const DEFAULT_HEIGHT = 260
export const visibleKey = 'discard:terminal-visible'
export const heightKey = 'discard:terminal-height'

/** The reason shown on the disabled toggle of a channel that has no project. */
export const NO_PROJECT_REASON =
  'Direct messages have no terminal. Open a project channel to use the terminal.'

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // The terminal keeps working for this window without saved layout.
  }
}

export function loadVisible(): boolean {
  return read(visibleKey) === 'true'
}

export function saveVisible(visible: boolean) {
  write(visibleKey, String(visible))
}

/** Keep the height between the minimum and 70% of the main panel. */
export function clampHeight(height: number, container: number): number {
  const max = Math.max(MIN_HEIGHT, Math.floor(container * MAX_HEIGHT_RATIO))
  if (!Number.isFinite(height)) return Math.min(DEFAULT_HEIGHT, max)
  return Math.min(max, Math.max(MIN_HEIGHT, Math.round(height)))
}

export function loadHeight(): number {
  const value = Number(read(heightKey))
  return Number.isFinite(value) && value > 0 ? value : DEFAULT_HEIGHT
}

export function saveHeight(height: number) {
  write(heightKey, String(Math.round(height)))
}

/**
 * Whether the terminal can open for the shown page and channel. A blocked state has a reason and is not a planned
 * feature: a project channel on the conversation page enables it.
 */
export function terminalAvailability(
  state:
    | { channelId?: string | null; activeProjectId?: string | null; page?: string }
    | null
    | undefined,
): { available: boolean; reason: string } {
  if (!state?.channelId) return { available: false, reason: 'Open a channel to use the terminal.' }
  if (!state.activeProjectId) return { available: false, reason: NO_PROJECT_REASON }
  if (state.page && state.page !== 'conversation')
    return { available: false, reason: 'Open a channel conversation to use the terminal.' }
  return { available: true, reason: '' }
}

/** The terminal visibility of this window. It is not per channel. Each change is saved. */
export function useTerminalVisibility(): { visible: Ref<boolean>; toggle: () => void } {
  const visible = ref(loadVisible())
  watch(visible, saveVisible)
  return { visible, toggle: () => (visible.value = !visible.value) }
}

export type Phase = 'connecting' | 'running' | 'exited' | 'closed' | 'failed' | 'moved'

/** The notice of a window whose session another window took. */
export const MOVED_NOTICE = 'This terminal is open in another window.'

export interface SessionView {
  phase: Phase
  cwd: string
  exitCode: number | null
  signal: string | null
  /** Why the app closed the session, or why it could not start. */
  message: string
}

export const connecting = (): SessionView => ({
  phase: 'connecting',
  cwd: '',
  exitCode: null,
  signal: null,
  message: '',
})

/** The view after `terminal_open` returned. */
export function opened(view: SessionView, result: TerminalOpened): SessionView {
  return {
    ...view,
    phase: result.running ? 'running' : 'exited',
    cwd: result.cwd,
    exitCode: result.exitCode,
    signal: result.signal,
    message: '',
  }
}

/** The view after `terminal_open` failed. */
export function failed(view: SessionView, error: unknown): SessionView {
  return { ...view, phase: 'failed', message: String(error) }
}

/** The view after a stream event. Output changes nothing. */
export function applyEvent(view: SessionView, event: TerminalEvent): SessionView {
  if (event.kind === 'exit')
    return { ...view, phase: 'exited', exitCode: event.exitCode, signal: event.signal }
  if (event.kind === 'closed') return { ...view, phase: 'closed', message: event.reason }
  if (event.kind === 'moved') return { ...view, phase: 'moved' }
  return view
}

/** A started shell can restart when it ended or the app closed it. */
export function canRestart(view: SessionView): boolean {
  return view.phase === 'exited' || view.phase === 'closed' || view.phase === 'failed'
}

export function canKill(view: SessionView): boolean {
  return view.phase === 'running'
}

/** One short line for the status strip. */
export function statusLine(view: SessionView): string {
  if (view.phase === 'exited') {
    if (view.signal) return `The shell stopped (${view.signal}).`
    return view.exitCode === null
      ? 'The shell exited.'
      : `The shell exited with code ${view.exitCode}.`
  }
  if (view.phase === 'closed' || view.phase === 'failed') return view.message
  if (view.phase === 'moved') return MOVED_NOTICE
  return ''
}
