import { flushPromises, mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import Composer from '../Composer.vue'
import { maySend } from '../composer'

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

test('send stays next to stop while running when the composer allows it', async () => {
  const composer = mount(Composer, {
    props: {
      archived: false,
      busy: false,
      running: true,
      sendWhileRunning: true,
      channel: 'general',
    },
  })
  expect(composer.find('[data-ui-id="composer-stop"]').exists()).toBe(true)
  const send = composer.get('[data-ui-id="composer-send"]')
  expect(send.attributes('disabled')).toBeDefined()
  await composer.get('textarea').setValue('steer')
  await send.trigger('click')
  expect(composer.emitted('send')?.[0]).toEqual(['steer'])
  // Without the prop a running turn blocks Enter as before.
  const plain = mount(Composer, {
    props: { archived: false, busy: false, running: true, channel: 'general' },
  })
  await plain.get('textarea').setValue('steer')
  await plain.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(plain.emitted('send')).toBeUndefined()
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

test('focusEnd places the cursor after an inserted draft without sending', async () => {
  const composer = mount(Composer, {
    attachTo: document.body,
    props: {
      archived: false,
      busy: false,
      running: false,
      channel: 'general',
      draft: '> Quoted message\n\n',
    },
  })
  try {
    await composer.vm.focusEnd()
    const input = composer.get('textarea').element
    expect(document.activeElement).toBe(input)
    expect(input.selectionStart).toBe(input.value.length)
    expect(input.selectionEnd).toBe(input.value.length)
    expect(composer.emitted('send')).toBeUndefined()
  } finally {
    composer.unmount()
  }
})

test('IME confirmation does not send a message', async () => {
  const composer = mount(Composer, {
    props: { archived: false, busy: false, running: false, channel: 'general' },
  })
  await composer.get('textarea').setValue('Draft')
  await composer.get('textarea').trigger('keydown', { key: 'Enter', isComposing: true })
  expect(composer.emitted('send')).toBeUndefined()
})

test('DM composer names the agent and has no channel labels or footer', () => {
  const composer = mount(Composer, {
    props: {
      archived: false,
      busy: false,
      running: false,
      channel: 'General',
      agent: 'Quinn',
      directMessage: true,
    },
  })
  expect(composer.get('textarea').attributes('placeholder')).toBe('Message Quinn')
  expect(composer.get('textarea').attributes('aria-label')).toBe('Message Quinn')
  expect(composer.text()).not.toContain('General')
  expect(composer.text()).not.toContain('Enter to send')
  expect(composer.element.lastElementChild?.getAttribute('data-slot')).toBe('input-group')
})

const api = vi.hoisted(() => ({ getAgents: vi.fn(), getProjectMembers: vi.fn() }))
vi.mock('../../bridge/api', () => api)
const configured = (id: string, name: string) => ({
  id,
  name,
  model: 'test',
  archived: false,
  provider: 'test',
})

function mentionComposer() {
  api.getAgents.mockResolvedValue([
    configured('alex', 'Alex'),
    configured('jordan', 'Jordan'),
    configured('lee', 'Jordan Lee'),
  ])
  api.getProjectMembers.mockResolvedValue([{ id: 'jordan' }])
  return mount(Composer, {
    props: {
      archived: false,
      busy: false,
      running: false,
      channel: 'worktree',
      projectId: 'project',
    },
  })
}

test('typing @ opens project members first; Enter selects without sending', async () => {
  const composer = mentionComposer()
  const input = composer.get('textarea')
  await input.setValue('@')
  await flushPromises()
  expect(composer.findAll('[role="option"]').map((option) => option.text())).toEqual([
    'JordanProject member',
    'AlexAdd to channel',
    'Jordan LeeAdd to channel',
  ])
  await input.trigger('keydown', { key: 'Enter' })
  expect(input.element.value).toBe('@Jordan ')
  expect(composer.emitted('send')).toBeUndefined()
  await input.trigger('keydown', { key: 'Enter' })
  expect(composer.emitted('send')?.[0]).toEqual(['@Jordan'])
})

test('filters names, supports Tab, and quotes names with spaces', async () => {
  const composer = mentionComposer()
  const input = composer.get('textarea')
  await input.setValue('@Jo')
  await flushPromises()
  await input.trigger('keydown', { key: 'ArrowDown' })
  await input.trigger('keydown', { key: 'Tab' })
  expect(input.element.value).toBe('@"Jordan Lee" ')
  expect(composer.emitted('send')).toBeUndefined()
})

test('mention button opens suggestions and Escape closes them', async () => {
  const composer = mentionComposer()
  await composer.get('[aria-label="Mention an agent"]').trigger('click')
  await flushPromises()
  expect(composer.get('textarea').element.value).toBe('@')
  expect(composer.find('[role="listbox"]').exists()).toBe(true)
  await composer.get('textarea').trigger('keydown', { key: 'Escape' })
  expect(composer.find('[role="listbox"]').exists()).toBe(false)
})

test('click inserts a mention at the caret and preserves trailing text', async () => {
  const composer = mentionComposer()
  const input = composer.get('textarea')
  await input.setValue('Ask @Jo about this')
  input.element.setSelectionRange(7, 7)
  await input.trigger('click')
  await flushPromises()
  await composer.findAll('[role="option"]')[0]!.trigger('click')
  expect(input.element.value).toBe('Ask @Jordan about this')
})

test('duplicate names are disabled and unmatched Enter does not send', async () => {
  const composer = mentionComposer()
  api.getAgents.mockResolvedValue([configured('one', 'Jordan'), configured('two', 'Jordan')])
  await composer.get('textarea').setValue('@Jo')
  await flushPromises()
  expect(
    composer
      .findAll('[role="option"]')
      .every((option) => option.attributes('disabled') !== undefined),
  ).toBe(true)
  await composer.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(composer.emitted('send')).toBeUndefined()
})

test('the placeholder and typed message prose stay sans', () => {
  const composer = mount(Composer, {
    props: { archived: false, busy: false, running: false, channel: 'general' },
  })
  expect(composer.get('textarea').classes()).not.toContain('placeholder:font-mono')
  expect(composer.get('textarea').classes()).not.toContain('font-mono')
  composer.unmount()
})
