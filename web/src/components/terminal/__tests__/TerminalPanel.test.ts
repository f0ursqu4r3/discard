import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import type { ThemeDto } from '@/bridge/types'
import { activeTheme } from '@/bridge/theme'
import TerminalPanel from '../TerminalPanel.vue'
import { TAB_LIMIT_REASON, heightKey, rememberActive } from '../terminal-state'

const mocks = vi.hoisted(() => {
  class FakeTerminal {
    static instances: FakeTerminal[] = []
    options: Record<string, unknown>
    cols = 80
    rows = 24
    written: Uint8Array[] = []
    resets = 0
    focused = 0
    disposed = false
    selection = ''
    pasted: string[] = []
    dataHandler: (data: string) => void = () => {}
    resizeHandler: (size: { cols: number; rows: number }) => void = () => {}
    keyHandler: (event: KeyboardEvent) => boolean = () => true
    constructor(options: Record<string, unknown>) {
      this.options = { ...options }
      FakeTerminal.instances.push(this)
    }
    loadAddon() {}
    open() {}
    write(bytes: Uint8Array) {
      this.written.push(bytes)
    }
    reset() {
      this.resets++
      this.written = []
    }
    focus() {
      this.focused++
    }
    dispose() {
      this.disposed = true
    }
    hasSelection() {
      return this.selection !== ''
    }
    getSelection() {
      return this.selection
    }
    paste(text: string) {
      this.pasted.push(text)
    }
    onData(handler: (data: string) => void) {
      this.dataHandler = handler
    }
    onResize(handler: (size: { cols: number; rows: number }) => void) {
      this.resizeHandler = handler
    }
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
      this.keyHandler = handler
    }
    text() {
      return this.written.map((bytes) => new TextDecoder().decode(bytes)).join('')
    }
  }
  return { FakeTerminal, channels: [] as { onmessage: (event: unknown) => void }[] }
})
vi.mock('@xterm/xterm', () => ({ Terminal: mocks.FakeTerminal }))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit() {}
  },
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage: (event: unknown) => void = () => {}
    constructor() {
      mocks.channels.push(this)
    }
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@/bridge/shortcuts', () => ({ isWebShortcut: () => false }))

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
const bytes = (text: string) => btoa(text)
const opened = (overrides: Record<string, unknown> = {}) => ({
  sessionId: 's1',
  number: 1,
  cwd: '/Users/dev/project',
  scrollback: '',
  running: true,
  exitCode: null,
  signal: null,
  ...overrides,
})
const tab = (sessionId: string, number: number, overrides: Record<string, unknown> = {}) => ({
  sessionId,
  number,
  cwd: '/Users/dev/project',
  running: true,
  exitCode: null,
  signal: null,
  ...overrides,
})
const numberOf = (sessionId: string) => Number(sessionId.replace(/\D/g, '')) || 1
// The backend double: it answers the tab commands. Tests replace the tab list or single answers.
const backend = {
  tabs: [] as ReturnType<typeof tab>[],
  created: 0,
  answers: {} as Record<string, (args: Record<string, unknown>) => unknown>,
}
async function answer(command: string, args?: Record<string, unknown>) {
  const custom = backend.answers[command]
  if (custom) return custom(args ?? {})
  const id = String(args?.sessionId ?? 's1')
  if (command === 'terminal_tabs') return backend.tabs
  if (command === 'terminal_create') {
    backend.created++
    const sessionId = `n${backend.created}`
    return opened({ sessionId, number: backend.tabs.length + backend.created })
  }
  if (command === 'terminal_open') return opened({ sessionId: id, number: numberOf(id) })
  if (command === 'terminal_restart') return opened({ sessionId: `${id}r`, number: numberOf(id) })
  return undefined
}
const calls = (name: string) => vi.mocked(invoke).mock.calls.filter(([command]) => command === name)
const term = () => mocks.FakeTerminal.instances.at(-1)!
const views: ReturnType<typeof mount>[] = []
function show(props: Record<string, unknown> = {}) {
  const view = mount(TerminalPanel, {
    props: { channelId: 'c1', ...props },
    attachTo: document.body,
  })
  views.push(view)
  return view
}
// jsdom has no layout. This gives every element the height of the main panel that holds the terminal.
function mainPanelHeight(height: number) {
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get: () => height,
  })
}
function stream(index = -1) {
  return mocks.channels.at(index)!
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub)
  vi.mocked(invoke).mockReset()
  backend.tabs = [tab('s1', 1)]
  backend.created = 0
  backend.answers = {}
  // The window remembers the active tab per channel. Each test starts without it.
  for (const channel of ['c1', 'c2']) rememberActive(channel, '')
  vi.mocked(invoke).mockImplementation(async (command, args) =>
    answer(command, args as Record<string, unknown>),
  )
  mocks.FakeTerminal.instances.length = 0
  mocks.channels.length = 0
  localStorage.clear()
  activeTheme.value = null
})
afterEach(() => {
  Reflect.deleteProperty(HTMLElement.prototype, 'clientHeight')
  views.splice(0).forEach((view) => view.unmount())
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

test('it opens the tab at the terminal size and shows the short folder', async () => {
  const view = show()
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(1)
  expect(calls('terminal_open')[0]![1]).toMatchObject({ sessionId: 's1', cols: 80, rows: 24 })
  expect(view.get('[data-testid="terminal-cwd"]').text()).toBe('~/project')
  expect(view.get('[data-testid="terminal-cwd"]').attributes('title')).toBe('/Users/dev/project')
  expect(term().focused).toBe(1)
})

test('the scrollback is drawn first, then output that arrived while it opened, in order', async () => {
  let finish!: (value: unknown) => void
  backend.answers.terminal_open = () =>
    new Promise((resolve) => {
      finish = resolve
    })
  show()
  await flushPromises()
  // Output can reach the page before the open call returns.
  stream().onmessage({ kind: 'output', data: bytes('live ') })
  stream().onmessage({ kind: 'output', data: bytes('output') })
  expect(term().text()).toBe('')
  finish(opened({ scrollback: bytes('old ') }))
  await flushPromises()
  expect(term().text()).toBe('old live output')
  stream().onmessage({ kind: 'output', data: bytes('!') })
  expect(term().text()).toBe('old live output!')
})

test('output is written as raw bytes so a split UTF-8 sequence stays whole', async () => {
  show()
  await flushPromises()
  const euro = new TextEncoder().encode('€')
  stream().onmessage({ kind: 'output', data: btoa(String.fromCharCode(...euro.slice(0, 2))) })
  stream().onmessage({ kind: 'output', data: btoa(String.fromCharCode(...euro.slice(2))) })
  expect(term().written.every((chunk) => chunk instanceof Uint8Array)).toBe(true)
  expect([...term().written.flatMap((chunk) => [...chunk])]).toEqual([...euro])
})

test('keystrokes go to the shell one call at a time and in order', async () => {
  show()
  await flushPromises()
  term().dataHandler('l')
  term().dataHandler('s')
  term().dataHandler('\r')
  await flushPromises()
  expect(calls('terminal_write').map(([, args]) => (args as { data: string }).data)).toEqual([
    'l',
    's',
    '\r',
  ])
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 's1' })
})

test('a resize of the terminal reaches the session', async () => {
  show()
  await flushPromises()
  term().resizeHandler({ cols: 120, rows: 40 })
  expect(calls('terminal_resize')).toEqual([
    ['terminal_resize', { sessionId: 's1', cols: 120, rows: 40 }],
  ])
})

test('a shell that exits shows its code and a Restart button, and Restart starts a new shell in the tab', async () => {
  const view = show()
  await flushPromises()
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(true)
  stream().onmessage({ kind: 'exit', exitCode: 2, signal: null })
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toBe('The shell exited with code 2.')
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(false)
  // Typing after the exit goes nowhere.
  term().dataHandler('x')
  await flushPromises()
  expect(calls('terminal_write')).toHaveLength(0)

  await view.get('[data-testid="terminal-restart"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_restart')).toHaveLength(1)
  expect(calls('terminal_restart')[0]![1]).toMatchObject({ sessionId: 's1', cols: 80, rows: 24 })
  expect(calls('terminal_close')).toHaveLength(0)
  expect(term().resets).toBe(2)
  expect(view.find('[data-testid="terminal-status"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(true)
})

test('a session that had already ended opens as exited with its scrollback', async () => {
  backend.answers.terminal_open = () =>
    opened({ running: false, exitCode: 0, scrollback: bytes('done\r\n') })
  const view = show()
  await flushPromises()
  expect(term().text()).toBe('done\r\n')
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('Kill calls terminal_kill and keeps the tab as closed', async () => {
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-kill"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_kill')).toEqual([['terminal_kill', { sessionId: 's1' }]])
  expect(calls('terminal_close')).toHaveLength(0)
  expect(view.findAll('[data-testid="terminal-tab"]')).toHaveLength(1)
  expect(view.get('[data-testid="terminal-status"]').text()).toBe('The session was killed.')
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('a close from the app, such as a removed folder, shows its reason', async () => {
  const view = show()
  await flushPromises()
  stream().onmessage({
    kind: 'closed',
    reason: 'The worktree of this terminal was removed. The session ended.',
  })
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toContain(
    'worktree of this terminal was removed',
  )
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('a moved event shows the notice and Show Here, and no keystroke or resize leaves the panel', async () => {
  const view = show()
  await flushPromises()
  stream().onmessage({ kind: 'moved' })
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toBe(
    'This terminal is open in another window.',
  )
  expect(view.get('[data-testid="terminal-show-here"]').text()).toBe('Show Here')
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(false)
  term().dataHandler('x')
  term().resizeHandler({ cols: 100, rows: 30 })
  await flushPromises()
  expect(calls('terminal_write')).toHaveLength(0)
  expect(calls('terminal_resize')).toHaveLength(0)
})

test('a moved terminal offers only Show Here: no Kill and no Restart, whatever the phase before', async () => {
  const view = show()
  await flushPromises()
  // From running.
  stream().onmessage({ kind: 'moved' })
  await flushPromises()
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-show-here"]').exists()).toBe(true)
  // From an ended session, which would offer Restart.
  stream().onmessage({ kind: 'exited', exitCode: 0, signal: null })
  await flushPromises()
  stream().onmessage({ kind: 'moved' })
  await flushPromises()
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(false)
  expect(calls('terminal_close')).toHaveLength(0)
})

test('Show Here opens the tab again and removes the notice', async () => {
  backend.answers.terminal_open = () => opened({ scrollback: bytes('kept') })
  const view = show()
  await flushPromises()
  stream().onmessage({ kind: 'moved' })
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(1)
  await view.get('[data-testid="terminal-show-here"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(2)
  expect(calls('terminal_open')[1]![1]).toMatchObject({ sessionId: 's1' })
  expect(view.find('[data-testid="terminal-status"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-show-here"]').exists()).toBe(false)
  expect(term().text()).toBe('kept')
  // The session is shown here again, so keystrokes flow.
  term().dataHandler('y')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 's1', data: 'y' })
})

test('an open that fails shows the error and offers Restart', async () => {
  vi.mocked(invoke).mockRejectedValue('Could not start the shell /bin/zsh')
  const view = show()
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toBe(
    'Could not start the shell /bin/zsh',
  )
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('a channel switch detaches the old session, clears the view, and replays the new scrollback', async () => {
  backend.answers.terminal_tabs = (args) => [
    tab((args.channelId as string) === 'c2' ? 's9' : 's1', 1),
  ]
  backend.answers.terminal_open = (args) =>
    opened({
      sessionId: args.sessionId,
      scrollback: bytes(args.sessionId === 's9' ? 'second' : 'first'),
    })
  const view = show()
  await flushPromises()
  expect(term().text()).toBe('first')
  await view.setProps({ channelId: 'c2' })
  await flushPromises()
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { sessionId: 's1' }]])
  expect(
    calls('terminal_open').map(([, args]) => (args as { sessionId: string }).sessionId),
  ).toEqual(['s1', 's9'])
  expect(term().text()).toBe('second')
  // Input now goes to the new channel.
  term().dataHandler('y')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 's9' })
  expect(mocks.FakeTerminal.instances).toHaveLength(1)
})

test('output from a stream that a newer open replaced is dropped', async () => {
  const view = show()
  await flushPromises()
  const old = stream()
  await view.setProps({ channelId: 'c2' })
  await flushPromises()
  old.onmessage({ kind: 'output', data: bytes('stale') })
  expect(term().text()).not.toContain('stale')
})

test('hiding detaches from the session without killing it and disposes xterm', async () => {
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-hide"]').trigger('click')
  expect(view.emitted('hide')).toHaveLength(1)
  view.unmount()
  views.pop()
  expect(term().disposed).toBe(true)
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { sessionId: 's1' }]])
  expect(calls('terminal_close')).toHaveLength(0)
})

const tabsOf = (view: ReturnType<typeof mount>) => view.findAll('[data-testid="terminal-tab"]')
const closeOf = (view: ReturnType<typeof mount>) =>
  view.findAll('[data-testid="terminal-tab-close"]')
const eight = () => Array.from({ length: 8 }, (_, index) => tab(`s${index + 1}`, index + 1))

test('it lists the tabs of the channel and opens the remembered one', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 4)]
  const first = show()
  await flushPromises()
  expect(first.find('[data-testid="terminal-tabs"]').exists()).toBe(true)
  expect(tabsOf(first).map((entry) => entry.text())).toEqual([
    'Terminal 1',
    'Terminal 2',
    'Terminal 4',
  ])
  expect(calls('terminal_open')[0]![1]).toMatchObject({ sessionId: 's1' })
  await tabsOf(first)[2]!.trigger('click')
  await flushPromises()
  first.unmount()
  views.pop()
  vi.mocked(invoke).mockClear()
  const second = show()
  await flushPromises()
  expect(calls('terminal_open')[0]![1]).toMatchObject({ sessionId: 's3' })
  expect(tabsOf(second)[2]!.attributes('aria-selected')).toBe('true')
  expect(tabsOf(second)[0]!.attributes('aria-selected')).toBe('false')
  expect(calls('terminal_create')).toHaveLength(0)
})

test('a channel without tabs starts one with terminal_create', async () => {
  backend.tabs = []
  const view = show()
  await flushPromises()
  expect(calls('terminal_create')).toHaveLength(1)
  expect(calls('terminal_create')[0]![1]).toMatchObject({ channelId: 'c1', cols: 80, rows: 24 })
  expect(calls('terminal_open')).toHaveLength(0)
  expect(tabsOf(view)).toHaveLength(1)
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
})

test('+ creates a tab and makes it active', async () => {
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-new-tab"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_create')).toHaveLength(1)
  expect(calls('terminal_create')[0]![1]).toMatchObject({ channelId: 'c1' })
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { sessionId: 's1' }]])
  expect(tabsOf(view)).toHaveLength(2)
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('false')
  term().dataHandler('z')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 'n1', data: 'z' })
})

test('+ is disabled with the limit reason at 8 tabs', async () => {
  backend.tabs = eight()
  const view = show()
  await flushPromises()
  const add = view.get('[data-testid="terminal-new-tab"]')
  expect(add.attributes('disabled')).toBeDefined()
  expect(add.attributes('title')).toBe(TAB_LIMIT_REASON)
  await add.trigger('click')
  expect(calls('terminal_create')).toHaveLength(0)
})

test('a backend refusal of the ninth tab shows its message', async () => {
  backend.answers.terminal_create = () => Promise.reject('A channel can have 8 terminals.')
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-new-tab"]').trigger('click')
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toBe('A channel can have 8 terminals.')
  expect(tabsOf(view)).toHaveLength(1)
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
  // The kept tab shows again and takes input.
  expect(calls('terminal_open').at(-1)![1]).toMatchObject({ sessionId: 's1' })
  term().dataHandler('k')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 's1' })
})

test('switching tabs detaches the old session and opens the new one', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  backend.answers.terminal_open = (args) =>
    opened({ sessionId: args.sessionId, scrollback: bytes(`out-${args.sessionId}`) })
  const view = show()
  await flushPromises()
  expect(term().text()).toBe('out-s1')
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  const order = vi
    .mocked(invoke)
    .mock.calls.filter(([command]) => command === 'terminal_detach' || command === 'terminal_open')
    .map(([command, args]) => `${command}:${(args as { sessionId: string }).sessionId}`)
  expect(order).toEqual(['terminal_open:s1', 'terminal_detach:s1', 'terminal_open:s2'])
  expect(term().text()).toBe('out-s2')
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(view.get('[data-testid="terminal-cwd"]').text()).toBe('~/project')
})

test('closing an inactive tab keeps the active stream', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  const view = show()
  await flushPromises()
  await closeOf(view)[1]!.trigger('click')
  await flushPromises()
  expect(calls('terminal_close')).toEqual([['terminal_close', { sessionId: 's2' }]])
  expect(calls('terminal_detach')).toHaveLength(0)
  expect(calls('terminal_open')).toHaveLength(1)
  expect(tabsOf(view)).toHaveLength(1)
  stream().onmessage({ kind: 'output', data: bytes('still live') })
  expect(term().text()).toBe('still live')
})

test('closing the active tab opens its neighbour', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 3)]
  const view = show()
  await flushPromises()
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  await closeOf(view)[1]!.trigger('click')
  await flushPromises()
  expect(calls('terminal_close')).toEqual([['terminal_close', { sessionId: 's2' }]])
  expect(calls('terminal_open').at(-1)![1]).toMatchObject({ sessionId: 's3' })
  expect(tabsOf(view).map((entry) => entry.text())).toEqual(['Terminal 1', 'Terminal 3'])
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(view.emitted('hide')).toBeUndefined()
})

test('closing the last tab emits hide', async () => {
  const view = show()
  await flushPromises()
  await closeOf(view)[0]!.trigger('click')
  await flushPromises()
  expect(calls('terminal_close')).toEqual([['terminal_close', { sessionId: 's1' }]])
  expect(view.emitted('hide')).toHaveLength(1)
  expect(tabsOf(view)).toHaveLength(0)
})

test('Restart calls terminal_restart and keeps the tab in place', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 3)]
  const view = show()
  await flushPromises()
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  stream().onmessage({ kind: 'exit', exitCode: 0, signal: null })
  await flushPromises()
  await view.get('[data-testid="terminal-restart"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_restart')[0]![1]).toMatchObject({ sessionId: 's2' })
  expect(tabsOf(view).map((entry) => entry.text())).toEqual([
    'Terminal 1',
    'Terminal 2',
    'Terminal 3',
  ])
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  // The tab takes the new session id.
  term().dataHandler('r')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 's2r' })
})

test('a failed restart shows the error and keeps the tab', async () => {
  backend.answers.terminal_restart = () => Promise.reject('Could not start the shell /bin/zsh')
  const view = show()
  await flushPromises()
  stream().onmessage({ kind: 'exit', exitCode: 1, signal: null })
  await flushPromises()
  await view.get('[data-testid="terminal-restart"]').trigger('click')
  await flushPromises()
  expect(view.get('[data-testid="terminal-status"]').text()).toBe(
    'Could not start the shell /bin/zsh',
  )
  expect(tabsOf(view)).toHaveLength(1)
  expect(tabsOf(view)[0]!.text()).toBe('Terminal 1')
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('a channel switch detaches only the active tab and recalls the remembered tab on return', async () => {
  backend.answers.terminal_tabs = (args) =>
    args.channelId === 'c1' ? [tab('s1', 1), tab('s2', 2)] : [tab('s7', 1)]
  const view = show()
  await flushPromises()
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  vi.mocked(invoke).mockClear()
  await view.setProps({ channelId: 'c2' })
  await flushPromises()
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { sessionId: 's2' }]])
  expect(calls('terminal_open')[0]![1]).toMatchObject({ sessionId: 's7' })
  vi.mocked(invoke).mockClear()
  await view.setProps({ channelId: 'c1' })
  await flushPromises()
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { sessionId: 's7' }]])
  expect(calls('terminal_open')[0]![1]).toMatchObject({ sessionId: 's2' })
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(calls('terminal_close')).toHaveLength(0)
})

test('the app theme sets colors, font, and size, and a theme change restyles the live terminal', async () => {
  const theme = (background: string, fontScale: number): ThemeDto => ({
    mode: 'dark',
    tokens: {},
    radius: null,
    fontScale,
    spacingScale: 1,
    protectedBg: '#000000',
    protectedFg: '#ffffff',
    protectedBorder: '#ffffff',
    monoFont: 'Iosevka Term',
    terminal: {
      palette: Array.from({ length: 16 }, () => '#abcdef'),
      background,
      foreground: '#ffffff',
      cursor: '#ffffff',
      cursorAccent: '#000000',
      selectionBackground: '#444444',
      selectionForeground: null,
    },
  })
  activeTheme.value = theme('#101010', 1)
  show()
  await flushPromises()
  expect(term().options.theme).toMatchObject({ background: '#101010', red: '#abcdef' })
  expect(String(term().options.fontFamily)).toContain('Iosevka Term')
  expect(term().options.fontSize).toBe(13)
  activeTheme.value = theme('#202020', 1.5)
  await flushPromises()
  expect(term().options.theme).toMatchObject({ background: '#202020' })
  expect(term().options.fontSize).toBe(20)
})

test('a key handler gives the shell its keys and the app its own', async () => {
  show({ toggleShortcut: 'Ctrl+`' })
  await flushPromises()
  expect(term().keyHandler(new KeyboardEvent('keydown', { key: 'a' }))).toBe(true)
  expect(term().keyHandler(new KeyboardEvent('keydown', { key: 'c', ctrlKey: true }))).toBe(true)
  expect(term().keyHandler(new KeyboardEvent('keydown', { key: '`', ctrlKey: true }))).toBe(false)
})

test('copy writes the selection to the clipboard and Ctrl+Shift+V pastes through xterm', async () => {
  const writeText = vi.fn(async () => {})
  const readText = vi.fn(async () => 'pasted')
  vi.stubGlobal('navigator', {
    ...navigator,
    platform: 'Linux x86_64',
    clipboard: { writeText, readText },
  })
  show()
  await flushPromises()
  term().selection = 'chosen'
  expect(
    term().keyHandler(new KeyboardEvent('keydown', { key: 'C', ctrlKey: true, shiftKey: true })),
  ).toBe(false)
  expect(writeText).toHaveBeenCalledWith('chosen')
  expect(
    term().keyHandler(new KeyboardEvent('keydown', { key: 'V', ctrlKey: true, shiftKey: true })),
  ).toBe(false)
  await flushPromises()
  expect(term().pasted).toEqual(['pasted'])
})

test('dragging the top edge resizes within the limits and saves the height', async () => {
  mainPanelHeight(1000)
  const view = show()
  await flushPromises()
  const panel = view.get('[data-testid="terminal-panel"]')
  const handle = view.get('[data-testid="terminal-resizer"]')
  expect(panel.attributes('style')).toContain('height: 260px')
  const down = new MouseEvent('pointerdown', { clientY: 600, bubbles: true, cancelable: true })
  handle.element.dispatchEvent(down)
  handle.element.dispatchEvent(new MouseEvent('pointermove', { clientY: 500, bubbles: true }))
  await flushPromises()
  expect(panel.attributes('style')).toContain('height: 360px')
  handle.element.dispatchEvent(new MouseEvent('pointermove', { clientY: -900, bubbles: true }))
  await flushPromises()
  expect(panel.attributes('style')).toContain('height: 700px')
  handle.element.dispatchEvent(new MouseEvent('pointermove', { clientY: 5000, bubbles: true }))
  await flushPromises()
  expect(panel.attributes('style')).toContain('height: 120px')
  handle.element.dispatchEvent(new MouseEvent('pointerup', { bubbles: true }))
  expect(localStorage.getItem(heightKey)).toBe('120')
})

test('the saved height is restored and the arrow keys move the edge', async () => {
  mainPanelHeight(1000)
  localStorage.setItem(heightKey, '300')
  const view = show()
  await flushPromises()
  const panel = view.get('[data-testid="terminal-panel"]')
  expect(panel.attributes('style')).toContain('height: 300px')
  await view.get('[data-testid="terminal-resizer"]').trigger('keydown', { key: 'ArrowUp' })
  expect(panel.attributes('style')).toContain('height: 316px')
  expect(localStorage.getItem(heightKey)).toBe('316')
})

test('Enter on a close button closes the tab and does not select it', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  const view = show()
  await flushPromises()
  const close = closeOf(view)[1]!
  const press = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
  close.element.dispatchEvent(press)
  // The browser turns the unprevented key into a click.
  expect(press.defaultPrevented).toBe(false)
  await close.trigger('click')
  await flushPromises()
  expect(calls('terminal_close')).toEqual([['terminal_close', { sessionId: 's2' }]])
  expect(calls('terminal_detach')).toHaveLength(0)
  expect(calls('terminal_open')).toHaveLength(1)
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
})

test('tabs use a roving tabindex and the arrow keys move the active tab', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 3)]
  const view = show()
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.attributes('tabindex'))).toEqual(['0', '-1', '-1'])
  await tabsOf(view)[0]!.trigger('keydown', { key: 'ArrowRight' })
  await flushPromises()
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(tabsOf(view).map((entry) => entry.attributes('tabindex'))).toEqual(['-1', '0', '-1'])
  await tabsOf(view)[1]!.trigger('keydown', { key: 'ArrowLeft' })
  await flushPromises()
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
  await tabsOf(view)[0]!.trigger('keydown', { key: 'ArrowLeft' })
  await flushPromises()
  expect(tabsOf(view)[2]!.attributes('aria-selected')).toBe('true')
  expect(calls('terminal_open').at(-1)![1]).toMatchObject({ sessionId: 's3' })
})

test('a new tab whose create was superseded still joins the strip', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  let finish!: (value: unknown) => void
  backend.answers.terminal_create = () =>
    new Promise((resolve) => {
      finish = resolve
    })
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-new-tab"]').trigger('click')
  await flushPromises()
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  finish(opened({ sessionId: 'n1', number: 3 }))
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.text())).toEqual([
    'Terminal 1',
    'Terminal 2',
    'Terminal 3',
  ])
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(tabsOf(view)[2]!.attributes('aria-selected')).toBe('false')
  expect(calls('terminal_detach')).toContainEqual(['terminal_detach', { sessionId: 'n1' }])
})

test('arrow keys keep focus on the tab strip and do not focus xterm', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 3)]
  const view = show()
  await flushPromises()
  const focusedAfterStart = term().focused
  const buttons = () => tabsOf(view).map((entry) => entry.element as HTMLElement)
  buttons()[0]!.focus()
  await tabsOf(view)[0]!.trigger('keydown', { key: 'ArrowRight' })
  await flushPromises()
  expect(document.activeElement).toBe(buttons()[1])
  await tabsOf(view)[1]!.trigger('keydown', { key: 'ArrowRight' })
  await flushPromises()
  expect(tabsOf(view)[2]!.attributes('aria-selected')).toBe('true')
  expect(document.activeElement).toBe(buttons()[2])
  expect(term().focused).toBe(focusedAfterStart)
})

const ENDED = 'This terminal session has ended.'

test('after an app-side close, inactive ghost tabs disappear after the reload', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  const view = show()
  await flushPromises()
  backend.tabs = []
  stream().onmessage({ kind: 'closed', reason: 'The worktree of this terminal was removed.' })
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.text())).toEqual(['Terminal 1'])
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('Restart after an app-side close starts a new shell with terminal_create in the old place', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  const view = show()
  await flushPromises()
  backend.answers.terminal_restart = () => Promise.reject(ENDED)
  backend.answers.terminal_open = () => Promise.reject(ENDED)
  backend.tabs = []
  stream().onmessage({ kind: 'closed', reason: 'The worktree of this terminal was removed.' })
  await flushPromises()
  await view.get('[data-testid="terminal-restart"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_create')).toHaveLength(1)
  expect(tabsOf(view)).toHaveLength(1)
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
  term().dataHandler('q')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ sessionId: 'n1' })
  expect(view.find('[data-testid="terminal-status"]').exists()).toBe(false)
})

test('opening a tab that the backend dropped reloads the list', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  const view = show()
  await flushPromises()
  backend.answers.terminal_open = () => Promise.reject(ENDED)
  backend.tabs = []
  await tabsOf(view)[1]!.trigger('click')
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.text())).toEqual(['Terminal 2'])
  expect(view.get('[data-testid="terminal-status"]').text()).toBe(ENDED)
})

test('a refused close keeps the tab and shows the message', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  backend.answers.terminal_close = () => Promise.reject('This terminal is open in another window.')
  const view = show()
  await flushPromises()
  await closeOf(view)[1]!.trigger('click')
  await flushPromises()
  expect(tabsOf(view)).toHaveLength(2)
  expect(view.get('[data-testid="terminal-status"]').text()).toBe(
    'This terminal is open in another window.',
  )
  await closeOf(view)[0]!.trigger('click')
  await flushPromises()
  expect(tabsOf(view)).toHaveLength(2)
  expect(view.emitted('hide')).toBeUndefined()
})

test('a close of a tab that already ended removes it', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2)]
  backend.answers.terminal_close = () => Promise.reject(ENDED)
  const view = show()
  await flushPromises()
  await closeOf(view)[1]!.trigger('click')
  await flushPromises()
  expect(tabsOf(view)).toHaveLength(1)
})

test('a tab the person selects during a close is not overridden', async () => {
  backend.tabs = [tab('s1', 1), tab('s2', 2), tab('s3', 3)]
  let finish!: (value: unknown) => void
  backend.answers.terminal_close = () =>
    new Promise((resolve) => {
      finish = resolve
    })
  const view = show()
  await flushPromises()
  await closeOf(view)[0]!.trigger('click')
  await tabsOf(view)[2]!.trigger('click')
  await flushPromises()
  finish(undefined)
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.text())).toEqual(['Terminal 2', 'Terminal 3'])
  expect(tabsOf(view)[1]!.attributes('aria-selected')).toBe('true')
  expect(calls('terminal_open').at(-1)![1]).toMatchObject({ sessionId: 's3' })
})

test('a refused new tab reloads the list and shows tabs from other windows', async () => {
  backend.answers.terminal_create = () => Promise.reject('A channel can have 8 terminals.')
  const view = show()
  await flushPromises()
  backend.tabs = [tab('s1', 1), tab('s5', 5, { running: false })]
  await view.get('[data-testid="terminal-new-tab"]').trigger('click')
  await flushPromises()
  expect(tabsOf(view).map((entry) => entry.text())).toEqual(['Terminal 1', 'Terminal 5'])
  expect(tabsOf(view)[0]!.attributes('aria-selected')).toBe('true')
  expect(view.get('[data-testid="terminal-status"]').text()).toBe('A channel can have 8 terminals.')
})
