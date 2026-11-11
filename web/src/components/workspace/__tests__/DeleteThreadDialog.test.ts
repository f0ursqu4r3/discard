import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { deleteThread, threadPostedCount } from '@/bridge/api'
import DeleteThreadDialog from '../DeleteThreadDialog.vue'

vi.mock('@/bridge/api', () => ({ deleteThread: vi.fn(), threadPostedCount: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(threadPostedCount).mockResolvedValue(0)
})
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(DeleteThreadDialog, {
    attachTo: document.body,
    props: { open: true, threadId: 't', name: 'Why a thread?' },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="delete-thread-confirm"]')!
const cancelButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="delete-thread-cancel"]')!
const posted = () => document.querySelector('[data-ui-id="delete-thread-posted"]')
const content = () => document.querySelector<HTMLElement>('[data-ui-id="delete-thread-dialog"]')!
const press = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))

test('the dialog names the thread and puts the focus on Cancel', async () => {
  await open()
  expect(document.body.textContent).toContain('Delete thread?')
  expect(document.body.textContent).toContain('“Why a thread?”')
  expect(document.activeElement).toBe(cancelButton())
  expect(confirmButton().textContent!.trim()).toBe('Delete thread')
  expect(threadPostedCount).toHaveBeenCalledWith('t')
})

test('the dialog says how many posted messages stay only when the count is above zero', async () => {
  await open()
  expect(posted()).toBeNull()
  view!.unmount()
  document.body.innerHTML = ''
  vi.mocked(threadPostedCount).mockResolvedValue(1)
  await open()
  expect(posted()!.textContent!.trim()).toBe(
    '1 posted message stays in the channel as a normal message.',
  )
  view!.unmount()
  document.body.innerHTML = ''
  vi.mocked(threadPostedCount).mockResolvedValue(3)
  await open()
  expect(posted()!.textContent!.trim()).toBe(
    '3 posted messages stay in the channel as normal messages.',
  )
})

test('a failed count does not block the delete', async () => {
  vi.mocked(threadPostedCount).mockRejectedValue('no')
  await open()
  expect(posted()).toBeNull()
  expect(confirmButton().disabled).toBe(false)
})

test('Delete calls the app and closes the dialog', async () => {
  vi.mocked(deleteThread).mockResolvedValue(0)
  await open()
  confirmButton().click()
  await flushPromises()
  expect(deleteThread).toHaveBeenCalledWith('t')
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('Enter confirms, but a focused Cancel button keeps its own action', async () => {
  vi.mocked(deleteThread).mockResolvedValue(0)
  await open()
  press(cancelButton(), 'Enter')
  await flushPromises()
  expect(deleteThread).not.toHaveBeenCalled()
  press(content(), 'Enter')
  await flushPromises()
  expect(deleteThread).toHaveBeenCalledWith('t')
})

test('Cancel and Escape close the dialog and delete nothing', async () => {
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
  expect(deleteThread).not.toHaveBeenCalled()
})

test('a backend error shows a toast and keeps the dialog open', async () => {
  vi.mocked(deleteThread).mockRejectedValueOnce('A turn runs in this thread.')
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not delete thread', {
    description: 'A turn runs in this thread.',
  })
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(confirmButton().disabled).toBe(false)
})
