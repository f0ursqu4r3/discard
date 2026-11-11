import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { removePostedMessage } from '@/bridge/api'
import type { MessageDto } from '@/bridge/types'
import { clearAction, pendingAction, requestAction } from '../thread-actions'
import ThreadActionDialogs from './ThreadActionDialogs.vue'

vi.mock('@/bridge/api', () => ({
  renameThread: vi.fn(),
  deleteThread: vi.fn(),
  threadPostedCount: vi.fn(async () => 0),
  removePostedMessage: vi.fn(async () => undefined),
  renameTextChannel: vi.fn(),
}))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.mocked(removePostedMessage).mockClear())
afterEach(() => {
  view?.unmount()
  view = undefined
  document.body.innerHTML = ''
  clearAction()
})
const from = { sourceMessageId: 's', threadChannelId: 't', threadName: 'Why?' }
const at = (id: string, role: MessageDto['role'], postedFrom?: MessageDto['postedFrom']) =>
  ({ id, role, continuesGroup: false, blocks: [], postedFrom }) as MessageDto
function host(messages: MessageDto[] = []) {
  view = mount(ThreadActionDialogs, { attachTo: document.body, props: { messages } })
  return view
}
const dialog = (id: string) => document.querySelector(`[data-ui-id="${id}-dialog"]`)

test('no dialog shows until a control asks for one', async () => {
  host()
  await flushPromises()
  expect(document.querySelector('[role="dialog"]')).toBeNull()
})

test('each request opens its own dialog with its values', async () => {
  host()
  requestAction({ kind: 'rename-thread', threadId: 't', name: 'Why?' })
  await flushPromises()
  expect(dialog('rename-thread')).not.toBeNull()
  expect(document.querySelector<HTMLInputElement>('#rename-thread-name')!.value).toBe('Why?')
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  await flushPromises()
  expect(pendingAction.value).toBeNull()
  expect(dialog('rename-thread')).toBeNull()

  requestAction({ kind: 'delete-thread', threadId: 't', name: 'Why?' })
  await flushPromises()
  expect(dialog('delete-thread')).not.toBeNull()
  clearAction()
  await flushPromises()

  requestAction({ kind: 'rename-channel', projectId: 'p', channelId: 'c', name: 'plans' })
  await flushPromises()
  expect(dialog('rename-channel')).not.toBeNull()
  expect(document.querySelector<HTMLInputElement>('#rename-channel-name')!.value).toBe('plans')
})

test('the remove dialog warns only when an agent message follows the row', async () => {
  host([at('q', 'user'), at('ref', 'agent', from), at('reply', 'agent')])
  requestAction({ kind: 'remove-from-channel', messageId: 'ref', threadName: 'Why?' })
  await flushPromises()
  expect(document.querySelector('[data-ui-id="remove-from-channel-warning"]')).not.toBeNull()
  clearAction()
  await flushPromises()
  view!.unmount()
  document.body.innerHTML = ''

  host([at('q', 'user'), at('ref', 'agent', from), at('next', 'user')])
  requestAction({ kind: 'remove-from-channel', messageId: 'ref', threadName: 'Why?' })
  await flushPromises()
  expect(dialog('remove-from-channel')).not.toBeNull()
  expect(document.querySelector('[data-ui-id="remove-from-channel-warning"]')).toBeNull()
})

test('the warning stays while the dialog closes after the row left the transcript', async () => {
  const view = host([at('ref', 'agent', from), at('reply', 'agent')])
  requestAction({ kind: 'remove-from-channel', messageId: 'ref', threadName: 'Why?' })
  await flushPromises()
  await view.setProps({ messages: [at('reply', 'agent')] })
  expect(document.querySelector('[data-ui-id="remove-from-channel-warning"]')).not.toBeNull()
})

test('confirming the remove dialog calls the app once and clears the request', async () => {
  host([at('ref', 'agent', from)])
  requestAction({ kind: 'remove-from-channel', messageId: 'ref', threadName: 'Why?' })
  await flushPromises()
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-from-channel-confirm"]')!.click()
  await flushPromises()
  expect(removePostedMessage).toHaveBeenCalledTimes(1)
  expect(removePostedMessage).toHaveBeenCalledWith('ref')
  expect(pendingAction.value).toBeNull()
})
