import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Workspaces from './pages/Workspaces.vue'
import Logs from './pages/Logs.vue'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

const worktree = (path: string, branch: string, exists = true) => ({ path, branch, exists })
const channel = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  mode: 'Worktree',
  archived: false,
  current: false,
  worktree: null,
  ...extra,
})

beforeEach(() => {
  vi.clearAllMocks()
})

test('workspaces sorts the current project first and archived channels last', async () => {
  invoke.mockImplementation(async () => [
    { name: 'Other', path: '/Users/ada/other', exists: true, channels: [channel('a')] },
    {
      name: 'Mine',
      path: '/Users/ada/mine',
      exists: false,
      channels: [
        channel('old', { archived: true }),
        channel('main', { mode: 'Main checkout', current: true }),
        channel('feat', { worktree: worktree('/Users/ada/mine-wt/feat', 'ada/feat', false) }),
      ],
    },
  ])
  const view = mount(Workspaces)
  await flushPromises()
  const cards = view.findAll('section')
  expect(cards[0].get('h2').text()).toBe('Mine')
  expect(cards[0].text()).toContain('3 channels')
  expect(cards[0].text()).toContain('missing')
  expect(cards[0].get('[title="/Users/ada/mine"]').text()).toBe('~/mine')
  const names = cards[0].findAll('li').map((li) => li.get('[title]').attributes('title'))
  expect(names).toEqual(['main', 'feat', 'old'])
  expect(cards[0].findAll('li')[1].text()).toContain('~/mine-wt/feat')
  expect(cards[0].findAll('li')[1].text()).toContain('ada/feat')
})

test('workspaces reveals a worktree and a project with labelled icon buttons', async () => {
  invoke.mockImplementation(async (name: string) =>
    name === 'settings_workspaces'
      ? [
          {
            name: 'Mine',
            path: '/p',
            exists: true,
            channels: [channel('feat', { worktree: worktree('/wt', 'feat') })],
          },
        ]
      : undefined,
  )
  const view = mount(Workspaces)
  await flushPromises()
  await view.get('button[aria-label="Reveal feat"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('settings_reveal_workspace', { path: '/wt' })
  await view.get('button[aria-label="Reveal Mine"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('settings_reveal_workspace', { path: '/p' })
  // The branch equals the channel name, so the branch cell stays empty.
  expect(view.get('li').text()).not.toContain('feat feat')
})

test('workspaces collapses a project body from its header', async () => {
  invoke.mockImplementation(async () => [
    { name: 'Mine', path: '/p', exists: true, channels: [channel('feat')] },
  ])
  const view = mount(Workspaces, { attachTo: document.body })
  await flushPromises()
  expect(view.findAll('li')).toHaveLength(1)
  await view.get('button[aria-expanded]').trigger('click')
  await flushPromises()
  expect(view.findAll('li')).toHaveLength(0)
  view.unmount()
})

test('logs renders parsed rows with date separators and a raw fallback', async () => {
  invoke.mockImplementation(async () => [
    '2026-11-09T23:59:59.100Z Info Other first  line',
    '2026-11-10T00:00:01.250000Z Error MCP boom',
    'not a log line',
    '2026-11-10T00:00:02.000Z Warn Plugin careful',
  ])
  const view = mount(Logs)
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_logs', { level: 'INFO', component: 'All', id: '' })
  expect(view.text()).toContain('4 lines')
  expect(view.findAll('.section-label').map((n) => n.text())).toEqual(['2026-11-09', '2026-11-10'])
  const errorRow = view.findAll('.grid').find((n) => n.text().includes('boom'))!
  expect(errorRow.text()).toContain('00:00:01.250')
  expect(errorRow.text()).toContain('ERROR')
  expect(errorRow.find('.text-danger').text()).toBe('ERROR')
  expect(
    view
      .findAll('.grid')
      .find((n) => n.text().includes('careful'))!
      .find('.text-warning')
      .text(),
  ).toBe('WARN')
  // Extra spaces in the message survive.
  expect(view.findAll('.grid')[0].text()).toContain('first  line')
  expect(view.text()).toContain('not a log line')
  view.unmount()
})

test('logs shows an empty state', async () => {
  invoke.mockImplementation(async () => [])
  const view = mount(Logs)
  await flushPromises()
  expect(view.text()).toContain('no log lines match')
  expect(view.text()).toContain('0 lines')
  view.unmount()
})
