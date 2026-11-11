import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { getAgents, type AgentEntry } from '@/bridge/api'
import { listen } from '@tauri-apps/api/event'
import FriendsList from './FriendsList.vue'
import { Tabs } from '@/components/ui/tabs'
vi.mock('@/bridge/api', () => ({ getAgents: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
const base: AgentEntry = {
  id: 'robin',
  name: 'Robin',
  avatar: null,
  provider: 'codex',
  model: 'exact',
  systemPrompt: '',
  policy: 'deny',
  pluginWrite: false,
  workspaceWrite: false,
  sandboxAutorun: false,
  sandboxAutorunUnavailable: null,
  archived: false,
}
let rows: AgentEntry[], changed: () => void
const stop = vi.fn()
let view: ReturnType<typeof mount>
beforeEach(() => {
  vi.clearAllMocks()
  rows = [{ ...base }, { ...base, id: 'sam', name: 'Sam', archived: true }]
  vi.mocked(getAgents).mockImplementation(async () => [...rows])
  vi.mocked(listen).mockImplementation(async (_event, handler) => {
    changed = () => handler({ event: 'discard://agents', id: 0, payload: undefined })
    return stop
  })
})
afterEach(() => view?.unmount())
test('Friends searches active profiles and sends the selected agent identity', async () => {
  view = mount(FriendsList)
  await flushPromises()
  expect(view.text()).toContain('Robin')
  expect(view.text()).not.toContain('Sam')
  await view.get('[aria-label="Search agents"]').setValue('missing')
  expect(view.text()).toContain('No matching agents')
  await view.get('[aria-label="Search agents"]').setValue('rOb')
  await view.get('[aria-label="Open chat with Robin"]').trigger('click')
  await view
    .findAll('button')
    .find((b) => b.text() === 'View profile')!
    .trigger('click')
  await view.get('[aria-label="DM Robin"]').trigger('click')
  await view
    .findAll('button')
    .find((b) => b.text() === 'Add agent')!
    .trigger('click')
  expect(view.emitted('action')).toEqual([
    ['message_agent:robin'],
    ['view_agent_profile:robin'],
    ['message_agent:robin'],
    ['add_agent'],
  ])
})
test('archived friends expose their profile without offering a new message', async () => {
  view = mount(FriendsList)
  await flushPromises()
  view.getComponent(Tabs).vm.$emit('update:modelValue', 'archived')
  await flushPromises()
  expect(view.text()).toContain('Sam')
  expect(view.text()).not.toContain('Robin')
  expect(view.find('[aria-label="DM Sam"]').exists()).toBe(false)
  await view.get('[aria-label="View Sam\'s profile"]').trigger('click')
  expect(view.emitted('action')).toEqual([['view_agent_profile:sam']])
})
test('saved agent changes refresh Friends and event listeners are released', async () => {
  view = mount(FriendsList)
  await flushPromises()
  rows = [{ ...base, name: 'River' }]
  changed()
  await flushPromises()
  expect(view.text()).toContain('River')
  expect(view.text()).not.toContain('Robin')
  view.unmount()
  expect(stop).toHaveBeenCalledOnce()
})
