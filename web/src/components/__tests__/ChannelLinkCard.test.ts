import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import ChannelLinkCard from '../ChannelLinkCard.vue'
import type { BlockDto } from '../../bridge/types'
const mocks = vi.hoisted(() => ({ switchChannel: vi.fn(), refresh: vi.fn() }))
vi.mock('@/bridge/api', () => ({ switchChannel: mocks.switchChannel }))
vi.mock('@/bridge/store', () => ({ useStore: () => ({ refresh: mocks.refresh }) }))
afterEach(() => vi.clearAllMocks())
type LinkBlock = Extract<BlockDto, { type: 'channel_link' }>
const to: LinkBlock = {
  type: 'channel_link',
  direction: 'moved_to',
  channel_id: 'c-feature',
  channel_name: 'feature',
}
test('shows the direction and the channel', () => {
  expect(mount(ChannelLinkCard, { props: { block: to } }).text()).toContain('Moved to #feature')
  const from: LinkBlock = { ...to, direction: 'moved_from', channel_name: 'general' }
  expect(mount(ChannelLinkCard, { props: { block: from } }).text()).toContain('Moved from #general')
})
test('open channel switches the channel and refreshes', async () => {
  const order: string[] = []
  mocks.switchChannel.mockImplementation(async () => void order.push('switch'))
  mocks.refresh.mockImplementation(async () => void order.push('refresh'))
  const view = mount(ChannelLinkCard, { props: { block: to } })
  await view
    .findAll('button')
    .find((b) => b.text() === 'Open channel')!
    .trigger('click')
  await flushPromises()
  expect(mocks.switchChannel).toHaveBeenCalledWith('c-feature')
  expect(order).toEqual(['switch', 'refresh'])
})
test('a failed switch shows the error', async () => {
  mocks.switchChannel.mockRejectedValue('boom')
  const view = mount(ChannelLinkCard, { props: { block: to } })
  await view
    .findAll('button')
    .find((b) => b.text() === 'Open channel')!
    .trigger('click')
  await flushPromises()
  expect(view.text()).toContain('Could not open channel: boom')
  expect(mocks.refresh).not.toHaveBeenCalled()
})
