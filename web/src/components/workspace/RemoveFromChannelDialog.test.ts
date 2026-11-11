import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { removePostedMessage } from '@/bridge/api'
import RemoveFromChannelDialog from './RemoveFromChannelDialog.vue'

vi.mock('@/bridge/api', () => ({ removePostedMessage: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open(agentRead = false) {
  view = mount(RemoveFromChannelDialog, {
    attachTo: document.body,
    props: { open: true, messageId: 'ref', threadName: 'Why?', agentRead },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-from-channel-confirm"]')!
const cancelButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-from-channel-cancel"]')!
const warning = () => document.querySelector('[data-ui-id="remove-from-channel-warning"]')
const content = () =>
  document.querySelector<HTMLElement>('[data-ui-id="remove-from-channel-dialog"]')!
const press = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))

test('the dialog says that the message stays in the thread and can be posted again', async () => {
  await open()
  expect(document.body.textContent).toContain('Remove from channel?')
  expect(document.body.textContent).toContain('It stays in the thread “Why?”.')
  expect(document.body.textContent).toContain('You can post it again.')
  expect(document.activeElement).toBe(cancelButton())
})

test('the warning shows only when an agent message follows the row', async () => {
  await open(false)
  expect(warning()).toBeNull()
  view!.unmount()
  document.body.innerHTML = ''
  await open(true)
  expect(warning()!.textContent).toContain('The agent already read this message.')
  expect(warning()!.textContent).toContain('Earlier replies can still refer to it.')
  expect(warning()!.textContent).toContain('Later turns will not see it.')
})

test('Remove calls the app with the row ID, with the warning or without it', async () => {
  for (const agentRead of [false, true]) {
    vi.mocked(removePostedMessage).mockResolvedValue(undefined)
    await open(agentRead)
    confirmButton().click()
    await flushPromises()
    expect(removePostedMessage).toHaveBeenLastCalledWith('ref')
    expect(view!.emitted('update:open')).toEqual([[false]])
    view!.unmount()
    document.body.innerHTML = ''
  }
  expect(removePostedMessage).toHaveBeenCalledTimes(2)
})

test('Enter confirms, but a focused Cancel button keeps its own action', async () => {
  vi.mocked(removePostedMessage).mockResolvedValue(undefined)
  await open()
  press(cancelButton(), 'Enter')
  await flushPromises()
  expect(removePostedMessage).not.toHaveBeenCalled()
  press(content(), 'Enter')
  await flushPromises()
  expect(removePostedMessage).toHaveBeenCalledWith('ref')
})

test('Cancel and Escape close the dialog and remove nothing', async () => {
  await open()
  cancelButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  view!.unmount()
  document.body.innerHTML = ''
  await open()
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(removePostedMessage).not.toHaveBeenCalled()
})

test('a backend error shows a toast and keeps the dialog open', async () => {
  vi.mocked(removePostedMessage).mockRejectedValueOnce('This message is gone.')
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not remove the message', {
    description: 'This message is gone.',
  })
  expect(view!.emitted('update:open')).toBeUndefined()
})
