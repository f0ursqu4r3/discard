import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { createGitBranch, renameGitBranch } from '@/bridge/api'
import BranchNameDialog from '../BranchNameDialog.vue'

vi.mock('@/bridge/api', () => ({ createGitBranch: vi.fn(), renameGitBranch: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
let view: ReturnType<typeof mount> | undefined
beforeEach(() => vi.resetAllMocks())
afterEach(() => {
  view?.unmount()
  document.body.innerHTML = ''
})
async function open(mode: 'rename' | 'create') {
  view = mount(BranchNameDialog, {
    attachTo: document.body,
    props:
      mode === 'rename'
        ? { open: true, mode, projectId: 'p', repository: '', branch: 'feature/one' }
        : { open: true, mode, projectId: 'p', repository: '', worktree: '/tree', from: 'main' },
  })
  await flushPromises()
}
const input = (mode: string) => document.querySelector<HTMLInputElement>(`#${mode}-branch-name`)!
const submit = (mode: string) =>
  document.querySelector<HTMLButtonElement>(`[data-ui-id="${mode}-branch-submit"]`)!
const form = (mode: string) => document.querySelector<HTMLFormElement>(`#${mode}-branch-form`)!
const error = (mode: string) => document.querySelector(`[data-ui-id="${mode}-branch-error"]`)
async function type(mode: string, value: string) {
  input(mode).value = value
  input(mode).dispatchEvent(new Event('input', { bubbles: true }))
  await flushPromises()
}
const confirm = async (mode: string) => {
  form(mode).dispatchEvent(new Event('submit', { cancelable: true }))
  await flushPromises()
}

test('the rename field starts with the current name, selected', async () => {
  await open('rename')
  expect(document.body.textContent).toContain('Rename branch')
  expect(input('rename').value).toBe('feature/one')
  expect(document.activeElement).toBe(input('rename'))
  expect(input('rename').selectionStart).toBe(0)
  expect(input('rename').selectionEnd).toBe('feature/one'.length)
  expect(submit('rename').textContent!.trim()).toBe('Rename Branch')
})

test('an empty name keeps Rename Branch disabled and sends nothing', async () => {
  await open('rename')
  await type('rename', '   ')
  expect(submit('rename').disabled).toBe(true)
  await confirm('rename')
  expect(renameGitBranch).not.toHaveBeenCalled()
})

test('rename sends the typed name, then closes the dialog', async () => {
  vi.mocked(renameGitBranch).mockResolvedValue('feature/two')
  await open('rename')
  await type('rename', 'feature/two')
  await confirm('rename')
  expect(renameGitBranch).toHaveBeenCalledWith('p', '', 'feature/one', 'feature/two')
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('the reason of the app shows in the dialog, which stays open with the name intact', async () => {
  vi.mocked(renameGitBranch).mockRejectedValueOnce('A branch named `taken` already exists.')
  await open('rename')
  await type('rename', 'taken')
  await confirm('rename')
  expect(error('rename')!.textContent).toContain('A branch named `taken` already exists.')
  expect(view!.emitted('update:open')).toBeUndefined()
  expect(input('rename').value).toBe('taken')
  expect(submit('rename').disabled).toBe(false)
  // Typing again clears the old reason.
  await type('rename', 'other')
  expect(error('rename')).toBeNull()
})

test('an unchanged name goes to the app, which refuses it with a reason', async () => {
  vi.mocked(renameGitBranch).mockRejectedValueOnce('Enter a name that is not the current name.')
  await open('rename')
  await confirm('rename')
  expect(renameGitBranch).toHaveBeenCalledWith('p', '', 'feature/one', 'feature/one')
  expect(error('rename')!.textContent).toContain('not the current name')
})

test('the create field starts empty and creates the branch in the working tree', async () => {
  vi.mocked(createGitBranch).mockResolvedValue('topic')
  await open('create')
  expect(document.body.textContent).toContain('Create branch')
  expect(document.body.textContent).toContain('main')
  expect(input('create').value).toBe('')
  expect(submit('create').disabled).toBe(true)
  expect(submit('create').textContent!.trim()).toBe('Create Branch')
  await type('create', ' topic ')
  expect(submit('create').disabled).toBe(false)
  await confirm('create')
  expect(createGitBranch).toHaveBeenCalledWith('p', '', '/tree', ' topic ')
  expect(view!.emitted('update:open')).toEqual([[false]])
})

test('create shows the reason of the app and stays open', async () => {
  vi.mocked(createGitBranch).mockRejectedValueOnce('`two words` is not a valid branch name.')
  await open('create')
  await type('create', 'two words')
  await confirm('create')
  expect(error('create')!.textContent).toContain('not a valid branch name')
  expect(view!.emitted('update:open')).toBeUndefined()
})
