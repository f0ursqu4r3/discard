import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  actionShortcut,
  decodeBytes,
  matchesDisplayChord,
  terminalClose,
  terminalCreate,
  terminalDetach,
  terminalKill,
  terminalOpen,
  terminalResize,
  terminalRestart,
  terminalTabs,
  terminalWrite,
  useActionShortcut,
  type TerminalEvent,
} from '../terminal'

const mocks = vi.hoisted(() => ({
  channels: [] as { onmessage: (event: unknown) => void }[],
}))
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

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockClear()
  mocks.channels.length = 0
})

test('open passes the session ID, the size, and a stream that delivers events in order', async () => {
  const opened = {
    sessionId: 's1',
    number: 1,
    cwd: '/work',
    scrollback: '',
    running: true,
    exitCode: null,
    signal: null,
  }
  vi.mocked(invoke).mockResolvedValue(opened)
  const heard: TerminalEvent[] = []
  await expect(terminalOpen('s1', 100, 30, (event) => heard.push(event))).resolves.toEqual(opened)
  const [command, args] = vi.mocked(invoke).mock.calls[0]!
  expect(command).toBe('terminal_open')
  expect(args).toMatchObject({ sessionId: 's1', cols: 100, rows: 30 })
  expect((args as { onEvent: unknown }).onEvent).toBe(mocks.channels[0])
  mocks.channels[0]!.onmessage({ kind: 'output', data: 'YQ==' })
  mocks.channels[0]!.onmessage({ kind: 'exit', exitCode: 0, signal: null })
  expect(heard.map((event) => event.kind)).toEqual(['output', 'exit'])
})

test('create and restart pass a stream and exact arguments', async () => {
  vi.mocked(invoke).mockResolvedValue({})
  await terminalCreate('channel-1', 100, 30, () => {})
  await terminalRestart('s1', 90, 20, () => {})
  const [create, restart] = vi.mocked(invoke).mock.calls
  expect(create![0]).toBe('terminal_create')
  expect(create![1]).toEqual({
    channelId: 'channel-1',
    cols: 100,
    rows: 30,
    onEvent: mocks.channels[0],
  })
  expect(restart![0]).toBe('terminal_restart')
  expect(restart![1]).toEqual({ sessionId: 's1', cols: 90, rows: 20, onEvent: mocks.channels[1] })
})

test('the other commands use exact names and camel case arguments', async () => {
  vi.mocked(invoke).mockResolvedValue(null)
  await terminalTabs('c')
  await terminalWrite('s', 'ls\n')
  await terminalResize('s', 80, 24)
  await terminalDetach('s')
  await terminalClose('s')
  await terminalKill('s')
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['terminal_tabs', { channelId: 'c' }],
    ['terminal_write', { sessionId: 's', data: 'ls\n' }],
    ['terminal_resize', { sessionId: 's', cols: 80, rows: 24 }],
    ['terminal_detach', { sessionId: 's' }],
    ['terminal_close', { sessionId: 's' }],
    ['terminal_kill', { sessionId: 's' }],
  ])
})

test('a backend error reaches the caller as a rejection', async () => {
  vi.mocked(invoke).mockRejectedValue('Direct messages have no terminal.')
  await expect(terminalOpen('dm', 80, 24, () => {})).rejects.toBe(
    'Direct messages have no terminal.',
  )
})

test('bytes survive base64 decoding, so a UTF-8 sequence can span two chunks', () => {
  const euro = new TextEncoder().encode('€')
  const first = decodeBytes(btoa(String.fromCharCode(...euro.slice(0, 2))))
  const second = decodeBytes(btoa(String.fromCharCode(...euro.slice(2))))
  expect([...first, ...second]).toEqual([...euro])
  const decoder = new TextDecoder()
  expect(decoder.decode(first, { stream: true }) + decoder.decode(second, { stream: true })).toBe(
    '€',
  )
})

test('actionShortcut reads the shown shortcut and ignores an unbound action', async () => {
  vi.mocked(invoke).mockResolvedValue([
    { id: 'toggle_terminal', chord: 'Ctrl+`', bound: true },
    { id: 'zoom', chord: 'Not set', bound: false },
  ])
  await expect(actionShortcut('toggle_terminal')).resolves.toBe('Ctrl+`')
  await expect(actionShortcut('zoom')).resolves.toBe('')
  await expect(actionShortcut('missing')).resolves.toBe('')
})

test('useActionShortcut loads the shortcut and reloads when the keymap changes', async () => {
  let rows = [{ id: 'toggle_terminal', chord: 'Ctrl+`', bound: true }]
  vi.mocked(invoke).mockImplementation(async () => rows)
  let changed: () => void = () => {}
  vi.mocked(listen).mockImplementation(async (_event, handler) => {
    changed = handler as () => void
    return () => {}
  })
  const view = mount(
    defineComponent({
      setup() {
        const shortcut = useActionShortcut('toggle_terminal')
        return () => h('span', shortcut.value)
      },
    }),
  )
  await vi.waitFor(() => expect(view.text()).toBe('Ctrl+`'))
  rows = [{ id: 'toggle_terminal', chord: 'Cmd+J', bound: true }]
  changed()
  await vi.waitFor(() => expect(view.text()).toBe('Cmd+J'))
  expect(listen).toHaveBeenCalledWith('discard://shortcuts', expect.any(Function))
  view.unmount()
})

test('useActionShortcut stays empty when the bridge is absent', async () => {
  vi.mocked(invoke).mockRejectedValue(new Error('no bridge'))
  const view = mount(
    defineComponent({
      setup() {
        const shortcut = useActionShortcut('toggle_terminal')
        return () => h('span', shortcut.value)
      },
    }),
  )
  await Promise.resolve()
  expect(view.text()).toBe('')
  view.unmount()
})

test('a displayed shortcut matches its key event and no other', () => {
  const backtick = new KeyboardEvent('keydown', { key: '`', ctrlKey: true })
  expect(matchesDisplayChord('Ctrl+`', backtick)).toBe(true)
  expect(matchesDisplayChord('Ctrl+`', new KeyboardEvent('keydown', { key: '`' }))).toBe(false)
  expect(
    matchesDisplayChord(
      'Ctrl+`',
      new KeyboardEvent('keydown', { key: '`', ctrlKey: true, shiftKey: true }),
    ),
  ).toBe(false)
  expect(
    matchesDisplayChord(
      'Cmd+Shift+K',
      new KeyboardEvent('keydown', { key: 'K', metaKey: true, shiftKey: true }),
    ),
  ).toBe(true)
  expect(
    matchesDisplayChord('Cmd++', new KeyboardEvent('keydown', { key: '+', metaKey: true })),
  ).toBe(true)
  expect(matchesDisplayChord('', backtick)).toBe(false)
})
