import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import type { MessageDto, ThreadChipDto } from '../../bridge/types'
import MessageRow from '../MessageRow.vue'
import ThreadChip from '../ThreadChip.vue'
import AgentAvatar from '../agents/AgentAvatar.vue'
import { ContextMenuItem } from '../ui/context-menu'
import { clearAction, pendingAction } from '../thread-actions'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
const views: ReturnType<typeof mount>[] = []
beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation(async (command: string) =>
    command === 'context_menu'
      ? [{ kind: 'action', id: 'copy_message_text', label: 'Copy Message Text', enabled: true }]
      : null,
  )
})
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
  clearAction()
})
const chip: ThreadChipDto = {
  channelId: 'thread',
  name: 'Why a thread?',
  messageCount: 2,
  lastMessage: { role: 'agent', authorName: 'Quinn', preview: 'It keeps the channel clean.' },
  followed: true,
  unread: true,
  kind: 'question',
  state: null,
  agentName: 'Quinn',
  pendingApprovals: 0,
}
const base: MessageDto = {
  id: 'm1',
  role: 'agent',
  continuesGroup: false,
  blocks: [{ type: 'markdown', text: 'Hello' }],
}
function row(message: Partial<MessageDto> = {}, props: Record<string, unknown> = {}, id = 'm1') {
  const view = mount(MessageRow, {
    attachTo: document.body,
    props: { channelId: 'channel', message: { ...base, ...message, id }, ...props },
  })
  views.push(view)
  return view
}
const toolbar = (view: ReturnType<typeof row>) => view.get('[aria-label="Message actions"]')
test('the actions bar shows on hover and keyboard focus, not on a mouse click focus', () => {
  const classes = toolbar(row()).classes()
  expect(classes).toContain('invisible')
  expect(classes).toContain('group-hover:visible')
  expect(classes).toContain('group-focus-visible:visible')
  expect(classes).toContain('group-has-[:focus-visible]:visible')
  expect(classes).not.toContain('group-focus-within:visible')
})
async function menuItems(view: ReturnType<typeof row>) {
  await view.get('article').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
  await flushPromises()
  return view.findAllComponents(ContextMenuItem)
}

test('a message without a thread offers Start thread and asks the app to start a draft', async () => {
  const view = row()
  const button = toolbar(view).get('button[aria-label="Start thread"]')
  expect(button.attributes('disabled')).toBeUndefined()
  await button.trigger('click')
  expect(view.emitted('startThread')?.[0]?.[0]).toMatchObject({ id: 'm1' })
  expect(view.emitted('openThread')).toBeUndefined()
  expect(invoke).not.toHaveBeenCalledWith('start_thread', expect.anything())
  const items = await menuItems(view)
  const item = items.find((entry) => entry.text() === 'Start thread')!
  expect(item.attributes('aria-disabled')).not.toBe('true')
  item.vm.$emit('select')
  expect(view.emitted('startThread')).toHaveLength(2)
})

test('a message with a thread offers Open thread and never Start thread', async () => {
  const view = row({ thread: chip })
  expect(toolbar(view).find('button[aria-label="Start thread"]').exists()).toBe(false)
  await toolbar(view).get('button[aria-label="Open thread"]').trigger('click')
  expect(view.emitted('openThread')).toEqual([['thread']])
  expect(view.emitted('startThread')).toBeUndefined()
  const items = await menuItems(view)
  expect(items.some((entry) => entry.text() === 'Start thread')).toBe(false)
  items.find((entry) => entry.text() === 'Open thread')!.vm.$emit('select')
  expect(view.emitted('openThread')).toHaveLength(2)
})

test('Start thread is disabled while the channel is read only, Open thread is not', async () => {
  const view = row({}, { threadDisabled: true })
  const button = toolbar(view).get('button[aria-label="Start thread"]')
  expect(button.attributes('disabled')).toBeDefined()
  await button.trigger('click')
  expect(view.emitted('startThread')).toBeUndefined()
  const items = await menuItems(view)
  expect(items.find((entry) => entry.text() === 'Start thread')!.attributes('aria-disabled')).toBe(
    'true',
  )
  const open = row({ thread: chip }, { threadDisabled: true })
  expect(
    toolbar(open).get('button[aria-label="Open thread"]').attributes('disabled'),
  ).toBeUndefined()
})

test('a streaming row has no thread action', () => {
  const view = row({}, { showCursor: true }, 'live')
  expect(view.find('[aria-label="Message actions"]').exists()).toBe(false)
  expect(view.text()).not.toContain('Start thread')
})

test('a reference row opens its source thread and cannot start one', async () => {
  const view = row({
    postedFrom: { sourceMessageId: 's', threadChannelId: 'source', threadName: 'Why?' },
  })
  expect(toolbar(view).find('button[aria-label="Start thread"]').exists()).toBe(false)
  await toolbar(view).get('button[aria-label="Open thread"]').trigger('click')
  expect(view.emitted('openThread')).toEqual([['source']])
  const link = view.get('[data-ui-id="posted-from-thread"]')
  expect(link.attributes('aria-label')).toBe('From thread Why?')
  expect(link.text()).toContain('from thread')
  expect(link.text()).toContain('Why?')
  await link.trigger('click')
  expect(view.emitted('openThread')).toEqual([['source'], ['source']])
})

test('Remove from channel on a reference row asks for a confirm dialog, from the toolbar and the menu', async () => {
  const view = row({
    postedFrom: { sourceMessageId: 's', threadChannelId: 'source', threadName: 'Why?' },
  })
  const button = toolbar(view).get('button[aria-label="Remove from channel"]')
  expect(button.attributes('disabled')).toBeUndefined()
  expect(view.text()).not.toContain('Soon')
  await button.trigger('click')
  expect(pendingAction.value).toEqual({
    kind: 'remove-from-channel',
    messageId: 'm1',
    threadName: 'Why?',
  })
  expect(invoke).not.toHaveBeenCalledWith('remove_posted_message', expect.anything())
  clearAction()
  const items = await menuItems(view)
  const remove = items.find((entry) => entry.text() === 'Remove from channel')!
  expect(remove.attributes('aria-disabled')).toBeUndefined()
  remove.vm.$emit('select')
  expect(pendingAction.value).toMatchObject({ kind: 'remove-from-channel', messageId: 'm1' })
  expect(invoke).not.toHaveBeenCalledWith('remove_posted_message', expect.anything())
  // A normal message has no such control.
  expect(row().find('button[aria-label="Remove from channel"]').exists()).toBe(false)
})

test('the anchor row in a thread panel has no thread action and no chip', () => {
  const view = row({ thread: chip }, { surface: 'anchor' })
  expect(toolbar(view).find('[aria-label$="thread"]').exists()).toBe(false)
  expect(toolbar(view).find('[aria-label="Post to channel"]').exists()).toBe(false)
  expect(view.findComponent(ThreadChip).exists()).toBe(false)
})

test('a thread message has no thread action and can post to the channel', async () => {
  const view = row({}, { surface: 'thread', channelId: 'thread' })
  expect(toolbar(view).find('[aria-label$="thread"]').exists()).toBe(false)
  await toolbar(view).get('button[aria-label="Post to channel"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('post_thread_message', { messageId: 'm1' })
})

test('Post to channel is in the context menu of a thread message', async () => {
  const view = row({}, { surface: 'thread', channelId: 'thread' })
  const items = await menuItems(view)
  items.find((entry) => entry.text() === 'Post to channel')!.vm.$emit('select')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('post_thread_message', { messageId: 'm1' })
})

test('a posted thread message shows the done state and posts nothing again', async () => {
  const view = row({ postedToChannel: true }, { surface: 'thread', channelId: 'thread' })
  const button = toolbar(view).get('button[aria-label="Posted to channel"]')
  expect(button.attributes('disabled')).toBeDefined()
  await button.trigger('click')
  const items = await menuItems(view)
  const item = items.find((entry) => entry.text() === 'Posted to channel')!
  expect(item.attributes('aria-disabled')).toBe('true')
  item.vm.$emit('select')
  await flushPromises()
  expect(invoke).not.toHaveBeenCalledWith('post_thread_message', expect.anything())
})

test('Post to channel is disabled while the thread is read only', () => {
  const view = row({}, { surface: 'thread', channelId: 'thread', threadDisabled: true })
  expect(
    toolbar(view).get('button[aria-label="Post to channel"]').attributes('disabled'),
  ).toBeDefined()
})

test('the chip under an anchor shows the thread and opens it', async () => {
  const view = row({ thread: chip })
  const element = view.get('[data-ui-id="thread-chip"]')
  expect(element.text()).toContain('Why a thread?')
  expect(element.text()).toContain('2 messages')
  expect(element.text()).toContain('Quinn: It keeps the channel clean.')
  expect(element.find('[aria-label="Unread messages"]').exists()).toBe(true)
  expect(element.find('[aria-label="Follow thread Why a thread?"]').exists()).toBe(false)
  await element.get('button').trigger('click')
  expect(view.emitted('openThread')).toEqual([['thread']])
})

test('the chip is a native button, so the keyboard reaches it', () => {
  const view = row({ thread: chip })
  const button = view.get('[data-ui-id="thread-chip"] button')
  expect(button.element.tagName).toBe('BUTTON')
  expect(button.attributes('aria-label')).toBe('Open thread Why a thread?')
  expect(button.attributes('tabindex')).toBeUndefined()
})

test('a chip of a thread that is not followed offers Follow without opening the thread', async () => {
  const view = row({ thread: { ...chip, followed: false, unread: false, messageCount: 1 } })
  const follow = view.get('[aria-label="Follow thread Why a thread?"]')
  expect(view.get('[data-ui-id="thread-chip"]').text()).toContain('1 message')
  await follow.trigger('click')
  expect(invoke).toHaveBeenCalledWith('follow_thread', { threadId: 'thread' })
  expect(view.emitted('openThread')).toBeUndefined()
})

test('a user message in the chip is named You', () => {
  const view = row({
    thread: { ...chip, lastMessage: { role: 'user', authorName: null, preview: 'Why?' } },
  })
  expect(view.get('[data-ui-id="thread-chip"]').text()).toContain('You: Why?')
})

const subAgentChip = (id: string, patch: Partial<ThreadChipDto> = {}): ThreadChipDto => ({
  ...chip,
  channelId: id,
  name: `Task ${id}`,
  lastMessage: null,
  followed: false,
  unread: false,
  kind: 'sub_agent',
  state: 'done',
  agentName: 'Reviewer',
  ...patch,
})
const chipLabels = (view: ReturnType<typeof row>) =>
  view
    .findAll('[data-ui-id="thread-chip"]')
    .map((node) => node.get('button').attributes('aria-label'))

test('several sub-agent chips show in task order under the question chip', async () => {
  const view = row({
    thread: chip,
    subAgentThreads: [
      subAgentChip('b', { state: 'running' }),
      subAgentChip('a', { state: 'failed' }),
      subAgentChip('c', { state: 'timed_out' }),
    ],
  })
  const chips = view.findAll('[data-ui-id="thread-chip"]')
  expect(chips.map((node) => node.attributes('data-kind'))).toEqual([
    'question',
    'sub_agent',
    'sub_agent',
    'sub_agent',
  ])
  expect(chipLabels(view)).toEqual([
    'Open thread Why a thread?',
    'Open thread Task b, Reviewer, Running',
    'Open thread Task a, Reviewer, Failed',
    'Open thread Task c, Reviewer, Timed out',
  ])
  await chips[2]!.get('button').trigger('click')
  expect(view.emitted('openThread')).toEqual([['a']])
})

test('a response with no question thread still shows its sub-agent chips', () => {
  const view = row({ subAgentThreads: [subAgentChip('a')] })
  expect(view.findAll('[data-ui-id="thread-chip"]')).toHaveLength(1)
  // Sub-agent chips do not change the message thread action.
  expect(toolbar(view).find('button[aria-label="Start thread"]').exists()).toBe(true)
})

test('the anchor row and a thread row show no sub-agent chips', () => {
  const subAgentThreads = [subAgentChip('a')]
  expect(
    row({ subAgentThreads }, { surface: 'anchor' }).find('[data-ui-id="thread-chip"]').exists(),
  ).toBe(false)
  expect(
    row({ subAgentThreads }, { surface: 'thread' }).find('[data-ui-id="thread-chip"]').exists(),
  ).toBe(false)
})

test('a brief shows the parent agent as its author, not You', () => {
  const view = row(
    {
      role: 'user',
      author: { agentId: 'parent', name: 'Quinn', provider: 'claude', avatar: null },
      blocks: [{ type: 'markdown', text: 'Find the callers.' }],
    },
    { surface: 'thread' },
  )
  expect(view.get('[data-testid="message-identity"]').text()).toBe('Quinn')
  expect(view.find('[data-testid="message-agent-tag"]').exists()).toBe(false)
  expect(view.findComponent(AgentAvatar).exists()).toBe(true)
  expect(view.text()).not.toContain('You')
})

test('a user message without an author is still You', () => {
  const view = row({ role: 'user' })
  expect(view.get('[data-testid="message-identity"]').text()).toBe('You')
})
