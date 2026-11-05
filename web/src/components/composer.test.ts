import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import Composer from './Composer.vue'
import { maySend } from './composer'

test('maySend rejects archived and blank messages', () => {
  expect(maySend(true, 'hello')).toBe(false)
  expect(maySend(false, '  ')).toBe(false)
  expect(maySend(false, 'hello')).toBe(true)
})
test('Enter sends and Shift+Enter does not', async () => {
  const composer = mount(Composer, {
    props: { archived: false, busy: false, running: false, channel: 'general' },
  })
  const input = composer.get('textarea')
  await input.setValue('hello')
  await input.trigger('keydown', { key: 'Enter', shiftKey: true })
  expect(composer.emitted('send')).toBeUndefined()
  await input.trigger('keydown', { key: 'Enter' })
  expect(composer.emitted('send')?.[0]).toEqual(['hello'])
})
test.each([
  { archived: true, busy: false },
  { archived: false, busy: true },
])('composer is disabled for %o', (state) => {
  const composer = mount(Composer, { props: { ...state, running: false, channel: 'general' } })
  expect(composer.get('textarea').attributes('disabled')).toBeDefined()
})
test('stop replaces send while running', () => {
  const composer = mount(Composer, {
    props: { archived: false, busy: false, running: true, channel: 'general' },
  })
  expect(composer.text()).toContain('Stop')
  expect(composer.text()).not.toContain('Send')
})

test('a parent-owned draft survives a composer remount', async () => {
  const { defineComponent, ref } = await import('vue')
  const Host = defineComponent({
    components: { Composer },
    setup: () => ({ visible: ref(true), draft: ref('') }),
    template:
      '<button @click="visible = !visible">Toggle view</button><Composer v-if="visible" v-model:draft="draft" :archived="false" :busy="false" :running="false" channel="general" />',
  })
  const host = mount(Host)
  await host.get('textarea').setValue('Unsent message')
  await host.get('button').trigger('click')
  await host.get('button').trigger('click')
  expect((host.get('textarea').element as HTMLTextAreaElement).value).toBe('Unsent message')
})

test('IME confirmation does not send a message', async () => {
  const composer = mount(Composer, {
    props: { archived: false, busy: false, running: false, channel: 'general' },
  })
  await composer.get('textarea').setValue('Draft')
  await composer.get('textarea').trigger('keydown', { key: 'Enter', isComposing: true })
  expect(composer.emitted('send')).toBeUndefined()
})
