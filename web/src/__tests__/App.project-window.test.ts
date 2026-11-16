import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from '../bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '../bridge/types'
const mocks = vi.hoisted(() => ({
  store: { snapshot: null as SnapshotDto | null, history: [], refresh: vi.fn(async () => {}) },
  label: 'main',
  selectProject: vi.fn(async () => ''),
  getRailLayout: vi.fn(async () => ({ items: [] })),
  getProjects: vi.fn(async () => []),
  getConversations: vi.fn(async () => []),
  openProjectWindow: vi.fn(async () => {}),
  showMainWindow: vi.fn(async () => {}),
  switchChannel: vi.fn(async () => {}),
  error: vi.fn(),
}))
type Handler = (event: { payload?: unknown }) => void
const listeners: { name: string; handler: Handler; options?: unknown }[] = []
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: Handler, options?: unknown) => {
    const entry = { name, handler, options }
    listeners.push(entry)
    return () => listeners.splice(listeners.indexOf(entry), 1)
  }),
}))
vi.mock('../bridge/window', () => ({ currentWindowLabel: () => mocks.label }))
vi.mock('../bridge/store', () => ({ useStore: () => mocks.store }))
vi.mock('../bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('../bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: mocks.getProjects,
  getProjectChannels: vi.fn(async () => []),
  getConversations: mocks.getConversations,
  getRailLayout: mocks.getRailLayout,
  selectProject: mocks.selectProject,
  switchChannel: mocks.switchChannel,
  openProjectWindow: mocks.openProjectWindow,
  showMainWindow: mocks.showMainWindow,
}))
vi.mock('vue-sonner', () => ({
  toast: Object.assign(vi.fn(), { error: mocks.error, success: vi.fn() }),
}))
import App from '../App.vue'
import NavigationPanel from '../components/workspace/NavigationPanel.vue'
import ServerRail from '../components/ServerRail.vue'
import UserBar from '../components/workspace/UserBar.vue'

const project = { id: 'p1', name: 'Project one', path: '/one' }
const window = { kind: 'WebviewWindow' }
const wrappers: ReturnType<typeof shallowMount>[] = []
beforeEach(() => {
  vi.clearAllMocks()
  listeners.length = 0
  localStorage.clear()
  mocks.label = 'main'
  mocks.store.snapshot = structuredClone(fixture) as SnapshotDto
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  )
})
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  vi.unstubAllGlobals()
  location.hash = ''
})
function app(hash: string, mountPanel = false) {
  location.hash = hash
  const wrapper = shallowMount(
    App,
    mountPanel ? { global: { stubs: { NavigationPanel: false } } } : {},
  )
  wrappers.push(wrapper)
  return wrapper
}
function emit(name: string, payload?: unknown) {
  const entries = listeners.filter((entry) => entry.name === name)
  expect(entries.length).toBeGreaterThan(0)
  for (const entry of entries) entry.handler({ payload })
}
const projectMode = () => app('#/project/p1')

test('a project window mounts no project rail, direct messages entry, or Add Project control', async () => {
  const wrapper = app('#/project/p1', true)
  await flushPromises()
  const panel = wrapper.getComponent(NavigationPanel)
  expect(panel.props('rail')).toBe(false)
  expect(wrapper.findComponent(ServerRail).exists()).toBe(false)
  expect(wrapper.findComponent(UserBar).exists()).toBe(true)
  expect(wrapper.find('[data-ui-id="project-rail"]').exists()).toBe(false)
  expect(wrapper.find('[data-ui-id="direct-messages-button"]').exists()).toBe(false)
})

test('the main window mounts the project rail', async () => {
  const wrapper = app('', true)
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('rail')).toBe(true)
  expect(wrapper.findComponent(ServerRail).exists()).toBe(true)
})

test('a project window does not load the rail layout and still loads the projects', async () => {
  const wrapper = projectMode()
  await flushPromises()
  expect(mocks.getRailLayout).not.toHaveBeenCalled()
  expect(mocks.getProjects).toHaveBeenCalled()
  expect(wrapper.getComponent(NavigationPanel).props('railLayout')).toBeNull()
})

test('the main window loads the rail layout', async () => {
  app('')
  await flushPromises()
  expect(mocks.getRailLayout).toHaveBeenCalled()
})

test('Agents in a project window shows the main window and keeps the page', async () => {
  const wrapper = projectMode()
  await flushPromises()
  const panel = wrapper.getComponent(NavigationPanel)
  panel.vm.$emit('action', 'show_friends')
  await flushPromises()
  expect(mocks.showMainWindow).toHaveBeenCalledWith('direct-messages')
  expect(panel.props('page')).toBe('conversation')
})

test('a direct messages navigation in a project window shows the main window and keeps the page', async () => {
  localStorage.setItem('discard:last-direct-message', 'dm-1')
  const wrapper = projectMode()
  await flushPromises()
  const panel = wrapper.getComponent(NavigationPanel)
  panel.vm.$emit('navigate', 'direct-messages')
  await flushPromises()
  expect(mocks.showMainWindow).toHaveBeenCalledWith('direct-messages')
  expect(mocks.getConversations).not.toHaveBeenCalled()
  expect(mocks.switchChannel).not.toHaveBeenCalled()
  expect(panel.props('page')).toBe('conversation')
})

test('an agent profile event with no agent shows the main window in a project window', async () => {
  const wrapper = projectMode()
  await flushPromises()
  emit('discard://agent-profile', '')
  await flushPromises()
  expect(mocks.showMainWindow).toHaveBeenCalledWith('direct-messages')
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('conversation')
})

test('other pages open inside a project window', async () => {
  const wrapper = projectMode()
  await flushPromises()
  const panel = wrapper.getComponent(NavigationPanel)
  panel.vm.$emit('navigate', 'overview')
  await flushPromises()
  expect(panel.props('page')).toBe('overview')
  expect(mocks.showMainWindow).not.toHaveBeenCalled()
})

test('the main window opens direct messages in place', async () => {
  const wrapper = app('')
  await flushPromises()
  wrapper.getComponent(NavigationPanel).vm.$emit('action', 'show_friends')
  await flushPromises()
  expect(mocks.showMainWindow).not.toHaveBeenCalled()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
})

test('Open in New Window opens the project window and keeps the selection', async () => {
  const wrapper = app('')
  await flushPromises()
  wrapper.getComponent(NavigationPanel).vm.$emit('projectAction', 'open-window', project)
  await flushPromises()
  expect(mocks.openProjectWindow).toHaveBeenCalledWith('p1')
  expect(mocks.selectProject).not.toHaveBeenCalled()
  expect(mocks.store.refresh).not.toHaveBeenCalled()
})

test('a failed Open in New Window shows an error', async () => {
  mocks.openProjectWindow.mockRejectedValueOnce(new Error('no window'))
  const wrapper = app('')
  await flushPromises()
  wrapper.getComponent(NavigationPanel).vm.$emit('projectAction', 'open-window', project)
  await flushPromises()
  expect(mocks.error).toHaveBeenCalledWith('Could not complete project action', {
    description: 'Error: no window',
  })
})

test('the main window follows the navigate event to a known page', async () => {
  const wrapper = app('')
  await flushPromises()
  emit('discard://navigate', 'direct-messages')
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
  emit('discard://navigate', 'no-such-page')
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
  emit('discard://navigate', 'overview')
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('overview')
})

test('a project window does not listen for the navigate event', async () => {
  projectMode()
  await flushPromises()
  expect(listeners.some((entry) => entry.name === 'discard://navigate')).toBe(false)
})

test.each([
  ['', 'main'],
  ['#/project/p1', 'project-p1'],
])('the window events register for the own window only (%j)', async (hash, label) => {
  mocks.label = label
  app(hash)
  await flushPromises()
  const names = [
    'discard://conversation-opened',
    'discard://toggle-terminal',
    'discard://agent-profile',
    ...(hash ? [] : ['discard://navigate']),
  ]
  for (const name of names) {
    const entry = listeners.find((item) => item.name === name)
    expect(entry, name).toBeDefined()
    expect(entry!.options, name).toEqual({ target: { ...window, label } })
  }
})
