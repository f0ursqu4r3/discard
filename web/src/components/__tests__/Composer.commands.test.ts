import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Composer from '../Composer.vue'
import type { AttachmentDto } from '@/bridge/types'
import type { PendingReply } from '../reply'

const api = vi.hoisted(() => ({
  chooseAttachments: vi.fn(),
  importAttachmentFiles: vi.fn(),
  attachmentPreview: vi.fn(),
  getAgents: vi.fn(),
  getProjectMembers: vi.fn(),
  runCommand: vi.fn(),
}))
vi.mock('@/bridge/api', () => api)

const attachment: AttachmentDto = {
  id: 'file',
  name: 'note.txt',
  mediaType: 'text/plain',
  size: 1,
  text: 'a',
}
const reply: PendingReply = {
  prefix: '> [Message from Agent](#discard-message/c1/m1)\n>\n> Question',
  source: { channelId: 'c1', messageId: 'm1' },
  author: 'Agent',
  excerpt: 'Question',
}
const props = { channelId: 'c1', archived: false, busy: false, running: false, channel: 'general' }

type View = ReturnType<typeof mount<typeof Composer>>
let w: View
const type = async (view: View, text: string) => {
  const input = view.get('textarea')
  await input.setValue(text)
}
const enter = (view: View) => view.get('textarea').trigger('keydown', { key: 'Enter' })
const input = (view: View) => view.get('textarea').element as HTMLTextAreaElement
const panel = (view: View) => view.find('[data-ui-id="command-result"]')

beforeEach(() => {
  vi.clearAllMocks()
  api.getAgents.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([])
  api.attachmentPreview.mockResolvedValue('data:image/png;base64,aW1hZ2U=')
  w = mount(Composer, { props: { ...props, alsoPostLabel: undefined } })
})

test('runs a command and does not send it', async () => {
  api.runCommand.mockResolvedValue('pong')
  await type(w, '/ping a b')
  await enter(w)
  await flushPromises()
  expect(api.runCommand).toHaveBeenCalledWith('c1', 'ping', 'a b')
  expect(w.emitted('send')).toBeUndefined()
  expect(panel(w).text()).toContain('/ping a b')
  expect(panel(w).get('[role="status"]').text()).toBe('pong')
  expect(input(w).value).toBe('')
})

test('keeps the draft and shows an alert on an error', async () => {
  api.runCommand.mockRejectedValue('Unknown command: /nope')
  await type(w, '/nope')
  await enter(w)
  await flushPromises()
  expect(panel(w).get('[role="alert"]').text()).toBe('Unknown command: /nope')
  expect(input(w).value).toBe('/nope')
})

test('shows no panel for a success with no output and closes an earlier one', async () => {
  api.runCommand.mockResolvedValueOnce('first').mockResolvedValueOnce(null)
  await type(w, '/a')
  await enter(w)
  await flushPromises()
  expect(panel(w).exists()).toBe(true)
  await type(w, '/b')
  await enter(w)
  await flushPromises()
  expect(panel(w).exists()).toBe(false)
})

test('shows markup in the output as literal text', async () => {
  api.runCommand.mockResolvedValue('<img src=x onerror=alert(1)> **bold**')
  await type(w, '/x')
  await enter(w)
  await flushPromises()
  expect(panel(w).find('img').exists()).toBe(false)
  expect(panel(w).find('strong').exists()).toBe(false)
  expect(panel(w).text()).toContain('<img src=x onerror=alert(1)> **bold**')
})

test('runs a command while a turn runs', async () => {
  await w.setProps({ running: true })
  api.runCommand.mockResolvedValue(null)
  await type(w, '/stop')
  await enter(w)
  await flushPromises()
  expect(api.runCommand).toHaveBeenCalledWith('c1', 'stop', '')
})

test('the send button works for a command during a turn, not for other text', async () => {
  await w.setProps({ running: true })
  api.runCommand.mockResolvedValue(null)
  expect(w.find('[data-ui-id="composer-send"]').exists()).toBe(false)
  await type(w, 'hello')
  expect(w.find('[data-ui-id="composer-send"]').exists()).toBe(false)
  await type(w, '/stop')
  await w.get('[data-ui-id="composer-send"]').trigger('click')
  await flushPromises()
  expect(api.runCommand).toHaveBeenCalledWith('c1', 'stop', '')
})

test('does not run a command in a busy composer', async () => {
  await w.setProps({ busy: true })
  await type(w, '/help')
  await enter(w)
  expect(api.runCommand).not.toHaveBeenCalled()
})

test('does not run a command in an archived composer', async () => {
  await w.setProps({ archived: true })
  await type(w, '/help')
  await enter(w)
  expect(api.runCommand).not.toHaveBeenCalled()
})

test('refuses a command with attachments and keeps both', async () => {
  await w.setProps({ attachments: [attachment] })
  await type(w, '/help')
  await enter(w)
  expect(api.runCommand).not.toHaveBeenCalled()
  expect(w.get('[role="alert"]').text()).toBe('A command cannot have attachments.')
  expect(panel(w).text()).toContain('/help')
  expect(input(w).value).toBe('/help')
  expect(w.emitted('update:attachments')).toBeUndefined()
})

test('does not accept a second command while one runs', async () => {
  let finish!: (value: null) => void
  api.runCommand.mockReturnValue(new Promise((resolve) => (finish = resolve)))
  await type(w, '/slow')
  await enter(w)
  await enter(w)
  expect(api.runCommand).toHaveBeenCalledTimes(1)
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('true')
  finish(null)
  await flushPromises()
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('false')
})

test('drops a result that arrives after the channel changed', async () => {
  let finish!: (value: string) => void
  api.runCommand.mockReturnValue(new Promise((resolve) => (finish = resolve)))
  await w.setProps({ draft: '/slow' })
  await enter(w)
  await w.setProps({ channelId: 'c2', draft: 'other' })
  finish('late')
  await flushPromises()
  expect(panel(w).exists()).toBe(false)
  expect((w.emitted('update:draft') ?? []).filter(([value]) => value === '')).toEqual([])
})

test('keeps a draft typed while a command runs and still shows the result', async () => {
  let finish!: (value: string) => void
  api.runCommand.mockReturnValue(new Promise((resolve) => (finish = resolve)))
  await type(w, '/slow')
  await enter(w)
  await type(w, 'new text')
  finish('done')
  await flushPromises()
  expect(input(w).value).toBe('new text')
  expect(panel(w).get('[role="status"]').text()).toBe('done')
})

test('drops a late command result after a plain message is sent', async () => {
  let finish!: (value: string) => void
  api.runCommand.mockReturnValue(new Promise((resolve) => (finish = resolve)))
  await type(w, '/slow')
  await enter(w)
  await type(w, 'hello')
  await enter(w)
  expect(w.emitted('send')?.[0]?.[0]).toBe('hello')
  finish('late')
  await flushPromises()
  expect(panel(w).exists()).toBe(false)
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('false')
  // The composer accepts a command again after the old run settles.
  api.runCommand.mockResolvedValue('again')
  await type(w, '/next')
  await enter(w)
  await flushPromises()
  expect(api.runCommand).toHaveBeenCalledTimes(2)
  expect(panel(w).get('[role="status"]').text()).toBe('again')
})

test('does not run a second command after a plain message while the first still runs', async () => {
  let finish!: (value: string) => void
  api.runCommand.mockReturnValue(new Promise((resolve) => (finish = resolve)))
  await type(w, '/slow')
  await enter(w)
  await type(w, 'hello')
  await enter(w)
  await type(w, '/second')
  await enter(w)
  expect(api.runCommand).toHaveBeenCalledTimes(1)
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('true')
  finish('late')
  await flushPromises()
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('false')
})

test('a dropped failure and a dropped result after a channel change end the busy state', async () => {
  let fail!: (reason: string) => void
  api.runCommand.mockReturnValue(new Promise((_, reject) => (fail = reject)))
  await type(w, '/slow')
  await enter(w)
  await w.setProps({ channelId: 'c2' })
  expect(panel(w).exists()).toBe(false)
  fail('boom')
  await flushPromises()
  expect(panel(w).exists()).toBe(false)
  expect(w.get('[data-ui-id="composer-send"]').attributes('aria-busy')).toBe('false')
  api.runCommand.mockResolvedValue('ok')
  await type(w, '/next')
  await enter(w)
  await flushPromises()
  expect(api.runCommand).toHaveBeenLastCalledWith('c2', 'next', '')
})

test('clears the result when the channel changes', async () => {
  api.runCommand.mockResolvedValue('out')
  await type(w, '/x')
  await enter(w)
  await flushPromises()
  expect(panel(w).exists()).toBe(true)
  await w.setProps({ channelId: 'c2' })
  expect(panel(w).exists()).toBe(false)
})

test('sends the text of an escaped slash as typed', async () => {
  await type(w, '//help')
  await enter(w)
  expect(api.runCommand).not.toHaveBeenCalled()
  expect(w.emitted('send')?.[0]?.[0]).toBe('//help')
})

test('sends path-like and uppercase text as a message', async () => {
  await type(w, '/usr/bin/env')
  await enter(w)
  expect(w.emitted('send')?.[0]?.[0]).toBe('/usr/bin/env')
  await type(w, '/Help')
  await enter(w)
  expect(w.emitted('send')?.[1]?.[0]).toBe('/Help')
  expect(api.runCommand).not.toHaveBeenCalled()
})

test('closes the result on dismiss and on the next message', async () => {
  api.runCommand.mockResolvedValue('out')
  await type(w, '/x')
  await enter(w)
  await flushPromises()
  await w.get('[aria-label="Dismiss command result"]').trigger('click')
  expect(panel(w).exists()).toBe(false)
  await type(w, '/x')
  await enter(w)
  await flushPromises()
  expect(panel(w).exists()).toBe(true)
  await type(w, 'hello')
  await enter(w)
  expect(panel(w).exists()).toBe(false)
})

test('keeps the reply and the also-post box for a command', async () => {
  w = mount(Composer, { props: { ...props, reply, alsoPostLabel: 'general' } })
  api.runCommand.mockResolvedValue(null)
  await w.get('[role="checkbox"]').trigger('click')
  await type(w, '/help')
  await enter(w)
  await flushPromises()
  expect(w.emitted('cancelReply')).toBeUndefined()
  expect(w.emitted('send')).toBeUndefined()
  expect(w.get('[role="checkbox"]').attributes('aria-checked')).toBe('true')
})
