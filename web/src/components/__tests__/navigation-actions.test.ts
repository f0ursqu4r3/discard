import { expect, test } from 'vite-plus/test'
import {
  branchMenuState,
  channelBusy,
  channelMenuState,
  isDefaultBranch,
  worktreeMenuState,
} from '../navigation-actions'
import type { BranchRowDto, ChannelDto, ProjectChannelDto } from '../../bridge/types'

const live = (extra: Partial<ChannelDto> = {}): ChannelDto => ({
  id: 'c',
  projectId: 'p',
  agentId: 'a',
  name: 'c',
  agent: 'Agent',
  provider: 'codex',
  workspaceMode: 'main_checkout',
  running: false,
  unread: false,
  pendingApprovals: 0,
  ...extra,
})
const saved = (extra: Partial<ProjectChannelDto> = {}): ProjectChannelDto => ({
  id: 'c',
  projectId: 'p',
  name: 'c',
  agentId: 'a',
  workspaceMode: 'main_checkout',
  kind: 'text',
  branch: null,
  archived: false,
  archivedInto: null,
  hasMessages: false,
  ...extra,
})
const row = (extra: Partial<BranchRowDto> = {}): BranchRowDto => ({
  repository: '',
  key: 'k',
  label: 'main',
  kind: 'branch',
  depth: 0,
  current: false,
  collapsed: false,
  hasChildren: false,
  ...extra,
})

test('a channel is busy while a turn runs, an approval waits, a task runs or an archive runs', () => {
  expect(channelBusy(undefined)).toBe(false)
  expect(channelBusy(live())).toBe(false)
  expect(channelBusy(live({ running: true }))).toBe(true)
  expect(channelBusy(live({ pendingApprovals: 1 }))).toBe(true)
  expect(channelBusy(live({ threadPendingApprovals: 1 }))).toBe(true)
  const thread = {
    id: 't',
    parentChannelId: 'c',
    name: 'T',
    unread: false,
    running: false,
    pendingApprovals: 0,
    kind: 'sub_agent' as const,
    state: null,
  }
  expect(channelBusy(live({ threads: [thread] }))).toBe(false)
  expect(channelBusy(live({ threads: [{ ...thread, running: true }] }))).toBe(true)
  expect(channelBusy(live({ threads: [{ ...thread, state: 'running' }] }))).toBe(true)
  expect(channelBusy(live({ threads: [{ ...thread, pendingApprovals: 1 }] }))).toBe(true)
  expect(channelBusy(live(), true)).toBe(true)
  expect(channelBusy(undefined, true)).toBe(true)
})

test('the default branch is the branch of the main checkout of the same repository', () => {
  const rows = [
    row({ kind: 'main', label: 'main' }),
    row({ kind: 'main', label: 'trunk', repository: '/other' }),
    row({ kind: 'worktree', label: 'topic' }),
  ]
  expect(isDefaultBranch(rows, row({ label: 'main' }))).toBe(true)
  expect(isDefaultBranch(rows, row({ label: 'main', repository: '/other' }))).toBe(false)
  expect(isDefaultBranch(rows, row({ label: 'trunk', repository: '/other' }))).toBe(true)
  expect(isDefaultBranch(rows, row({ label: 'topic' }))).toBe(false)
  expect(
    isDefaultBranch(
      [row({ kind: 'main', label: 'abc1234', detached: true })],
      row({ label: 'abc1234' }),
    ),
  ).toBe(false)
})

test('Archive Channel… is blocked for #general, an archived channel, a busy channel and an unknown channel', () => {
  expect(channelMenuState(saved(), false)).toEqual({ muted: false, archiveDisabled: false })
  expect(channelMenuState(saved({ muted: true }), false).muted).toBe(true)
  expect(channelMenuState(saved({ kind: 'general' }), false).archiveDisabled).toBe(true)
  expect(channelMenuState(saved({ archived: true }), false).archiveDisabled).toBe(true)
  expect(channelMenuState(saved(), true).archiveDisabled).toBe(true)
  expect(channelMenuState(undefined, false).archiveDisabled).toBe(true)
})

test('each branch action follows its own rule', () => {
  const free = { checkedOut: false, isDefault: false, channel: undefined, busy: false }
  expect(branchMenuState(free)).toEqual({
    createWorktreeDisabled: false,
    renameBranchDisabled: false,
    deleteBranchDisabled: false,
  })
  expect(branchMenuState({ ...free, checkedOut: true })).toEqual({
    createWorktreeDisabled: true,
    renameBranchDisabled: false,
    deleteBranchDisabled: true,
  })
  expect(branchMenuState({ ...free, isDefault: true })).toEqual({
    createWorktreeDisabled: false,
    renameBranchDisabled: false,
    deleteBranchDisabled: true,
  })
  expect(branchMenuState({ ...free, busy: true })).toEqual({
    createWorktreeDisabled: false,
    renameBranchDisabled: true,
    deleteBranchDisabled: true,
  })
  expect(
    branchMenuState({ ...free, channel: saved({ worktree: { path: '/w', branch: 'b' } }) }),
  ).toEqual({
    createWorktreeDisabled: true,
    renameBranchDisabled: false,
    deleteBranchDisabled: false,
  })
})

test('the main checkout cannot be removed, and a busy channel blocks both worktree actions', () => {
  expect(worktreeMenuState(row({ kind: 'worktree' }), false)).toEqual({
    createBranchDisabled: false,
    removeWorktreeDisabled: false,
  })
  expect(worktreeMenuState(row({ kind: 'main' }), false)).toEqual({
    createBranchDisabled: false,
    removeWorktreeDisabled: true,
  })
  expect(worktreeMenuState(row({ kind: 'worktree' }), true)).toEqual({
    createBranchDisabled: true,
    removeWorktreeDisabled: true,
  })
})
