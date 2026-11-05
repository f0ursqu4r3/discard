import { mount } from '@vue/test-utils'
import { expect, test } from 'vitest'
import Composer from './Composer.vue'
import { maySend } from './composer'

test('maySend rejects archived and blank messages', () => {
  expect(maySend(true, 'hello')).toBe(false)
  expect(maySend(false, '  ')).toBe(false)
  expect(maySend(false, 'hello')).toBe(true)
})
test('Enter sends and Shift+Enter does not', async () => {
  const composer = mount(Composer, { props: { archived: false, busy: false, running: false, channel: 'general' } })
  const input = composer.get('textarea')
  await input.setValue('hello')
  await input.trigger('keydown', { key: 'Enter', shiftKey: true })
  expect(composer.emitted('send')).toBeUndefined()
  await input.trigger('keydown', { key: 'Enter' })
  expect(composer.emitted('send')?.[0]).toEqual(['hello'])
})
test.each([{ archived: true, busy: false }, { archived: false, busy: true }])('composer is disabled for %o', state => {
  const composer = mount(Composer, { props: { ...state, running: false, channel: 'general' } })
  expect(composer.get('textarea').attributes('disabled')).toBeDefined()
})
test('stop replaces send while running', () => {
  const composer = mount(Composer, { props: { archived: false, busy: false, running: true, channel: 'general' } })
  expect(composer.text()).toContain('Stop')
  expect(composer.text()).not.toContain('Send')
})
