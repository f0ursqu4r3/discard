import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { invoke } from '@tauri-apps/api/core'
import {
  archiveProjectChannel,
  branchDeletionInfo,
  createBranchWorktree,
  createGitBranch,
  deleteGitBranch,
  removeGitWorktree,
  renameGitBranch,
  setChannelMuted,
  worktreePlan,
  worktreeRemovalInfo,
} from '../api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
beforeEach(() => vi.mocked(invoke).mockClear())

test('the navigation menu commands send exact IDs, names and force choices', async () => {
  await setChannelMuted('p', 'c', true)
  await archiveProjectChannel('p', 'c')
  await archiveProjectChannel('p', 'c', true)
  await worktreePlan('p', '', 'feature')
  await createBranchWorktree('p', '', 'feature', 'new-id')
  await createBranchWorktree('p', '/repo/sub', 'feature')
  await renameGitBranch('p', '', 'feature', 'feature-2')
  await branchDeletionInfo('p', '', 'feature')
  await deleteGitBranch('p', '', 'feature')
  await deleteGitBranch('p', '', 'feature', true)
  await createGitBranch('p', '', '/tree', 'topic')
  await worktreeRemovalInfo('p', '', '/tree')
  await removeGitWorktree('p', '', '/tree')
  await removeGitWorktree('p', '', '/tree', true)
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['set_channel_muted', { projectId: 'p', channelId: 'c', muted: true }],
    ['archive_project_channel', { projectId: 'p', channelId: 'c', force: false }],
    ['archive_project_channel', { projectId: 'p', channelId: 'c', force: true }],
    ['worktree_plan', { projectId: 'p', repository: '', branch: 'feature' }],
    [
      'create_branch_worktree',
      { projectId: 'p', repository: '', branch: 'feature', channelId: 'new-id' },
    ],
    ['create_branch_worktree', { projectId: 'p', repository: '/repo/sub', branch: 'feature' }],
    [
      'rename_git_branch',
      { projectId: 'p', repository: '', branch: 'feature', newName: 'feature-2' },
    ],
    ['branch_deletion_info', { projectId: 'p', repository: '', branch: 'feature' }],
    ['delete_git_branch', { projectId: 'p', repository: '', branch: 'feature', force: false }],
    ['delete_git_branch', { projectId: 'p', repository: '', branch: 'feature', force: true }],
    ['create_git_branch', { projectId: 'p', repository: '', worktree: '/tree', name: 'topic' }],
    ['worktree_removal_info', { projectId: 'p', repository: '', worktree: '/tree' }],
    ['remove_git_worktree', { projectId: 'p', repository: '', worktree: '/tree', force: false }],
    ['remove_git_worktree', { projectId: 'p', repository: '', worktree: '/tree', force: true }],
  ])
})
