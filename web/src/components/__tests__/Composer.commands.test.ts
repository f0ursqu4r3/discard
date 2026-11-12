import { beforeEach, describe, expect, it, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Composer from '../Composer.vue'
import type { AttachmentDto, CommandDto } from '@/bridge/types'
import type { PendingReply } from '../reply'

const api = vi.hoisted(() => ({
  chooseAttachments: vi.fn(),
  importAttachmentFiles: vi.fn(),
  attachmentPreview: vi.fn(),
  getAgents: vi.fn(),
  getProjectMembers: vi.fn(),
  runCommand: vi.fn(),
  getCommands: vi.fn(),
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
const command = (name: string, extra: Partial<CommandDto> = {}): CommandDto => ({
  name,
  description: `Describe ${name}`,
  args: null,
  source: 'builtin',
  planned: false,
  ...extra,
})
const registry = [
  command('help'),
  command('stop'),
  command('model', { planned: true }),
  command('echo', { source: 'tools', args: '<text>', description: 'Repeat' }),
]
const props = { channelId: 'c1', archived: false, busy: false, running: false, channel: 'general' }

type View = ReturnType<typeof mount<typeof Composer>>
let w: View
const type = async (view: View, text: string) => {
  const input = view.get('textarea')
  await input.setValue(text)
}
const enter = (view: View) => view.get('textarea').trigger('keydown', { key: 'Enter' })
const input = (view: View) => view.get('textarea').element as HTMLTextAreaElement
const key = (view: View, name: string) => view.get('textarea').trigger('keydown', { key: name })
const panel = (view: View) => view.find('[data-ui-id="command-result"]')

beforeEach(() => {
  vi.clearAllMocks()
  api.getAgents.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([])
  api.getCommands.mockResolvedValue(registry)
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

describe('command popup', () => {
  const popup = (view: View) => view.find('[aria-label="Commands"]')
  const rows = (view: View) => view.findAll('[aria-label="Commands"] [role="option"]')

  it('opens for a partial name and lists matches in registry order', async () => {
    await type(w, '/')
    await flushPromises()
    const list = rows(w)
    expect(list.map((r) => r.text())).toEqual([
      expect.stringContaining('/help'),
      expect.stringContaining('/stop'),
      expect.stringContaining('/model'),
      expect.stringContaining('/echo'),
    ])
    expect(list[3]!.text()).toContain('<text>')
    expect(list[3]!.text()).toContain('Repeat')
    expect(list[3]!.text()).toContain('tools')
    expect(list[0]!.text()).not.toContain('builtin')
  })

  it('closes after the first space', async () => {
    await type(w, '/echo ')
    await flushPromises()
    expect(popup(w).exists()).toBe(false)
  })

  it('does not open in an archived or busy composer', async () => {
    await w.setProps({ busy: true })
    await type(w, '/')
    await flushPromises()
    expect(popup(w).exists()).toBe(false)
    expect(api.getCommands).not.toHaveBeenCalled()
    await w.setProps({ busy: false, archived: true })
    await type(w, '/h')
    await flushPromises()
    expect(popup(w).exists()).toBe(false)
  })

  it('marks a planned command as Soon and skips it', async () => {
    await type(w, '/')
    await flushPromises()
    const model = rows(w)[2]!
    expect(model.attributes('aria-disabled')).toBe('true')
    expect(model.text()).toContain('Soon')
    expect(model.text()).not.toContain('TODO')
    await key(w, 'ArrowDown')
    await key(w, 'ArrowDown') // help -> stop -> echo
    expect(w.find('[role="option"][aria-selected="true"]').text()).toContain('/echo')
    expect(w.get('textarea').attributes('aria-activedescendant')).toBe(
      w.get('[role="option"][aria-selected="true"]').attributes('id'),
    )
  })

  it('does not complete a planned command on click', async () => {
    await type(w, '/')
    await flushPromises()
    await rows(w)[2]!.trigger('click')
    expect(input(w).value).toBe('/')
  })

  it('completes with Tab', async () => {
    await type(w, '/st')
    await flushPromises()
    await key(w, 'Tab')
    expect(input(w).value).toBe('/stop ')
    expect(input(w).selectionStart).toBe(6)
    expect(api.runCommand).not.toHaveBeenCalled()
  })

  it('completes with a click', async () => {
    await type(w, '/ec')
    await flushPromises()
    await rows(w)[0]!.trigger('click')
    expect(input(w).value).toBe('/echo ')
    expect(popup(w).exists()).toBe(false)
    expect(api.runCommand).not.toHaveBeenCalled()
  })

  it('completes with Enter on a partial name and runs on an exact name', async () => {
    api.runCommand.mockResolvedValue(null)
    await type(w, '/he')
    await flushPromises()
    await enter(w)
    expect(input(w).value).toBe('/help ')
    expect(api.runCommand).not.toHaveBeenCalled()
    await type(w, '/help')
    await flushPromises()
    await enter(w)
    await flushPromises()
    expect(api.runCommand).toHaveBeenCalledWith('c1', 'help', '')
  })

  it('runs on Enter before the registry loads', async () => {
    api.getCommands.mockReturnValue(new Promise(() => {}))
    api.runCommand.mockResolvedValue(null)
    await type(w, '/help')
    await enter(w)
    await flushPromises()
    expect(api.runCommand).toHaveBeenCalledWith('c1', 'help', '')
  })

  it('shows one disabled row for no match and still runs on Enter', async () => {
    api.runCommand.mockRejectedValue('Unknown command: /zzz')
    await type(w, '/zzz')
    await flushPromises()
    expect(popup(w).text()).toBe('No matching command')
    expect(rows(w)[0]!.attributes('aria-disabled')).toBe('true')
    await enter(w)
    await flushPromises()
    expect(w.get('[role="alert"]').text()).toBe('Unknown command: /zzz')
  })

  it('shows a load failure row and still runs on Enter', async () => {
    api.getCommands.mockRejectedValue('x')
    api.runCommand.mockResolvedValue(null)
    await type(w, '/help')
    await flushPromises()
    expect(popup(w).text()).toBe('Could not load commands.')
    await enter(w)
    await flushPromises()
    expect(api.runCommand).toHaveBeenCalledWith('c1', 'help', '')
  })

  it('closes on Escape and keeps the draft', async () => {
    await type(w, '/he')
    await flushPromises()
    await key(w, 'Escape')
    expect(popup(w).exists()).toBe(false)
    expect(input(w).value).toBe('/he')
  })

  it('opens again on the next input after Escape', async () => {
    await type(w, '/he')
    await flushPromises()
    await key(w, 'Escape')
    await type(w, '/hel')
    expect(popup(w).exists()).toBe(true)
  })

  it('reads the registry each time it opens', async () => {
    await type(w, '/')
    await flushPromises()
    await type(w, 'x')
    await type(w, '/')
    await flushPromises()
    expect(api.getCommands).toHaveBeenCalledTimes(2)
  })

  it('opens in a direct message composer', async () => {
    await w.setProps({ directMessage: true })
    await type(w, '/')
    await flushPromises()
    expect(popup(w).exists()).toBe(true)
  })

  it('does not open for escaped or non-command text', async () => {
    await type(w, '//')
    await type(w, '/Help')
    expect(popup(w).exists()).toBe(false)
  })
})
