import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { setProjectShowArchived } from '@/bridge/api'
import ProjectChannelListSettings from './ProjectChannelListSettings.vue'

vi.mock('@/bridge/api', () => ({ setProjectShowArchived: vi.fn() }))
const project = { id: 'p', name: 'Discard', path: '/project' }
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => view?.unmount())
const toggle = () => view!.get('#project-show-archived')

test('the Channel list card holds one switch with its help text', () => {
  view = mount(ProjectChannelListSettings, { props: { project } })
  expect(view.text()).toContain('Channel list')
  expect(view.get('label[for="project-show-archived"]').text()).toBe('Show archived channels')
  expect(view.text()).toContain(
    'Show the Archived section in the channel list. It holds archived channels and channels of deleted branches that have messages.',
  )
  expect(view.findAll('[role="switch"]')).toHaveLength(1)
  expect(toggle().attributes('aria-checked')).toBe('false')
})

test('the switch shows the saved value', () => {
  view = mount(ProjectChannelListSettings, {
    props: { project: { ...project, showArchived: true } },
  })
  expect(toggle().attributes('aria-checked')).toBe('true')
})

test('turning the switch on saves it for the project and reports the change', async () => {
  vi.mocked(setProjectShowArchived).mockResolvedValue()
  view = mount(ProjectChannelListSettings, { props: { project } })
  await toggle().trigger('click')
  await flushPromises()
  expect(setProjectShowArchived).toHaveBeenCalledWith('p', true)
  expect(toggle().attributes('aria-checked')).toBe('true')
  expect(view.emitted('changed')).toHaveLength(1)
  await toggle().trigger('click')
  await flushPromises()
  expect(setProjectShowArchived).toHaveBeenLastCalledWith('p', false)
  expect(toggle().attributes('aria-checked')).toBe('false')
})

test('a failed save restores the switch and shows the error', async () => {
  vi.mocked(setProjectShowArchived).mockRejectedValue(new Error('Project unavailable'))
  view = mount(ProjectChannelListSettings, { props: { project } })
  await toggle().trigger('click')
  await flushPromises()
  expect(toggle().attributes('aria-checked')).toBe('false')
  expect(view.get('[role="alert"]').text()).toContain('Project unavailable')
  expect(view.emitted('changed')).toBeUndefined()
})

test('the switch follows a changed project value', async () => {
  view = mount(ProjectChannelListSettings, { props: { project } })
  await view.setProps({ project: { ...project, showArchived: true } })
  expect(toggle().attributes('aria-checked')).toBe('true')
})
