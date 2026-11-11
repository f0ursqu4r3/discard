import { expect, test } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import ChannelHeader from './ChannelHeader.vue'

const props = { channel: 'general', memberListVisible: false }

test('the Threads list button is visible, disabled and labelled Soon', async () => {
  const view = mount(ChannelHeader, { props })
  const button = view.get('button[aria-label="Threads — Soon"]')
  expect(button.attributes('disabled')).toBeDefined()
  await button.trigger('click')
  expect(view.emitted()).toEqual({})
  expect(view.get('span[aria-label="Threads — Soon"]').attributes('aria-disabled')).toBe('true')
})

test('pages without a channel have no Threads button', () => {
  const view = mount(ChannelHeader, { props: { ...props, conversation: false } })
  expect(view.find('[aria-label="Threads — Soon"]').exists()).toBe(false)
})
