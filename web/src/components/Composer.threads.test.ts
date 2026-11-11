import { expect, test } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import Composer from './Composer.vue'

const props = { archived: false, busy: false, running: false, channel: 'general' }
const box = (view: ReturnType<typeof mount>) => view.get('[data-ui-id="composer-also-post"]')

test('a channel composer has no Also post box and sends as before', async () => {
  const view = mount(Composer, { props })
  expect(view.find('[data-ui-id="composer-also-post"]').exists()).toBe(false)
  await view.get('textarea').setValue('hello')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[0]).toEqual(['hello'])
})

test('the thread composer names the parent channel and starts unchecked', () => {
  const view = mount(Composer, { props: { ...props, alsoPostLabel: '#general' } })
  expect(view.text()).toContain('Also post to #general')
  expect(box(view).attributes('aria-checked')).toBe('false')
})

test('Also post applies to one message and resets after the send', async () => {
  const view = mount(Composer, { props: { ...props, alsoPostLabel: '#general' } })
  await view.get('textarea').setValue('first')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[0]).toEqual(['first', [], false])
  await box(view).trigger('click')
  expect(box(view).attributes('aria-checked')).toBe('true')
  await view.get('textarea').setValue('second')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[1]).toEqual(['second', [], true])
  expect(box(view).attributes('aria-checked')).toBe('false')
  await view.get('textarea').setValue('third')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[2]).toEqual(['third', [], false])
})

test('the label names the checkbox and the box follows the disabled composer', () => {
  const view = mount(Composer, {
    props: {
      ...props,
      archived: true,
      alsoPostLabel: 'the conversation',
      placeholder: 'Reply in thread',
    },
  })
  expect(box(view).attributes('disabled')).toBeDefined()
  expect(view.get('label').text()).toBe('Also post to the conversation')
  expect(view.get('textarea').attributes('placeholder')).toBe('Reply in thread')
  expect(view.text()).not.toContain('Channel archived')
})
