import { expect, test } from 'vite-plus/test'
import { archivedChannels, treePrefix } from './branch-navigation'
import type { BranchRowDto, ProjectChannelDto } from '../bridge/types'
test('tree prefixes distinguish middle and last worktree children', () => {
  expect(treePrefix(false)).toBe('├')
  expect(treePrefix(true)).toBe('└')
})

const row = (label: string, extra: Partial<BranchRowDto> = {}): BranchRowDto => ({
  key: label,
  label,
  kind: 'branch',
  depth: 0,
  current: false,
  collapsed: false,
  hasChildren: false,
  repository: '',
  ...extra,
})
const channel = (id: string, extra: Partial<ProjectChannelDto> = {}): ProjectChannelDto => ({
  id,
  projectId: 'p',
  name: id,
  agentId: 'a',
  workspaceMode: 'main_checkout',
  kind: 'branch',
  branch: id,
  repository: '',
  archived: false,
  archivedInto: null,
  hasMessages: false,
  ...extra,
})
const ids = (channels: ProjectChannelDto[]) => channels.map((entry) => entry.id)

test('Archived holds archived, legacy, and archived legacy channels', () => {
  const channels = [
    channel('archived', { archived: true }),
    channel('legacy', { kind: 'legacy', branch: null }),
    channel('history', { kind: 'archived_legacy', archived: true, branch: null }),
    channel('archived-text', { kind: 'text', archived: true, branch: null }),
    channel('archived-general', { kind: 'general', archived: true, branch: null }),
  ]
  expect(ids(archivedChannels(channels, []))).toEqual([
    'archived',
    'legacy',
    'history',
    'archived-text',
    'archived-general',
  ])
})

test('Archived never holds an active general or text channel', () => {
  const channels = [
    channel('general', { kind: 'general', branch: null, hasMessages: true }),
    channel('plans', { kind: 'text', branch: null, hasMessages: true }),
  ]
  expect(archivedChannels(channels, [])).toEqual([])
})

test('a deleted branch shows in Archived only when its channel has messages', () => {
  const channels = [
    channel('gone-with-messages', { hasMessages: true }),
    channel('gone-empty', { hasMessages: false }),
    channel('listed', { hasMessages: true }),
    channel('listed-elsewhere', { hasMessages: true, repository: 'sub' }),
  ]
  const branches = [row('listed'), row('listed-elsewhere', { repository: 'other' })]
  expect(ids(archivedChannels(channels, branches))).toEqual([
    'gone-with-messages',
    'listed-elsewhere',
  ])
})

test('a branch row of the same name in the same repository keeps its channel out of Archived', () => {
  const channels = [channel('feature', { repository: 'sub', hasMessages: true })]
  expect(archivedChannels(channels, [row('feature', { repository: 'sub' })])).toEqual([])
})

test('a detached worktree row does not count as a Git branch', () => {
  const channels = [channel('feature', { hasMessages: true })]
  expect(ids(archivedChannels(channels, [row('feature', { detached: true })]))).toEqual(['feature'])
})
