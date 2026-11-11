import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import type { ThreadChipDto } from '../../bridge/types'
import ChannelHeader from '../ChannelHeader.vue'

const props = { channel: 'general', memberListVisible: false }
const views: ReturnType<typeof mount>[] = []
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
beforeEach(() => vi.stubGlobal('ResizeObserver', ResizeObserverStub))
afterEach(() => {
  vi.unstubAllGlobals()
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})
const chip = (id: string): ThreadChipDto => ({
  channelId: id,
  name: `Thread ${id}`,
  messageCount: 2,
  lastMessage: null,
  followed: true,
  unread: false,
  kind: 'question',
  state: null,
  agentName: 'Quinn',
  pendingApprovals: 0,
})

test('the Threads button is enabled, has no Soon label, and opens the list', async () => {
  const view = mount(ChannelHeader, {
    attachTo: document.body,
    props: { ...props, threads: [chip('a')] },
  })
  views.push(view)
  const button = view.get('button[aria-label="Threads"]')
  expect(button.attributes('disabled')).toBeUndefined()
  expect(view.text()).not.toContain('Soon')
  expect(view.find('[aria-label$="Soon"]').exists()).toBe(false)
  await button.trigger('click')
  await flushPromises()
  expect(document.querySelector('[data-ui-id="threads-list"]')).not.toBeNull()
  expect(document.querySelectorAll('[data-ui-id="threads-list-row"]')).toHaveLength(1)
})

test('opening a thread from the list tells the app and closes the list', async () => {
  const view = mount(ChannelHeader, {
    attachTo: document.body,
    props: { ...props, threads: [chip('a')] },
  })
  views.push(view)
  await view.get('button[aria-label="Threads"]').trigger('click')
  await flushPromises()
  document.querySelector<HTMLElement>('button[aria-label="Open thread Thread a"]')!.click()
  await flushPromises()
  expect(view.emitted('openThread')).toEqual([['a']])
  expect(document.querySelector('[data-ui-id="threads-list"]')).toBeNull()
})

test('pages without a channel have no Threads button', () => {
  const view = mount(ChannelHeader, { props: { ...props, conversation: false } })
  views.push(view)
  expect(view.find('[aria-label="Threads"]').exists()).toBe(false)
})
