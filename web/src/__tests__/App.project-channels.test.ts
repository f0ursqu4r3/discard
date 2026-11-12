import { shallowMount, flushPromises } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from '../bridge/fixtures/snapshot.json'
import type { ProjectChannelDto, SnapshotDto } from '../bridge/types'
const mocks = vi.hoisted(() => ({
  store: {
    snapshot: null as SnapshotDto | null,
    history: [] as unknown[],
    refresh: vi.fn(async () => {}),
  },
}))
const catalogHandlers = new Map<string, Set<() => void>>()
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    if (!catalogHandlers.has(event)) catalogHandlers.set(event, new Set())
    catalogHandlers.get(event)!.add(handler)
    return () => catalogHandlers.get(event)?.delete(handler)
  }),
}))
vi.mock('../bridge/store', () => ({ useStore: () => mocks.store }))
vi.mock('../bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('../bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  getProjectChannels: vi.fn(async () => []),
}))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
import App from '../App.vue'
import NavigationPanel from '../components/workspace/NavigationPanel.vue'
import { getProjectChannels } from '../bridge/api'

let view: ReturnType<typeof shallowMount> | undefined
function channel(id: string, projectId = 'project'): ProjectChannelDto {
  return {
    id,
    projectId,
    name: id,
    agentId: 'agent',
    workspaceMode: 'main_checkout',
    kind: 'text',
    branch: null,
    archived: false,
    archivedInto: null,
    hasMessages: false,
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => (resolve = done))
  return { promise, resolve }
}
function shown(wrapper: ReturnType<typeof app>) {
  return (
    wrapper.getComponent(NavigationPanel).props('projectChannels') as ProjectChannelDto[]
  ).map((row) => row.id)
}
function conversationsChanged() {
  for (const handler of catalogHandlers.get('discard://conversations') ?? []) handler()
}
beforeEach(() => {
  vi.clearAllMocks()
  catalogHandlers.clear()
  mocks.store.snapshot = {
    ...structuredClone(fixture),
    projectId: 'project',
    activeProjectId: 'project',
    composerDisabled: false,
    turnRunning: false,
    liveBlocks: [],
  }
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

test('a refresh keeps the shown channels while it loads and swaps the list once', async () => {
  vi.mocked(getProjectChannels).mockResolvedValueOnce([channel('one'), channel('two')])
  const wrapper = app()
  await flushPromises()
  expect(shown(wrapper)).toEqual(['one', 'two'])

  const next = deferred<ProjectChannelDto[]>()
  vi.mocked(getProjectChannels).mockReturnValueOnce(next.promise)
  conversationsChanged()
  await flushPromises()
  expect(shown(wrapper)).toEqual(['one', 'two'])

  next.resolve([channel('two'), channel('three')])
  await flushPromises()
  expect(shown(wrapper)).toEqual(['two', 'three'])
})

test('a refresh for another project clears the list at once', async () => {
  vi.mocked(getProjectChannels).mockResolvedValueOnce([channel('one')])
  const wrapper = app()
  await flushPromises()
  expect(shown(wrapper)).toEqual(['one'])

  const next = deferred<ProjectChannelDto[]>()
  vi.mocked(getProjectChannels).mockReturnValueOnce(next.promise)
  mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'other' }
  conversationsChanged()
  await flushPromises()
  expect(shown(wrapper)).toEqual([])

  next.resolve([channel('elsewhere', 'other')])
  await flushPromises()
  expect(shown(wrapper)).toEqual(['elsewhere'])
})

test('a stale response does not replace a newer list', async () => {
  const wrapper = app()
  await flushPromises()
  const older = deferred<ProjectChannelDto[]>()
  const newer = deferred<ProjectChannelDto[]>()
  vi.mocked(getProjectChannels)
    .mockReturnValueOnce(older.promise)
    .mockReturnValueOnce(newer.promise)
  conversationsChanged()
  conversationsChanged()
  await flushPromises()

  newer.resolve([channel('new')])
  await flushPromises()
  older.resolve([channel('old')])
  await flushPromises()
  expect(shown(wrapper)).toEqual(['new'])
})

test('a response for a project that is no longer shown is dropped', async () => {
  const wrapper = app()
  await flushPromises()
  const late = deferred<ProjectChannelDto[]>()
  vi.mocked(getProjectChannels).mockReturnValueOnce(late.promise)
  conversationsChanged()
  await flushPromises()
  mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'other' }
  late.resolve([channel('stale')])
  await flushPromises()
  expect(shown(wrapper)).toEqual([])
})

test('selecting a channel does not reload the project channels', async () => {
  vi.mocked(getProjectChannels).mockResolvedValue([channel('one'), channel('two')])
  const wrapper = app()
  await flushPromises()
  vi.mocked(getProjectChannels).mockClear()

  wrapper.getComponent(NavigationPanel).vm.$emit('selectChannel', 'two')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'two' })
  expect(getProjectChannels).not.toHaveBeenCalled()
  expect(shown(wrapper)).toEqual(['one', 'two'])
})
