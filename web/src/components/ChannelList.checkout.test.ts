import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import ChannelList from './ChannelList.vue'
import type { BranchRowDto, ProjectChannelDto } from '../bridge/types'
import { groupBranchRows, worktreeName } from './branch-navigation'

const base = { repository: 'repo', depth: 0, current: false, collapsed: false, hasChildren: false }
const branches: BranchRowDto[] = [
  { ...base, key: 'main', label: 'main', kind: 'main', worktree: '/repo' },
  { ...base, key: 'feature', label: 'feature', kind: 'worktree', worktree: '/feature-folder' },
  { ...base, key: 'other', label: 'other', kind: 'branch' },
  {
    ...base,
    key: 'detached',
    label: 'abc123',
    kind: 'worktree',
    worktree: '/detached',
    detached: true,
  },
]
const general: ProjectChannelDto = {
  id: 'general',
  projectId: 'project',
  name: 'general',
  agentId: 'agent',
  workspaceMode: 'main_checkout',
  kind: 'general',
  branch: null,
  archived: false,
  archivedInto: null,
}
const props = {
  projectId: 'project',
  projectPath: '/repo',
  channelId: 'general',
  repositories: [{ id: 'repo', label: 'Repo', path: '/repo' }],
  branches,
  projectChannels: [general],
}

test('worktrees nest under their branches while conversation selection remains separate', async () => {
  const view = mount(ChannelList, { props })
  try {
    expect(view.get('[data-ui-id="project-general-channel"]').attributes('aria-current')).toBe(
      'page',
    )
    expect(view.get('[data-ui-id="branches-toggle"]').text()).toContain('Branch channels 3')
    expect(view.find('[data-ui-id="worktree-count"]').exists()).toBe(false)
    const main = view.get('[data-branch-key="main"]')
    expect(main.get('[data-ui-id="branches-row-main"]').attributes('aria-current')).toBeUndefined()
    expect(main.get('[data-ui-id="worktrees-row-main"]').text()).toBe('repo Main')
    const feature = view.get('[data-branch-key="feature"]')
    expect(feature.get('[data-ui-id="worktrees-row-feature"]').text()).toBe('feature-folder')
    expect(feature.get('[data-ui-id="worktrees-row-feature"]').attributes('title')).toBe(
      '/feature-folder',
    )
    expect(
      view.get('[data-branch-key="other"]').find('[data-ui-id="worktree-child"]').exists(),
    ).toBe(false)
    expect(view.find('[data-ui-id="worktrees-list"]').exists()).toBe(false)
    expect(view.get('[data-ui-id="detached-worktrees-list"]').text()).toContain('Detached')
    expect(
      view
        .get('[data-ui-id="detached-worktrees-list"]')
        .find('[data-ui-id="branches-row-detached"]')
        .exists(),
    ).toBe(false)
    await view.get('[data-ui-id="worktrees-row-feature"]').trigger('click')
    expect(view.emitted('openBranch')).toEqual([['feature']])
    await view.setProps({
      channelId: 'other-channel',
      branches: branches.map((row) => ({ ...row, current: row.key === 'other' })),
    })
    expect(view.get('[data-ui-id="branches-row-other"]').attributes('aria-current')).toBe('page')
    expect(view.findAll('[data-ui-id="worktree-child"]')).toHaveLength(3)
    await view.setProps({ page: 'overview' })
    expect(view.find('[aria-current="page"][data-ui-id^="branches-row-"]').exists()).toBe(false)
    expect(view.findAll('[data-ui-id="worktree-child"]')).toHaveLength(3)
  } finally {
    view.unmount()
  }
})

test('a removed worktree disappears without removing its branch channel', async () => {
  const view = mount(ChannelList, { props: { ...props, branches: [branches[1]!] } })
  try {
    expect(view.find('[data-ui-id="worktree-child"]').exists()).toBe(true)
    await view.setProps({ branches: [{ ...branches[1]!, kind: 'branch', worktree: null }] })
    expect(view.find('[data-ui-id="worktree-child"]').exists()).toBe(false)
    expect(view.get('[data-ui-id="branches-row-feature"]').text()).toBe('feature')
  } finally {
    view.unmount()
  }
})

test('planned sidebar actions are disabled and do not open a conversation', async () => {
  const view = mount(ChannelList, { props })
  try {
    const planned = view
      .findAll('button')
      .filter(
        (button) =>
          button.attributes('aria-label')?.includes('Soon') ||
          button.text().includes('Manage worktrees'),
      )
    expect(planned).toHaveLength(4)
    for (const button of planned) {
      expect(button.attributes('disabled')).toBeDefined()
      await button.trigger('click')
    }
    expect(view.emitted('openBranch')).toBeUndefined()
    expect(view.emitted('action')).toBeUndefined()
  } finally {
    view.unmount()
  }
})

test('multiple checkouts share one branch parent, while detached worktrees stay separate', () => {
  const grouped = groupBranchRows([
    ...branches,
    { ...branches[1]!, worktree: '/second-feature', current: true },
  ])
  expect(grouped.branches).toHaveLength(3)
  expect(
    grouped.branches
      .find((branch) => branch.row.label === 'feature')
      ?.worktrees.map((row) => row.worktree),
  ).toEqual(['/feature-folder', '/second-feature'])
  expect(grouped.branches.find((branch) => branch.row.label === 'feature')?.row.current).toBe(true)
  expect(grouped.detached.map((row) => row.key)).toEqual(['detached'])
  expect(grouped.worktreeCount).toBe(4)
  expect(worktreeName({ ...branches[1]!, worktree: 'C:\\worktrees\\feature\\' })).toBe('feature')
})
