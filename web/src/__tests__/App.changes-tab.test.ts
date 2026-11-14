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
  live: null as { history: unknown[] } | null,
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
vi.mock('../bridge/store', async () => {
  const { reactive } = await import('vue')
  mocks.live = reactive(mocks.store)
  return { useStore: () => mocks.live }
})
vi.mock('../bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('../bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  getProjectChannels: vi.fn(async () => []),
}))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
import App from '../App.vue'
import ChangesView from '../components/changes/ChangesView.vue'
import { Tabs, TabsTrigger } from '../components/ui/tabs'
import { getProjectChannels } from '../bridge/api'

let view: ReturnType<typeof shallowMount> | undefined
function channel(kind: ProjectChannelDto['kind']): ProjectChannelDto {
  return {
    id: 'channel',
    projectId: 'project',
    name: 'channel',
    agentId: 'agent',
    workspaceMode: 'main_checkout',
    kind,
    branch: null,
    archived: false,
    archivedInto: null,
    hasMessages: false,
  }
}
const withChanges = [
  {
    id: 'm1',
    role: 'agent',
    continuesGroup: false,
    blocks: [{ type: 'diff', text: '--- a/p\n+++ b/p\n@@ -1 +1 @@\n-a\n+b' }],
  },
]
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
async function app(kind: ProjectChannelDto['kind'] | null, history: unknown[]) {
  vi.mocked(getProjectChannels).mockResolvedValue(kind ? [channel(kind)] : [])
  mocks.store.history = history
  view = shallowMount(App, {
    global: {
      renderStubDefaultSlot: true,
      stubs: {
        ResizablePanelGroup: { template: '<div><slot /></div>' },
        ResizablePanel: { template: '<div><slot /></div>' },
      },
    },
  })
  await flushPromises()
  return view
}
const triggers = (wrapper: NonNullable<typeof view>) =>
  wrapper.findAllComponents(TabsTrigger).map((item) => item.props('value'))
const tabs = (wrapper: NonNullable<typeof view>) =>
  wrapper
    .findAllComponents(Tabs)
    .find((item) => item.attributes('data-ui-id') === 'conversation-tabs')!

test.each(['general', 'text'] as const)(
  'a %s channel hides Changes until it has changes',
  async (kind) => {
    const empty = await app(kind, [])
    expect(triggers(empty)).toEqual(['conversation', 'run', 'review'])
    empty.unmount()
    const full = await app(kind, withChanges)
    expect(triggers(full)).toEqual(['conversation', 'changes', 'run', 'review'])
  },
)

test.each(['branch', 'legacy', 'archived_legacy'] as const)(
  'a %s channel keeps the Changes tab without changes',
  async (kind) => {
    expect(triggers(await app(kind, []))).toContain('changes')
  },
)

test('a channel that is not in the list yet keeps the Changes tab', async () => {
  expect(triggers(await app(null, []))).toContain('changes')
})

test('the open Changes tab returns to Conversation when the tab disappears', async () => {
  const wrapper = await app('text', withChanges)
  tabs(wrapper).vm.$emit('update:modelValue', 'changes')
  await flushPromises()
  expect(wrapper.findComponent(ChangesView).exists()).toBe(true)
  expect(wrapper.findComponent(ChangesView).props('branchless')).toBe(true)
  mocks.live!.history = []
  await flushPromises()
  expect(triggers(wrapper)).not.toContain('changes')
  expect(tabs(wrapper).props('modelValue')).toBe('conversation')
  expect(wrapper.findComponent(ChangesView).exists()).toBe(false)
})

test('ChangesView gets branchless false in a branch channel', async () => {
  const wrapper = await app('branch', [])
  tabs(wrapper).vm.$emit('update:modelValue', 'changes')
  await flushPromises()
  expect(wrapper.findComponent(ChangesView).props('branchless')).toBe(false)
})
