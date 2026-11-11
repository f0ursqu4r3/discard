import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { renameThread } from '@/bridge/api'
import RenameThreadDialog from '../RenameThreadDialog.vue'

vi.mock('@/bridge/api', () => ({ renameThread: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open(props: Record<string, unknown> = {}) {
  view = mount(RenameThreadDialog, {
    attachTo: document.body,
    props: { open: true, threadId: 't', name: 'Why a thread?', ...props },
  })
  await flushPromises()
}
const input = () => document.querySelector<HTMLInputElement>('#rename-thread-name')!
const submit = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="rename-thread-submit"]')!
const form = () => document.querySelector<HTMLFormElement>('#rename-thread-form')!
const count = () =>
  document.querySelector('[data-ui-id="rename-thread-count"]')!.textContent!.trim()
async function type(value: string) {
  input().value = value
  input().dispatchEvent(new Event('input', { bubbles: true }))
  await flushPromises()
}
const confirm = async () => {
  form().dispatchEvent(new Event('submit', { cancelable: true }))
  await flushPromises()
}

test('the dialog has one name field with the current name, focused and selected', async () => {
  await open()
  expect(document.body.textContent).toContain('Rename thread')
  expect(document.querySelectorAll('input')).toHaveLength(1)
  expect(input().value).toBe('Why a thread?')
  expect(document.activeElement).toBe(input())
  expect(input().selectionStart).toBe(0)
  expect(input().selectionEnd).toBe('Why a thread?'.length)
  expect(count()).toBe('13 / 80')
})

test('an empty name or a name over 80 characters keeps Rename disabled', async () => {
  await open()
  await type('   ')
  expect(submit().disabled).toBe(true)
  expect(input().getAttribute('aria-invalid')).toBe('true')
  await type('a'.repeat(81))
  expect(submit().disabled).toBe(true)
  expect(count()).toBe('81 / 80')
  await confirm()
  expect(renameThread).not.toHaveBeenCalled()
  await type('a'.repeat(80))
  expect(submit().disabled).toBe(false)
})

test('Enter in the field sends the typed name and closes the dialog', async () => {
  vi.mocked(renameThread).mockResolvedValue('Latency')
  await open()
  await type('  Latency  ')
  await confirm()
  expect(renameThread).toHaveBeenCalledWith('t', '  Latency  ')
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('an unchanged name closes the dialog with no write', async () => {
  await open()
  await type(' Why a thread? ')
  await confirm()
  expect(renameThread).not.toHaveBeenCalled()
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('Cancel and Escape close the dialog with no write', async () => {
  await open()
  const cancel = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!
  cancel.click()
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  view!.unmount()
  document.body.innerHTML = ''
  await open()
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(renameThread).not.toHaveBeenCalled()
})

test('a backend error shows a toast and keeps the dialog open with the input intact', async () => {
  vi.mocked(renameThread).mockRejectedValueOnce('This thread is unavailable.')
  await open()
  await type('Latency')
  await confirm()
  expect(toast.error).toHaveBeenCalledWith('Could not rename thread', {
    description: 'This thread is unavailable.',
  })
  expect(input().value).toBe('Latency')
  expect(view!.emitted('update:open')).toBeUndefined()
  vi.mocked(renameThread).mockResolvedValueOnce('Latency')
  await confirm()
  expect(renameThread).toHaveBeenCalledTimes(2)
  expect(view!.emitted('update:open')).toEqual([[false]])
})
