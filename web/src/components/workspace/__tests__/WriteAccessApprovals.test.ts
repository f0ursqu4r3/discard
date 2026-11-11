import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import WriteAccessApprovals from './WriteAccessApprovals.vue'
const api = vi.hoisted(() => ({ getWriteRequests: vi.fn(), decideWriteRequest: vi.fn() }))
vi.mock('@/bridge/api', () => api)
afterEach(() => vi.clearAllMocks())
const pending = {
  id: 'r',
  channelId: 'c',
  agentId: 'a',
  kind: 'worktree_create',
  branch: 'feature',
  path: '/target',
  state: 'pending',
  result: null,
}
test('shows the branch and target folder', async () => {
  api.getWriteRequests.mockResolvedValue([pending])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: false } })
  await flushPromises()
  expect(view.text()).toContain('feature')
  expect(view.text()).toContain('/target')
  expect(view.text()).toContain('Create a worktree for this branch?')
  view.unmount()
})
test('approve is disabled while the channel is running', async () => {
  api.getWriteRequests.mockResolvedValue([pending])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: true } })
  await flushPromises()
  expect(
    view
      .findAll('button')
      .find((b) => b.text() === 'Create worktree')!
      .attributes('disabled'),
  ).toBeDefined()
  expect(view.text()).toContain('Wait for the current response to finish before approving.')
  view.unmount()
})
test('approve and deny call the command once', async () => {
  for (const approve of [true, false]) {
    api.getWriteRequests.mockResolvedValue([pending])
    api.decideWriteRequest.mockResolvedValue({
      ...pending,
      state: approve ? 'completed' : 'denied',
    })
    const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: false } })
    await flushPromises()
    const button = view
      .findAll('button')
      .find((b) => b.text() === (approve ? 'Create worktree' : 'Deny'))!
    await button.trigger('click')
    await flushPromises()
    expect(api.decideWriteRequest).toHaveBeenLastCalledWith('c', 'r', approve)
    expect(view.findAll('button')).toHaveLength(0)
    view.unmount()
  }
  expect(api.decideWriteRequest).toHaveBeenCalledTimes(2)
})
