import { expect, test, vi, beforeEach } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import ProjectAlwaysAllowRules from '../ProjectAlwaysAllowRules.vue'
import { getAlwaysAllowRules, revokeAlwaysAllowRule } from '@/bridge/api'
vi.mock('@/bridge/api', () => ({ getAlwaysAllowRules: vi.fn(), revokeAlwaysAllowRule: vi.fn() }))
beforeEach(() => {
  vi.mocked(getAlwaysAllowRules).mockResolvedValue([
    {
      id: 'r',
      agentId: 'a',
      agentName: 'Scout',
      kind: 'command',
      text: 'cargo test',
      createdAt: 1,
    },
  ])
  vi.mocked(revokeAlwaysAllowRule).mockResolvedValue(true)
})
test('lists rules with agent and target', async () => {
  const view = mount(ProjectAlwaysAllowRules, { props: { projectId: 'p' } })
  await flushPromises()
  expect(view.text()).toContain('Scout')
  expect(view.text()).toContain('Command')
  expect(view.find('code').text()).toBe('cargo test')
})
test('revoke removes the row', async () => {
  const view = mount(ProjectAlwaysAllowRules, { props: { projectId: 'p' } })
  await flushPromises()
  await view.find('button').trigger('click')
  await flushPromises()
  expect(revokeAlwaysAllowRule).toHaveBeenLastCalledWith('p', 'r')
  expect(view.find('code').exists()).toBe(false)
})
test('empty state explains where rules come from', async () => {
  vi.mocked(getAlwaysAllowRules).mockResolvedValue([])
  const view = mount(ProjectAlwaysAllowRules, { props: { projectId: 'p' } })
  await flushPromises()
  expect(view.text()).toContain(
    'No Always-allow rules yet. Choose Always allow on an approval card to add one.',
  )
})
