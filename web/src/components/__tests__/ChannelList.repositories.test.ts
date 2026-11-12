import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test } from 'vite-plus/test'
import ChannelList from '../ChannelList.vue'
import type { BranchRowDto, ProjectChannelDto } from '../../bridge/types'

const repositories = [
  { id: '/project/frontend', label: 'frontend', path: '/project/frontend' },
  { id: '/project/backend', label: 'backend', path: '/project/backend' },
]
const branches: BranchRowDto[] = repositories.map((repo) => ({
  repository: repo.id,
  key: JSON.stringify([repo.id, 'main']),
  label: 'main',
  kind: 'main',
  depth: 0,
  current: false,
  collapsed: false,
  hasChildren: false,
}))
const props = { projectId: 'project', projectPath: '/project', repositories, branches }

beforeEach(() => localStorage.clear())

test('each repository has its own branches and worktrees with distinct actions', async () => {
  const list = mount(ChannelList, { props })
  const sections = list.findAll('[data-ui-id="repository-section"]')
  expect(sections).toHaveLength(2)
  for (const [index, section] of sections.entries()) {
    expect(section.get('[data-ui-id="repository-toggle"]').text()).toBe(repositories[index]!.label)
    expect(section.findAll('[data-ui-id^="branches-row-"]')).toHaveLength(1)
    expect(section.findAll('[data-ui-id^="worktrees-row-"]')).toHaveLength(1)
    await section.get('[data-ui-id^="branches-row-"]').trigger('click')
    expect(list.emitted('openBranch')?.[index]).toEqual([branches[index]!.key])
  }
  await sections[0]!.get('[data-ui-id="branches-toggle"]').trigger('click')
  expect(sections[0]!.get('[data-ui-id="branches-toggle"]').attributes('aria-expanded')).toBe(
    'false',
  )
  expect(sections[1]!.get('[data-ui-id="branches-toggle"]').attributes('aria-expanded')).toBe(
    'true',
  )
  list.unmount()
})

test('repository collapse state survives remounting and does not hide a single repository', async () => {
  let list = mount(ChannelList, { props })
  await list.findAll('[data-ui-id="repository-toggle"]')[0]!.trigger('click')
  list.unmount()
  list = mount(ChannelList, { props })
  expect(list.findAll('[data-ui-id="repository-toggle"]')[0]!.attributes('aria-expanded')).toBe(
    'false',
  )
  expect(list.findAll('[data-ui-id="repository-toggle"]')[1]!.attributes('aria-expanded')).toBe(
    'true',
  )
  await list.setProps({ repositories: repositories.slice(0, 1), branches: branches.slice(0, 1) })
  await flushPromises()
  expect(list.find('[data-ui-id="repository-toggle"]').exists()).toBe(false)
  expect(list.findAll('[data-ui-id^="branches-row-"]')).toHaveLength(1)
  await list.setProps({ repositories })
  expect(list.findAll('[data-ui-id="repository-toggle"]')[0]!.attributes('aria-expanded')).toBe(
    'false',
  )
  list.unmount()
})

test('collapse state belongs to the project and loading is distinct from no repositories', async () => {
  const list = mount(ChannelList, { props })
  await list.findAll('[data-ui-id="repository-toggle"]')[0]!.trigger('click')
  await list.setProps({ projectPath: '/other' })
  expect(list.findAll('[data-ui-id="repository-toggle"]')[0]!.attributes('aria-expanded')).toBe(
    'true',
  )
  await list.setProps({ repositories: [], branches: [], repositoriesLoading: true })
  expect(list.text()).toContain('Scanning repositories')
  expect(list.text()).not.toContain('No repositories found')
  await list.setProps({ repositoriesLoading: false })
  expect(list.text()).toContain('No repositories found')
  expect(list.find('[data-ui-id="branches-toggle"]').exists()).toBe(false)
  list.unmount()
})

test('a refresh keeps the rows and their expanded state instead of showing the loading text', async () => {
  const list = mount(ChannelList, { props })
  await list.findAll('[data-ui-id="repository-toggle"]')[0]!.trigger('click')
  const toggle = list.findAll('[data-ui-id="repository-toggle"]')[0]!.element
  await list.setProps({ repositoriesLoading: true })
  expect(list.text()).not.toContain('Scanning repositories')
  expect(list.findAll('[data-ui-id="repository-section"]')).toHaveLength(2)
  expect(list.findAll('[data-ui-id="repository-toggle"]')[0]!.element).toBe(toggle)
  expect(list.findAll('[data-ui-id="repository-toggle"]')[0]!.attributes('aria-expanded')).toBe(
    'false',
  )
  list.unmount()
})

test('unread branch indicators belong only to their repository', () => {
  const channel: ProjectChannelDto = {
    id: 'frontend-main',
    projectId: 'project',
    name: 'frontend/main',
    agentId: 'agent',
    workspaceMode: 'worktree',
    kind: 'branch',
    branch: 'main',
    repository: repositories[0]!.id,
    archived: false,
    archivedInto: null,
    hasMessages: false,
  }
  const list = mount(ChannelList, {
    props: {
      ...props,
      projectChannels: [channel],
      channels: [
        {
          id: channel.id,
          projectId: 'project',
          name: channel.name,
          agentId: 'agent',
          agent: 'Agent',
          provider: 'codex',
          workspaceMode: 'worktree',
          running: false,
          unread: true,
          pendingApprovals: 0,
        },
      ],
    },
  })
  const sections = list.findAll('[data-ui-id="repository-section"]')
  expect(sections[0]!.findAll('[data-ui-id="channel-unread-pip"]')).toHaveLength(2)
  expect(sections[1]!.findAll('[data-ui-id="channel-unread-pip"]')).toHaveLength(0)
  expect(list.find('[data-ui-id="archived-channel-navigation"]').exists()).toBe(false)
  list.unmount()
})

test('detached worktree unread markers use the saved path and commit, then clear on read', async () => {
  const base = {
    depth: 0,
    current: false,
    collapsed: false,
    hasChildren: false,
    kind: 'worktree',
    detached: true,
    label: 'abcdef1',
  }
  const channel: ProjectChannelDto = {
    id: 'detached',
    projectId: 'project',
    name: 'frontend/abcdef1',
    agentId: 'agent',
    workspaceMode: 'worktree',
    kind: 'legacy',
    branch: null,
    archived: false,
    archivedInto: null,
    hasMessages: false,
    worktree: { path: '/detached/first', branch: 'abcdef1' },
  }
  const runtime = {
    id: channel.id,
    projectId: 'project',
    name: channel.name,
    agentId: 'agent',
    agent: 'Agent',
    provider: 'codex',
    workspaceMode: 'worktree',
    running: false,
    unread: true,
    pendingApprovals: 0,
  }
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      projectPath: '/project',
      branches: [
        { ...base, key: 'first', worktree: '/detached/first' },
        { ...base, key: 'second', worktree: '/detached/second' },
      ],
      projectChannels: [channel],
      channels: [runtime],
    },
  })
  const unread = (key: string) =>
    list
      .get(`[data-ui-id="worktrees-row-${key}"]`)
      .find('[data-ui-id="channel-unread-pip"]')
      .exists()
  expect(unread('first')).toBe(true)
  expect(unread('second')).toBe(false)
  await list.setProps({ channels: [{ ...runtime, unread: false }] })
  expect(unread('first')).toBe(false)
  await list.setProps({
    channels: [runtime],
    projectChannels: [{ ...channel, worktree: { path: '/detached/first', branch: 'oldhead' } }],
  })
  expect(unread('first')).toBe(false)
  await list.setProps({ projectChannels: [{ ...channel, projectId: 'other' }] })
  expect(unread('first')).toBe(false)
  await list.setProps({ projectChannels: [{ ...channel, archived: true }] })
  expect(unread('first')).toBe(false)
  list.unmount()
})

test('worktree children have ordered tree prefixes and detached worktrees have none', () => {
  const row = { ...branches[0]!, worktree: '/one' }
  const list = mount(ChannelList, {
    props: {
      ...props,
      branches: [
        row,
        { ...row, key: 'two', kind: 'worktree', worktree: '/two' },
        { ...row, key: 'detached', detached: true, label: 'abc', worktree: '/detached' },
      ],
    },
  })
  expect(
    list.findAll('[data-branch-key] [data-testid="tree-prefix"]').map((prefix) => prefix.text()),
  ).toEqual(['├', '└'])
  expect(
    list.get('[data-ui-id="detached-worktrees-list"]').find('[data-testid="tree-prefix"]').exists(),
  ).toBe(false)
  list.unmount()
})
