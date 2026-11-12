import { shallowMount, flushPromises } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { toast } from 'vue-sonner'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from '../bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '../bridge/types'
const mocks = vi.hoisted(() => ({
  store: {
    snapshot: null as SnapshotDto | null,
    history: [] as unknown[],
    refresh: vi.fn(async () => {}),
  },
}))
const handlers = new Map<string, Set<() => void>>()
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    if (!handlers.has(event)) handlers.set(event, new Set())
    handlers.get(event)!.add(handler)
    return () => handlers.get(event)?.delete(handler)
  }),
}))
vi.mock('../bridge/store', () => ({ useStore: () => mocks.store }))
vi.mock('../bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('../bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  getProjectChannels: vi.fn(async () => []),
}))
vi.mock('vue-sonner', () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn() }),
}))
import App from '../App.vue'
import ChannelHeader from '../components/ChannelHeader.vue'
import TerminalPanel from '../components/terminal/TerminalPanel.vue'
import { NO_PROJECT_REASON, visibleKey } from '../components/terminal/terminal-state'

let view: ReturnType<typeof shallowMount> | undefined
function snapshot(projectId: string | null): SnapshotDto {
  return {
    ...structuredClone(fixture),
    channelId: projectId ? 'branch-channel' : 'direct-channel',
    projectId: projectId ?? '',
    activeProjectId: projectId,
    composerDisabled: false,
    turnRunning: false,
    liveBlocks: [],
  }
}
function app() {
  view = shallowMount(App, {
    global: {
      renderStubDefaultSlot: true,
      stubs: {
        ResizablePanelGroup: { template: '<div><slot /></div>' },
        ResizablePanel: { template: '<div><slot /></div>' },
      },
    },
  })
  return view
}
function toggleFromMenu() {
  for (const handler of handlers.get('discard://toggle-terminal') ?? []) handler()
}
const header = (wrapper: ReturnType<typeof app>) => wrapper.getComponent(ChannelHeader)
const panel = (wrapper: ReturnType<typeof app>) => wrapper.findComponent(TerminalPanel)

beforeEach(() => {
  vi.clearAllMocks()
  handlers.clear()
  localStorage.clear()
  mocks.store.snapshot = snapshot('project')
  mocks.store.history = []
  vi.mocked(invoke).mockResolvedValue(undefined)
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  )
})
afterEach(() => {
  view?.unmount()
  vi.unstubAllGlobals()
})

test('the terminal starts hidden and the menu action shows and hides it for the channel', async () => {
  const wrapper = app()
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(false)
  expect(header(wrapper).props('terminalVisible')).toBe(false)

  toggleFromMenu()
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(true)
  expect(panel(wrapper).props('channelId')).toBe('branch-channel')
  expect(header(wrapper).props('terminalVisible')).toBe(true)
  expect(localStorage.getItem(visibleKey)).toBe('true')

  toggleFromMenu()
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(false)
  expect(localStorage.getItem(visibleKey)).toBe('false')
})

test('the header button and the Hide button use the same state', async () => {
  const wrapper = app()
  await flushPromises()
  header(wrapper).vm.$emit('toggleTerminal')
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(true)
  panel(wrapper).vm.$emit('hide')
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(false)
  expect(localStorage.getItem(visibleKey)).toBe('false')
})

test('a saved visible state shows the terminal at startup', async () => {
  localStorage.setItem(visibleKey, 'true')
  const wrapper = app()
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(true)
})

test('a direct message disables the control with a reason and never shows a terminal', async () => {
  localStorage.setItem(visibleKey, 'true')
  mocks.store.snapshot = snapshot(null)
  const wrapper = app()
  await flushPromises()
  expect(header(wrapper).props('terminalDisabledReason')).toBe(NO_PROJECT_REASON)
  expect(panel(wrapper).exists()).toBe(false)

  toggleFromMenu()
  await flushPromises()
  expect(panel(wrapper).exists()).toBe(false)
  expect(toast).toHaveBeenCalledWith(NO_PROJECT_REASON)
  // The saved window state does not change when the toggle is refused.
  expect(localStorage.getItem(visibleKey)).toBe('true')
})

test('a project channel enables the control', async () => {
  const wrapper = app()
  await flushPromises()
  expect(header(wrapper).props('terminalDisabledReason')).toBe('')
})

test('the shortcut text of Toggle Terminal reaches the header from the keymap', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'settings_keymap_rows'
      ? [{ id: 'toggle_terminal', chord: 'Ctrl+`', bound: true }]
      : undefined,
  )
  const wrapper = app()
  await flushPromises()
  expect(header(wrapper).props('terminalShortcut')).toBe('Ctrl+`')
})
