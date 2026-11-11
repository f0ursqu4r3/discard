import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { renameTextChannel } from '@/bridge/api'
import RenameChannelDialog from './RenameChannelDialog.vue'

vi.mock('@/bridge/api', () => ({ renameTextChannel: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(RenameChannelDialog, {
    attachTo: document.body,
    props: { open: true, projectId: 'p', channelId: 'c', name: 'plans' },
  })
  await flushPromises()
}
const input = () => document.querySelector<HTMLInputElement>('#rename-channel-name')!
const submit = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="rename-channel-submit"]')!
const form = () => document.querySelector<HTMLFormElement>('#rename-channel-form')!
const preview = () =>
  document.querySelector('[data-ui-id="rename-channel-preview"]')!.textContent!.trim()
async function type(value: string) {
  input().value = value
  input().dispatchEvent(new Event('input', { bubbles: true }))
  await flushPromises()
}
const confirm = async () => {
  form().dispatchEvent(new Event('submit', { cancelable: true }))
  await flushPromises()
}

test('the dialog has one name field with the current name and a name preview', async () => {
  await open()
  expect(document.body.textContent).toContain('Rename channel')
  expect(document.querySelectorAll('input')).toHaveLength(1)
  expect(input().value).toBe('plans')
  expect(document.activeElement).toBe(input())
  expect(preview()).toBe('#plans')
  await type('  Release   Notes ')
  expect(preview()).toBe('#release-notes')
})

test('a name with no usable characters keeps Rename disabled and sends nothing', async () => {
  await open()
  await type(' -- ')
  expect(preview()).toBe('#')
  expect(submit().disabled).toBe(true)
  await confirm()
  expect(renameTextChannel).not.toHaveBeenCalled()
})

test('rename sends the typed name and closes the dialog', async () => {
  vi.mocked(renameTextChannel).mockResolvedValue('release-notes')
  await open()
  await type('Release Notes')
  await confirm()
  expect(renameTextChannel).toHaveBeenCalledWith('p', 'c', 'Release Notes')
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('an unchanged name closes the dialog with no write', async () => {
  await open()
  await type('PLANS')
  await confirm()
  expect(renameTextChannel).not.toHaveBeenCalled()
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('Cancel and Escape close the dialog with no write', async () => {
  await open()
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(renameTextChannel).not.toHaveBeenCalled()
})

test('a backend error shows a toast and keeps the dialog open with the name intact', async () => {
  vi.mocked(renameTextChannel).mockRejectedValueOnce('A channel with this name already exists.')
  await open()
  await type('notes')
  await confirm()
  expect(toast.error).toHaveBeenCalledWith('Could not rename channel', {
    description: 'A channel with this name already exists.',
  })
  expect(input().value).toBe('notes')
  expect(view!.emitted('update:open')).toBeUndefined()
  vi.mocked(renameTextChannel).mockResolvedValueOnce('notes-2')
  await type('notes-2')
  await confirm()
  expect(renameTextChannel).toHaveBeenLastCalledWith('p', 'c', 'notes-2')
  expect(view!.emitted('update:open')).toEqual([[false]])
})
