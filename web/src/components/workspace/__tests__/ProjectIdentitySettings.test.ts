import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { getAgents, setProjectMainAgent, updateProjectProfile } from '@/bridge/api'
import ProjectIdentitySettings from '../ProjectIdentitySettings.vue'
import AgentIconPicker from '../../agents/AgentIconPicker.vue'
import ServerRail from '../../ServerRail.vue'

vi.mock('@/bridge/api', () => ({
  getAgents: vi.fn(),
  setProjectMainAgent: vi.fn(),
  updateProjectProfile: vi.fn(),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const project = {
  id: 'p',
  name: 'Discard',
  path: '/project',
  avatar: 'lucide:rocket:#123456',
  mainAgentId: 'a',
}
let view: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(getAgents).mockResolvedValue([
    { id: 'a', name: 'Quinn', model: 'model', archived: false },
    { id: 'b', name: 'Robin', model: 'model', archived: false },
    { id: 'old', name: 'Archived', model: 'model', archived: true },
  ] as Awaited<ReturnType<typeof getAgents>>)
})
afterEach(() => view?.unmount())
test('saves the project alias and shared icon/color value using its stable ID', async () => {
  view = mount(ProjectIdentitySettings, { props: { project } })
  await flushPromises()
  await view.get('#project-display-name').setValue('  Mission  ')
  view.getComponent(AgentIconPicker).vm.$emit('update:modelValue', 'lucide:bot:#abcdef')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(updateProjectProfile).toHaveBeenCalledWith('p', 'Mission', 'lucide:bot:#abcdef')
  expect(view.emitted('changed')).toHaveLength(1)
  expect(view.text()).toContain('Project icon')
  expect(view.find('[aria-label="Custom project color"]').exists()).toBe(true)
})
test('primary agent selection uses the existing command and refreshes persisted state', async () => {
  view = mount(ProjectIdentitySettings, { props: { project } })
  await flushPromises()
  expect(view.get('option[value="old"]').attributes('disabled')).toBeDefined()
  await view.get('#project-primary-agent').setValue('b')
  await flushPromises()
  expect(setProjectMainAgent).toHaveBeenCalledWith('p', 'b')
  expect(view.emitted('changed')).toHaveLength(1)
  await view.setProps({ generalRunning: true })
  expect(view.get('#project-primary-agent').attributes('disabled')).toBeDefined()
})
test('a failed primary restart refreshes the stored selection and shows its error', async () => {
  vi.mocked(setProjectMainAgent).mockRejectedValue(new Error('Provider unavailable'))
  view = mount(ProjectIdentitySettings, { props: { project } })
  await flushPromises()
  await view.get('#project-primary-agent').setValue('b')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Provider unavailable')
  expect(view.emitted('changed')).toHaveLength(1)
  expect(view.get('#project-primary-agent').attributes('disabled')).toBeUndefined()
})
test('a failed profile save keeps edits and allows retry', async () => {
  vi.mocked(updateProjectProfile).mockRejectedValue(new Error('Save failed'))
  view = mount(ProjectIdentitySettings, { props: { project } })
  await view.get('#project-display-name').setValue('Mission')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Save failed')
  expect(view.emitted('changed')).toBeUndefined()
  expect((view.get('#project-display-name').element as HTMLInputElement).value).toBe('Mission')
})
test('project rail renders the saved icon and color', () => {
  view = mount(ServerRail, {
    props: {
      projects: [project],
      projectId: 'p',
      project: 'Discard',
      unread: false,
      pendingApprovals: 0,
    },
  })
  expect(view.get('[data-project-icon="rocket"]').attributes('data-project-color')).toBe('#123456')
  expect(view.find('[aria-label="Discard"]').exists()).toBe(true)
})
