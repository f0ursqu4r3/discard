import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { createTextChannel } from '@/bridge/api'
import CreateTextChannelDialog from './CreateTextChannelDialog.vue'

vi.mock('@/bridge/api', () => ({ createTextChannel: vi.fn() }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open(props: Record<string, unknown> = { projectId: 'p' }) {
  view = mount(CreateTextChannelDialog, {
    attachTo: document.body,
    props: { open: true, ...props },
  })
  await flushPromises()
}
const input = () => document.querySelector<HTMLInputElement>('#text-channel-name')!
const submit = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="create-text-channel-submit"]')!
const preview = () =>
  document.querySelector('[data-ui-id="text-channel-preview"]')!.textContent!.trim()
async function type(value: string) {
  input().value = value
  input().dispatchEvent(new Event('input', { bubbles: true }))
  await flushPromises()
}

test('the dialog has one name field, a name preview, and the help text', async () => {
  await open()
  expect(document.body.textContent).toContain('New text channel')
  expect(document.body.textContent).toContain(
    'Use a text channel for a project conversation that has no branch.',
  )
  expect(document.querySelectorAll('input')).toHaveLength(1)
  expect(preview()).toBe('#')
  expect(submit().disabled).toBe(true)
  await type('  Release   Notes ')
  expect(preview()).toBe('#release-notes')
  expect(submit().disabled).toBe(false)
})

test('a name with no usable characters keeps Create disabled', async () => {
  await open()
  await type(' -- ')
  expect(preview()).toBe('#')
  expect(submit().disabled).toBe(true)
})

test('create sends the typed name, closes the dialog, and reports the new channel', async () => {
  vi.mocked(createTextChannel).mockResolvedValue('new-id')
  await open()
  await type('Release Notes')
  submit().click()
  await flushPromises()
  expect(createTextChannel).toHaveBeenCalledWith('p', 'Release Notes')
  expect(view!.emitted('created')).toEqual([['new-id']])
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('a server error shows in the dialog and keeps the name for another try', async () => {
  vi.mocked(createTextChannel).mockRejectedValueOnce('A channel with this name already exists.')
  await open()
  await type('plans')
  submit().click()
  await flushPromises()
  expect(document.querySelector('[role="alert"]')?.textContent).toBe(
    'A channel with this name already exists.',
  )
  expect(input().value).toBe('plans')
  expect(view!.emitted('created')).toBeUndefined()
  expect(view!.emitted('update:open')).toBeUndefined()
  vi.mocked(createTextChannel).mockResolvedValueOnce('id')
  await type('plans-2')
  submit().click()
  await flushPromises()
  expect(createTextChannel).toHaveBeenLastCalledWith('p', 'plans-2')
  expect(document.querySelector('[role="alert"]')).toBeNull()
  expect(view!.emitted('created')).toEqual([['id']])
})

test('create needs a project', async () => {
  await open({ projectId: null })
  await type('plans')
  expect(submit().disabled).toBe(true)
  expect(createTextChannel).not.toHaveBeenCalled()
})

test('opening the dialog again clears the name and the error', async () => {
  vi.mocked(createTextChannel).mockRejectedValue('Bad name')
  await open()
  await type('plans')
  submit().click()
  await flushPromises()
  await view!.setProps({ open: false })
  await view!.setProps({ open: true })
  await flushPromises()
  expect(input().value).toBe('')
  expect(document.querySelector('[role="alert"]')).toBeNull()
})
