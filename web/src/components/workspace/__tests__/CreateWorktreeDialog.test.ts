import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { toast } from 'vue-sonner'
import { createBranchWorktree, worktreePlan } from '@/bridge/api'
import CreateWorktreeDialog from '../CreateWorktreeDialog.vue'

vi.mock('@/bridge/api', () => ({ createBranchWorktree: vi.fn(), worktreePlan: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(worktreePlan).mockResolvedValue({
    path: '/data/worktrees/repo-1/feature-abc',
    channelId: 'new-id',
  })
})
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open() {
  view = mount(CreateWorktreeDialog, {
    attachTo: document.body,
    props: { open: true, projectId: 'p', repository: '', branch: 'feature' },
  })
  await flushPromises()
}
const confirmButton = () =>
  document.querySelector<HTMLButtonElement>('[data-ui-id="create-worktree-confirm"]')!
const path = () =>
  document.querySelector('[data-ui-id="create-worktree-path"]')!.textContent!.trim()

test('the dialog shows the branch name and the target path, and the button says Create Worktree', async () => {
  await open()
  expect(worktreePlan).toHaveBeenCalledWith('p', '', 'feature')
  expect(document.body.textContent).toContain('feature')
  expect(path()).toBe('/data/worktrees/repo-1/feature-abc')
  expect(confirmButton().textContent!.trim()).toBe('Create Worktree')
  expect(confirmButton().disabled).toBe(false)
  expect(createBranchWorktree).not.toHaveBeenCalled()
})

test('Create Worktree sends the planned channel ID and closes the dialog', async () => {
  vi.mocked(createBranchWorktree).mockResolvedValue('/data/worktrees/repo-1/feature-abc')
  await open()
  confirmButton().click()
  await flushPromises()
  expect(createBranchWorktree).toHaveBeenCalledWith('p', '', 'feature', 'new-id')
  expect(toast.success).toHaveBeenCalled()
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('a Git failure shows an error toast with the Git message and keeps the dialog open', async () => {
  vi.mocked(createBranchWorktree).mockRejectedValueOnce("fatal: 'feature' is already checked out")
  await open()
  confirmButton().click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not create worktree', {
    description: "fatal: 'feature' is already checked out",
  })
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(confirmButton().disabled).toBe(false)
})

test('a branch that cannot get a worktree shows the reason and keeps the button off', async () => {
  vi.mocked(worktreePlan).mockRejectedValue('This branch is checked out in a working tree.')
  await open()
  expect(document.querySelector('[data-ui-id="create-worktree-error"]')!.textContent).toContain(
    'checked out in a working tree',
  )
  expect(confirmButton().disabled).toBe(true)
  confirmButton().click()
  await flushPromises()
  expect(createBranchWorktree).not.toHaveBeenCalled()
})
