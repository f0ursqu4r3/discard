import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { removeGitWorktree, worktreeRemovalInfo } from '@/bridge/api'
import RemoveWorktreeDialog from '../RemoveWorktreeDialog.vue'

vi.mock('@/bridge/api', () => ({ removeGitWorktree: vi.fn(), worktreeRemovalInfo: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(worktreeRemovalInfo).mockResolvedValue({ files: [], more: 0 })
})
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(RemoveWorktreeDialog, {
    attachTo: document.body,
    props: { open: true, projectId: 'p', repository: '', worktree: '/data/worktrees/feature-abc' },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-worktree-confirm"]')!
const cancelButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-worktree-cancel"]')!
const force = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="remove-worktree-force"]')
const changes = () => document.querySelector('[data-ui-id="remove-worktree-changes"]')
const content = () => document.querySelector<HTMLElement>('[data-ui-id="remove-worktree-dialog"]')!
const press = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
const files = (count: number) => Array.from({ length: count }, (_, i) => `file-${i}.txt`)

test('the dialog shows the path, says that the branch and its commits stay, and focuses Cancel', async () => {
  await open()
  expect(worktreeRemovalInfo).toHaveBeenCalledWith('p', '', '/data/worktrees/feature-abc')
  expect(document.querySelector('[data-ui-id="remove-worktree-path"]')!.textContent).toContain(
    '/data/worktrees/feature-abc',
  )
  expect(document.body.textContent).toContain('The branch and its commits stay.')
  expect(document.activeElement).toBe(cancelButton())
  expect(confirmButton().textContent!.trim()).toBe('Remove Worktree')
})

test('a clean working tree needs one confirm and is removed without force', async () => {
  vi.mocked(removeGitWorktree).mockResolvedValue({ removed: true, files: [], more: 0 })
  await open()
  expect(changes()).toBeNull()
  expect(force()).toBeNull()
  expect(confirmButton().disabled).toBe(false)
  confirmButton().click()
  await flushPromises()
  expect(removeGitWorktree).toHaveBeenCalledWith('p', '', '/data/worktrees/feature-abc', false)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('changed files are listed and Discard these changes gates the confirm button', async () => {
  vi.mocked(worktreeRemovalInfo).mockResolvedValue({ files: files(3), more: 0 })
  vi.mocked(removeGitWorktree).mockResolvedValue({ removed: true, files: [], more: 0 })
  await open()
  expect(changes()!.textContent).toContain('file-0.txt')
  expect(changes()!.textContent).toContain('file-2.txt')
  expect(document.querySelector('[data-ui-id="remove-worktree-more"]')).toBeNull()
  expect(document.body.textContent).toContain('Discard these changes')
  expect(confirmButton().disabled).toBe(true)
  confirmButton().click()
  press(content(), 'Enter')
  await flushPromises()
  expect(removeGitWorktree).not.toHaveBeenCalled()
  force()!.click()
  await flushPromises()
  expect(confirmButton().disabled).toBe(false)
  confirmButton().click()
  await flushPromises()
  expect(removeGitWorktree).toHaveBeenCalledWith('p', '', '/data/worktrees/feature-abc', true)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('ten files show with the count of the rest', async () => {
  vi.mocked(worktreeRemovalInfo).mockResolvedValue({ files: files(10), more: 5 })
  await open()
  expect(changes()!.querySelectorAll('li')).toHaveLength(10)
  expect(document.querySelector('[data-ui-id="remove-worktree-more"]')!.textContent!.trim()).toBe(
    'and 5 more files',
  )
})

test('one more file uses the singular', async () => {
  vi.mocked(worktreeRemovalInfo).mockResolvedValue({ files: files(10), more: 1 })
  await open()
  expect(document.querySelector('[data-ui-id="remove-worktree-more"]')!.textContent!.trim()).toBe(
    'and 1 more file',
  )
})

test('files that change after the dialog read the status keep the tree and ask again', async () => {
  vi.mocked(removeGitWorktree).mockResolvedValueOnce({ removed: false, files: files(2), more: 0 })
  await open()
  confirmButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(changes()!.textContent).toContain('file-1.txt')
  expect(confirmButton().disabled).toBe(true)
})

test('a working tree that the app refuses to remove shows the reason and keeps the button off', async () => {
  vi.mocked(worktreeRemovalInfo).mockRejectedValue('A turn runs in this channel. Stop it first.')
  await open()
  expect(document.querySelector('[data-ui-id="remove-worktree-refusal"]')!.textContent).toContain(
    'A turn runs',
  )
  expect(confirmButton().disabled).toBe(true)
  press(content(), 'Enter')
  await flushPromises()
  expect(removeGitWorktree).not.toHaveBeenCalled()
})

test('Cancel closes the dialog and removes nothing', async () => {
  await open()
  cancelButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(removeGitWorktree).not.toHaveBeenCalled()
})

test('a Git error shows a toast and keeps the dialog open', async () => {
  vi.mocked(removeGitWorktree).mockRejectedValueOnce('fatal: cannot remove a locked working tree')
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not remove worktree', {
    description: 'fatal: cannot remove a locked working tree',
  })
  expect(view!.emitted('update:open')).toBeUndefined()
})
