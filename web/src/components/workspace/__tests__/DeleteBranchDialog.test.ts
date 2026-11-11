import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { branchDeletionInfo, deleteGitBranch } from '@/bridge/api'
import DeleteBranchDialog from '../DeleteBranchDialog.vue'

vi.mock('@/bridge/api', () => ({ branchDeletionInfo: vi.fn(), deleteGitBranch: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(branchDeletionInfo).mockResolvedValue(0)
})
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(DeleteBranchDialog, {
    attachTo: document.body,
    props: { open: true, projectId: 'p', repository: '', branch: 'feature' },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="delete-branch-confirm"]')!
const cancelButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="delete-branch-cancel"]')!
const force = () => document.querySelector<HTMLButtonElement>('[data-ui-id="delete-branch-force"]')
const content = () => document.querySelector<HTMLElement>('[data-ui-id="delete-branch-dialog"]')!
const press = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))

test('the dialog names the branch and puts the focus on Cancel', async () => {
  await open()
  expect(branchDeletionInfo).toHaveBeenCalledWith('p', '', 'feature')
  expect(document.body.textContent).toContain('Delete branch?')
  expect(document.body.textContent).toContain('“feature”')
  expect(document.activeElement).toBe(cancelButton())
  expect(confirmButton().textContent!.trim()).toBe('Delete Branch')
})

test('a branch with no unmerged commits needs one confirm and deletes without force', async () => {
  vi.mocked(deleteGitBranch).mockResolvedValue({ deleted: true, unmerged: 0 })
  await open()
  expect(force()).toBeNull()
  expect(document.querySelector('[data-ui-id="delete-branch-warning"]')).toBeNull()
  expect(confirmButton().disabled).toBe(false)
  confirmButton().click()
  await flushPromises()
  expect(deleteGitBranch).toHaveBeenCalledWith('p', '', 'feature', false)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('unmerged commits show a warning with their count and gate the confirm button on a checkbox', async () => {
  vi.mocked(branchDeletionInfo).mockResolvedValue(3)
  vi.mocked(deleteGitBranch).mockResolvedValue({ deleted: true, unmerged: 0 })
  await open()
  const warning = document.querySelector('[data-ui-id="delete-branch-warning"]')!
  expect(warning.textContent).toContain('3 unmerged commits')
  expect(document.body.textContent).toContain('Delete 3 unmerged commits')
  expect(confirmButton().disabled).toBe(true)
  confirmButton().click()
  press(content(), 'Enter')
  await flushPromises()
  expect(deleteGitBranch).not.toHaveBeenCalled()
  force()!.click()
  await flushPromises()
  expect(confirmButton().disabled).toBe(false)
  confirmButton().click()
  await flushPromises()
  expect(deleteGitBranch).toHaveBeenCalledWith('p', '', 'feature', true)
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('one unmerged commit uses the singular', async () => {
  vi.mocked(branchDeletionInfo).mockResolvedValue(1)
  await open()
  expect(document.body.textContent).toContain('Delete 1 unmerged commit')
  expect(document.body.textContent).not.toContain('1 unmerged commits')
})

test('clearing the checkbox turns the confirm button off again', async () => {
  vi.mocked(branchDeletionInfo).mockResolvedValue(2)
  await open()
  force()!.click()
  await flushPromises()
  expect(confirmButton().disabled).toBe(false)
  force()!.click()
  await flushPromises()
  expect(confirmButton().disabled).toBe(true)
})

test('commits that appear after the dialog counted keep the branch and ask again', async () => {
  vi.mocked(deleteGitBranch).mockResolvedValueOnce({ deleted: false, unmerged: 2 })
  await open()
  confirmButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(document.querySelector('[data-ui-id="delete-branch-warning"]')!.textContent).toContain(
    '2 unmerged commits',
  )
  expect(confirmButton().disabled).toBe(true)
})

test('a branch that the app refuses to delete shows the reason and keeps the button off', async () => {
  vi.mocked(branchDeletionInfo).mockRejectedValue('The default branch cannot be deleted.')
  await open()
  expect(document.querySelector('[data-ui-id="delete-branch-refusal"]')!.textContent).toContain(
    'default branch cannot be deleted',
  )
  expect(confirmButton().disabled).toBe(true)
  press(content(), 'Enter')
  await flushPromises()
  expect(deleteGitBranch).not.toHaveBeenCalled()
})

test('a focused Cancel button keeps its own action when Enter is pressed', async () => {
  vi.mocked(deleteGitBranch).mockResolvedValue({ deleted: true, unmerged: 0 })
  await open()
  press(cancelButton(), 'Enter')
  await flushPromises()
  expect(deleteGitBranch).not.toHaveBeenCalled()
  press(content(), 'Enter')
  await flushPromises()
  expect(deleteGitBranch).toHaveBeenCalledTimes(1)
})

test('Cancel closes the dialog and deletes nothing', async () => {
  await open()
  cancelButton().click()
  await flushPromises()
  expect(view!.emitted('update:open')).toEqual([[false]])
  expect(deleteGitBranch).not.toHaveBeenCalled()
})

test('a Git error shows a toast and keeps the dialog open', async () => {
  vi.mocked(deleteGitBranch).mockRejectedValueOnce('error: branch not found')
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not delete branch', {
    description: 'error: branch not found',
  })
  expect(view!.emitted('update:open')).toBeUndefined()
})
