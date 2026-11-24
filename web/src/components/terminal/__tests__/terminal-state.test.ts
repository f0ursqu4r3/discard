import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { defineComponent, h, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import {
  DEFAULT_HEIGHT,
  MAX_TABS,
  MIN_HEIGHT,
  NO_PROJECT_REASON,
  TAB_LIMIT_REASON,
  applyEvent,
  canAddTab,
  canKill,
  canRestart,
  clampHeight,
  connecting,
  failed,
  heightKey,
  loadHeight,
  loadVisible,
  nextActive,
  opened,
  pickActive,
  recallActive,
  rememberActive,
  saveHeight,
  saveVisible,
  statusLine,
  tabName,
  terminalAvailability,
  useTerminalVisibility,
  visibleKey,
} from '../terminal-state'

beforeEach(() => localStorage.clear())

test('visibility is off by default and survives a reload', () => {
  expect(loadVisible()).toBe(false)
  saveVisible(true)
  expect(localStorage.getItem(visibleKey)).toBe('true')
  expect(loadVisible()).toBe(true)
  saveVisible(false)
  expect(loadVisible()).toBe(false)
})

test('useTerminalVisibility restores the saved state and saves each toggle', async () => {
  saveVisible(true)
  let api!: ReturnType<typeof useTerminalVisibility>
  mount(
    defineComponent({
      setup() {
        api = useTerminalVisibility()
        return () => h('div')
      },
    }),
  )
  expect(api.visible.value).toBe(true)
  api.toggle()
  await nextTick()
  expect(localStorage.getItem(visibleKey)).toBe('false')
  api.toggle()
  await nextTick()
  expect(localStorage.getItem(visibleKey)).toBe('true')
})

test('storage that throws does not break visibility or height', () => {
  const broken = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('blocked')
  })
  const blockedWrite = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('blocked')
  })
  expect(loadVisible()).toBe(false)
  expect(loadHeight()).toBe(DEFAULT_HEIGHT)
  expect(() => saveVisible(true)).not.toThrow()
  expect(() => saveHeight(300)).not.toThrow()
  broken.mockRestore()
  blockedWrite.mockRestore()
})

test('the height is saved rounded and read back, and a bad value falls back', () => {
  expect(loadHeight()).toBe(DEFAULT_HEIGHT)
  saveHeight(311.6)
  expect(localStorage.getItem(heightKey)).toBe('312')
  expect(loadHeight()).toBe(312)
  localStorage.setItem(heightKey, 'wide')
  expect(loadHeight()).toBe(DEFAULT_HEIGHT)
  localStorage.setItem(heightKey, '-5')
  expect(loadHeight()).toBe(DEFAULT_HEIGHT)
})

test('the height stays between 120 px and 70 percent of the main panel', () => {
  expect(clampHeight(50, 1000)).toBe(MIN_HEIGHT)
  expect(clampHeight(400, 1000)).toBe(400)
  expect(clampHeight(900, 1000)).toBe(700)
  // A small main panel never pushes the maximum below the minimum.
  expect(clampHeight(500, 100)).toBe(MIN_HEIGHT)
  expect(clampHeight(Number.NaN, 1000)).toBe(DEFAULT_HEIGHT)
})

test('a direct message disables the terminal with a reason, and a project channel enables it', () => {
  expect(terminalAvailability({ channelId: 'dm', activeProjectId: null })).toEqual({
    available: false,
    reason: NO_PROJECT_REASON,
  })
  expect(terminalAvailability({ channelId: 'c', activeProjectId: 'p' })).toEqual({
    available: true,
    reason: '',
  })
  expect(
    terminalAvailability({ channelId: 'c', activeProjectId: 'p', page: 'conversation' }).available,
  ).toBe(true)
})

test('no snapshot, no channel, or another page blocks the terminal with a reason', () => {
  for (const state of [
    null,
    undefined,
    { channelId: null, activeProjectId: 'p' },
    { channelId: 'c', activeProjectId: 'p', page: 'project-settings' },
  ]) {
    const result = terminalAvailability(state)
    expect(result.available).toBe(false)
    expect(result.reason).not.toBe('')
  }
})

test('a session goes from connecting to running, and an exit offers a restart', () => {
  let view = connecting()
  expect(view.phase).toBe('connecting')
  expect(canRestart(view)).toBe(false)
  view = opened(view, {
    sessionId: 's',
    number: 1,
    cwd: '/work',
    scrollback: '',
    running: true,
    exitCode: null,
    signal: null,
  })
  expect(view).toMatchObject({ phase: 'running', cwd: '/work' })
  expect(canKill(view)).toBe(true)
  expect(canRestart(view)).toBe(false)
  expect(applyEvent(view, { kind: 'output', data: 'YQ==' })).toBe(view)
  view = applyEvent(view, { kind: 'exit', exitCode: 3, signal: null })
  expect(view.phase).toBe('exited')
  expect(canRestart(view)).toBe(true)
  expect(canKill(view)).toBe(false)
  expect(statusLine(view)).toBe('The shell exited with code 3.')
})

test('a session that ended before the panel attached opens as exited', () => {
  const view = opened(connecting(), {
    sessionId: 's',
    number: 1,
    cwd: '/work',
    scrollback: '',
    running: false,
    exitCode: null,
    signal: 'Hangup',
  })
  expect(view.phase).toBe('exited')
  expect(statusLine(view)).toBe('The shell stopped (Hangup).')
})

test('a moved session shows the notice and cannot restart or be killed from this window', () => {
  const running = opened(connecting(), {
    sessionId: 's',
    number: 1,
    cwd: '/work',
    scrollback: '',
    running: true,
    exitCode: null,
    signal: null,
  })
  const view = applyEvent(running, { kind: 'moved' })
  expect(view.phase).toBe('moved')
  expect(statusLine(view)).toBe('This terminal is open in another window.')
  expect(canRestart(view)).toBe(false)
  expect(canKill(view)).toBe(false)
})

test('a closed or failed session shows its reason and can restart', () => {
  const closed = applyEvent(connecting(), { kind: 'closed', reason: 'The folder was removed.' })
  expect(closed.phase).toBe('closed')
  expect(canRestart(closed)).toBe(true)
  expect(statusLine(closed)).toBe('The folder was removed.')
  const broken = failed(connecting(), 'Could not start the shell /bin/zsh')
  expect(broken.phase).toBe('failed')
  expect(canRestart(broken)).toBe(true)
  expect(statusLine(broken)).toBe('Could not start the shell /bin/zsh')
})

test('tab names and the tab limit', () => {
  expect(tabName(2)).toBe('Terminal 2')
  expect(MAX_TABS).toBe(8)
  expect(TAB_LIMIT_REASON).toBe('A channel can have 8 terminals.')
  expect(canAddTab(7)).toBe(true)
  expect(canAddTab(8)).toBe(false)
})

test('pickActive prefers the remembered tab, then the first, then none', () => {
  const tabs = [{ sessionId: 'a' }, { sessionId: 'b' }]
  expect(pickActive(tabs, 'b')).toBe('b')
  expect(pickActive(tabs, 'gone')).toBe('a')
  expect(pickActive(tabs)).toBe('a')
  expect(pickActive([], 'b')).toBeNull()
})

test('nextActive picks the right neighbour, then the left, then none', () => {
  const tabs = [{ sessionId: 'a' }, { sessionId: 'b' }, { sessionId: 'c' }]
  expect(nextActive(tabs, 'b')).toBe('c')
  expect(nextActive(tabs, 'c')).toBe('b')
  expect(nextActive([{ sessionId: 'a' }], 'a')).toBeNull()
})

test('the active tab is remembered per channel', () => {
  rememberActive('c1', 's1')
  rememberActive('c2', 's2')
  expect(recallActive('c1')).toBe('s1')
  expect(recallActive('c2')).toBe('s2')
  expect(recallActive('c3')).toBeUndefined()
})
