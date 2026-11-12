import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from '../bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '../bridge/types'
const mocks = vi.hoisted(() => ({
  store: { snapshot: null as SnapshotDto | null, history: [], refresh: vi.fn(async () => {}) },
  selectProject: vi.fn(async () => ''),
  removeProject: vi.fn(async () => {}),
  revealProject: vi.fn(async () => {}),
  getRailLayout: vi.fn(async () => ({ items: [{ kind: 'project', projectId: 'other-project' }] })),
  moveRailProject: vi.fn(async () => {}),
  groupRailProjects: vi.fn(async () => 'group'),
  createRailGroup: vi.fn(async () => 'new-group'),
  moveRailGroup: vi.fn(async () => {}),
  renameRailGroup: vi.fn(async () => {}),
  setRailGroupColor: vi.fn(async () => {}),
  setRailGroupCollapsed: vi.fn(async () => {}),
  ungroupRailGroup: vi.fn(async () => {}),
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
  getRailLayout: mocks.getRailLayout,
  moveRailProject: mocks.moveRailProject,
  groupRailProjects: mocks.groupRailProjects,
  createRailGroup: mocks.createRailGroup,
  moveRailGroup: mocks.moveRailGroup,
  renameRailGroup: mocks.renameRailGroup,
  setRailGroupColor: mocks.setRailGroupColor,
  setRailGroupCollapsed: mocks.setRailGroupCollapsed,
  ungroupRailGroup: mocks.ungroupRailGroup,
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
test('the saved rail layout reaches the navigation panel', async () => {
  const wrapper = app()
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('railLayout')).toEqual({
    items: [{ kind: 'project', projectId: 'other-project' }],
  })
})
test('a rail layout that fails to load shows an error and keeps the projects', async () => {
  mocks.getRailLayout.mockRejectedValueOnce(new Error('offline'))
  const wrapper = app()
  await flushPromises()
  expect(mocks.error).toHaveBeenCalledWith('Could not load the project rail', {
    description: 'Error: offline',
  })
  expect(wrapper.getComponent(NavigationPanel).props('railLayout')).toBeNull()
})
test('rail changes call the matching bridge command', async () => {
  const wrapper = app()
  const panel = wrapper.getComponent(NavigationPanel)
  const created = vi.fn()
  for (const change of [
    { type: 'move-project', projectId: 'p', groupId: 'g', index: 2 },
    { type: 'move-project', projectId: 'p', groupId: null, index: 0 },
    { type: 'group-projects', draggedId: 'p', targetId: 'q' },
    { type: 'create-group', projectId: 'p', created },
    { type: 'move-group', groupId: 'g', index: 1 },
    { type: 'rename-group', groupId: 'g', name: 'Work' },
    { type: 'color-group', groupId: 'g', color: 'ansi-2' },
    { type: 'color-group', groupId: 'g', color: null },
    { type: 'collapse-group', groupId: 'g', collapsed: true },
    { type: 'ungroup', groupId: 'g' },
  ])
    panel.vm.$emit('railChange', change)
  await flushPromises()
  expect(mocks.moveRailProject.mock.calls).toEqual([
    ['p', 'g', 2],
    ['p', null, 0],
  ])
  expect(mocks.groupRailProjects).toHaveBeenCalledWith('p', 'q')
  expect(mocks.createRailGroup).toHaveBeenCalledWith('p')
  expect(created).toHaveBeenCalledWith('new-group')
  expect(mocks.moveRailGroup).toHaveBeenCalledWith('g', 1)
  expect(mocks.renameRailGroup).toHaveBeenCalledWith('g', 'Work')
  expect(mocks.setRailGroupColor.mock.calls).toEqual([
    ['g', 'ansi-2'],
    ['g', null],
  ])
  expect(mocks.setRailGroupCollapsed).toHaveBeenCalledWith('g', true)
  expect(mocks.ungroupRailGroup).toHaveBeenCalledWith('g')
  expect(mocks.error).not.toHaveBeenCalledWith(
    'Could not update the project rail',
    expect.anything(),
  )
})
test('a failed rail change shows an error and does not open the rename dialog', async () => {
  mocks.createRailGroup.mockRejectedValueOnce(new Error('gone'))
  const wrapper = app()
  const created = vi.fn()
  wrapper
    .getComponent(NavigationPanel)
    .vm.$emit('railChange', { type: 'create-group', projectId: 'p', created })
  await flushPromises()
  expect(created).not.toHaveBeenCalled()
  expect(mocks.error).toHaveBeenCalledWith('Could not update the project rail', {
    description: 'Error: gone',
  })
})
