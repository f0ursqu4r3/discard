import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { archiveProjectChannel } from '@/bridge/api'
import ArchiveChannelDialog from '../ArchiveChannelDialog.vue'

vi.mock('@/bridge/api', () => ({ archiveProjectChannel: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(ArchiveChannelDialog, {
    attachTo: document.body,
    props: { open: true, projectId: 'p', channelId: 'c', name: 'plans' },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="archive-channel-confirm"]')!
const cancelButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="archive-channel-cancel"]')!
const content = () => document.querySelector<HTMLElement>('[data-ui-id="archive-channel-dialog"]')!
const press = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))

test('the dialog names the channel, puts the focus on Cancel and labels the button Archive Channel', async () => {
  await open()
  expect(document.body.textContent).toContain('Archive channel?')
  expect(document.body.textContent).toContain('“plans”')
  expect(document.activeElement).toBe(cancelButton())
  expect(confirmButton().textContent!.trim()).toBe('Archive Channel')
  expect(archiveProjectChannel).not.toHaveBeenCalled()
})

test('Archive Channel archives once and closes the dialog', async () => {
  vi.mocked(archiveProjectChannel).mockResolvedValue({ archived: true, items: [] })
  await open()
  confirmButton().click()
  await flushPromises()
  expect(archiveProjectChannel).toHaveBeenCalledTimes(1)
  expect(archiveProjectChannel).toHaveBeenCalledWith('p', 'c', false)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('files that the archive deletes need a second, explicit confirm', async () => {
  vi.mocked(archiveProjectChannel)
    .mockResolvedValueOnce({ archived: false, items: ['wip.txt', 'notes/draft.md'] })
    .mockResolvedValueOnce({ archived: true, items: [] })
  await open()
  confirmButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toBeUndefined()
  const items = document.querySelector('[data-ui-id="archive-channel-items"]')!
  expect(items.textContent).toContain('Archiving will delete these items')
  expect(items.textContent).toContain('wip.txt')
  expect(items.textContent).toContain('notes/draft.md')
  expect(confirmButton().textContent!.trim()).toBe('Archive and delete')
  expect(document.activeElement).toBe(cancelButton())
  confirmButton().click()
  await flushPromises()
  expect(archiveProjectChannel).toHaveBeenNthCalledWith(1, 'p', 'c', false)
  expect(archiveProjectChannel).toHaveBeenNthCalledWith(2, 'p', 'c', true)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('Enter confirms, but a focused Cancel button keeps its own action', async () => {
  vi.mocked(archiveProjectChannel).mockResolvedValue({ archived: true, items: [] })
  await open()
  press(cancelButton(), 'Enter')
  await flushPromises()
  expect(archiveProjectChannel).not.toHaveBeenCalled()
  press(content(), 'Enter')
  await flushPromises()
  expect(archiveProjectChannel).toHaveBeenCalledTimes(1)
})

test('Cancel closes the dialog and archives nothing', async () => {
  await open()
  cancelButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(archiveProjectChannel).not.toHaveBeenCalled()
})

test('a refusal shows a toast and keeps the dialog open', async () => {
  vi.mocked(archiveProjectChannel).mockRejectedValueOnce(
    'A turn runs in this channel. Stop it first.',
  )
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not archive channel', {
    description: 'A turn runs in this channel. Stop it first.',
  })
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(confirmButton().disabled).toBe(false)
})
