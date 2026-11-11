import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from '../bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '../bridge/types'
const mocks = vi.hoisted(() => ({
  store: { snapshot: null as SnapshotDto | null, history: [], refresh: vi.fn(async () => {}) },
  selectProject: vi.fn(async () => ''),
  removeProject: vi.fn(async () => {}),
  revealProject: vi.fn(async () => {}),
  error: vi.fn(),
}))
vi.mock('../bridge/store', () => ({ useStore: () => mocks.store }))
vi.mock('../bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('../bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  selectProject: mocks.selectProject,
  removeProject: mocks.removeProject,
  revealProject: mocks.revealProject,
}))
vi.mock('vue-sonner', () => ({ toast: { error: mocks.error } }))
import App from '../App.vue'
import NavigationPanel from '../components/workspace/NavigationPanel.vue'
const target = { id: 'other-project', name: 'Other project', path: '/other-project' }
const wrappers: ReturnType<typeof shallowMount>[] = []
beforeEach(() => {
  vi.clearAllMocks()
  mocks.store.snapshot = structuredClone(fixture) as SnapshotDto
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  )
})
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  vi.unstubAllGlobals()
})
function app() {
  const wrapper = shallowMount(App)
  wrappers.push(wrapper)
  return wrapper
}
test('inactive project settings select the target before showing the page', async () => {
  mocks.store.refresh.mockImplementationOnce(async () => {
    mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: target.id, project: target.name }
  })
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('projectAction', 'settings', target)
  await flushPromises()
  expect(mocks.selectProject).toHaveBeenCalledWith(target.id)
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('project-settings')
})
test('project selection opens the returned channel instead of the overview', async () => {
  mocks.selectProject.mockResolvedValueOnce('general-channel')
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  wrapper.getComponent(NavigationPanel).vm.$emit('action', 'select_project:other-project')
  await flushPromises()
  expect(mocks.selectProject).toHaveBeenCalledWith('other-project')
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('conversation')
})
test('remove and reveal use the target without selecting it', async () => {
  const wrapper = app()
  const panel = wrapper.getComponent(NavigationPanel)
  panel.vm.$emit('projectAction', 'reveal', target)
  await flushPromises()
  expect(mocks.revealProject).toHaveBeenCalledWith(target.path)
  panel.vm.$emit('projectAction', 'remove', target)
  await flushPromises()
  expect(mocks.removeProject).toHaveBeenCalledWith(target.id)
  expect(mocks.selectProject).not.toHaveBeenCalled()
})
