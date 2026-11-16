import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import type { ThemeDto } from '@/bridge/types'
import { activeTheme } from '@/bridge/theme'
import TerminalPanel from '../TerminalPanel.vue'
import { heightKey } from '../terminal-state'

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
  cwd: '/Users/dev/project',
  scrollback: '',
  running: true,
  exitCode: null,
  signal: null,
  ...overrides,
})
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
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'terminal_open' ? opened() : undefined,
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

test('it opens the channel session at the terminal size and shows the short folder', async () => {
  const view = show()
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(1)
  expect(calls('terminal_open')[0]![1]).toMatchObject({ channelId: 'c1', cols: 80, rows: 24 })
  expect(view.get('[data-testid="terminal-cwd"]').text()).toBe('~/project')
  expect(view.get('[data-testid="terminal-cwd"]').attributes('title')).toBe('/Users/dev/project')
  expect(term().focused).toBe(1)
})

test('the scrollback is drawn first, then output that arrived while it opened, in order', async () => {
  let finish!: (value: unknown) => void
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'terminal_open'
      ? new Promise((resolve) => {
          finish = resolve
        })
      : Promise.resolve(undefined),
  )
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
  expect(calls('terminal_write')[0]![1]).toMatchObject({ channelId: 'c1' })
})

test('a resize of the terminal reaches the session', async () => {
  show()
  await flushPromises()
  term().resizeHandler({ cols: 120, rows: 40 })
  expect(calls('terminal_resize')).toEqual([
    ['terminal_resize', { channelId: 'c1', cols: 120, rows: 40 }],
  ])
})

test('a shell that exits shows its code and a Restart button, and Restart starts a new session', async () => {
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
  const order = vi.mocked(invoke).mock.calls.map(([command]) => command)
  expect(order.indexOf('terminal_close')).toBeGreaterThan(-1)
  expect(order.lastIndexOf('terminal_open')).toBeGreaterThan(order.indexOf('terminal_close'))
  expect(calls('terminal_open')).toHaveLength(2)
  expect(term().resets).toBe(2)
  expect(view.find('[data-testid="terminal-status"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-kill"]').exists()).toBe(true)
})

test('a session that had already ended opens as exited with its scrollback', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'terminal_open'
      ? opened({ running: false, exitCode: 0, scrollback: bytes('done\r\n') })
      : undefined,
  )
  const view = show()
  await flushPromises()
  expect(term().text()).toBe('done\r\n')
  expect(view.find('[data-testid="terminal-restart"]').exists()).toBe(true)
})

test('Kill closes the session and offers Restart', async () => {
  const view = show()
  await flushPromises()
  await view.get('[data-testid="terminal-kill"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_close')).toEqual([['terminal_close', { channelId: 'c1' }]])
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

test('Show Here opens the terminal of the channel again and removes the notice', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'terminal_open' ? opened({ scrollback: bytes('kept') }) : undefined,
  )
  const view = show()
  await flushPromises()
  stream().onmessage({ kind: 'moved' })
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(1)
  await view.get('[data-testid="terminal-show-here"]').trigger('click')
  await flushPromises()
  expect(calls('terminal_open')).toHaveLength(2)
  expect(calls('terminal_open')[1]![1]).toMatchObject({ channelId: 'c1' })
  expect(view.find('[data-testid="terminal-status"]').exists()).toBe(false)
  expect(view.find('[data-testid="terminal-show-here"]').exists()).toBe(false)
  expect(term().text()).toBe('kept')
  // The session is shown here again, so keystrokes flow.
  term().dataHandler('y')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ channelId: 'c1', data: 'y' })
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
  vi.mocked(invoke).mockImplementation(async (command, args) =>
    command === 'terminal_open'
      ? opened({
          scrollback: bytes(
            (args as { channelId: string }).channelId === 'c2' ? 'second' : 'first',
          ),
        })
      : undefined,
  )
  const view = show()
  await flushPromises()
  expect(term().text()).toBe('first')
  await view.setProps({ channelId: 'c2' })
  await flushPromises()
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { channelId: 'c1' }]])
  expect(
    calls('terminal_open').map(([, args]) => (args as { channelId: string }).channelId),
  ).toEqual(['c1', 'c2'])
  expect(term().text()).toBe('second')
  // Input now goes to the new channel.
  term().dataHandler('y')
  await flushPromises()
  expect(calls('terminal_write')[0]![1]).toMatchObject({ channelId: 'c2' })
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
  expect(calls('terminal_detach')).toEqual([['terminal_detach', { channelId: 'c1' }]])
  expect(calls('terminal_close')).toHaveLength(0)
})

test('New Terminal Tab is visible, disabled, labelled Soon, and has no handler', async () => {
  const view = show()
  await flushPromises()
  const planned = view.get('[data-testid="terminal-new-tab"]')
  expect(planned.attributes('disabled')).toBeDefined()
  expect(planned.text()).toContain('New Terminal Tab')
  expect(planned.text()).toContain('Soon')
  expect(planned.text()).not.toContain('TODO')
  const before = vi.mocked(invoke).mock.calls.length
  await planned.trigger('click')
  expect(vi.mocked(invoke).mock.calls.length).toBe(before)
  expect(view.emitted()).not.toHaveProperty('new-tab')
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
