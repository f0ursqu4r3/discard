import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import MoveRequestDetails from '../MoveRequestDetails.vue'
import type { MoveDetails } from '@/bridge/api'
const details: MoveDetails = {
  target: { kind: 'channel', channelId: 'ch', name: 'feature' },
  handoff: {
    goal: 'Ship the importer',
    constraints: [],
    decisions: ['Use CSV'],
    openQuestions: ['Which encoding?'],
  },
  sourceMessageId: 'm1',
}
test('shows an existing channel target', () => {
  const view = mount(MoveRequestDetails, { props: { details } })
  expect(view.text()).toContain('To')
  expect(view.text()).toContain('#feature')
})
test('shows a new branch with its base and short commit', () => {
  const view = mount(MoveRequestDetails, {
    props: {
      details: {
        ...details,
        target: {
          kind: 'new_branch',
          repository: 'repo',
          repoPath: '/repo',
          branch: 'importer',
          base: 'main',
          baseCommit: '1a2b3c4d5e6f',
        },
      },
    },
  })
  expect(view.text()).toContain('New branch importer')
  expect(view.text()).toContain('main at 1a2b3c4')
  expect(view.text()).not.toContain('1a2b3c4d')
})
const newBranch = (repository: string): MoveDetails => ({
  ...details,
  target: {
    kind: 'new_branch',
    repository,
    repoPath: '/repo',
    branch: 'importer',
    base: 'main',
    baseCommit: '1a2b3c4d5e6f',
  },
})
test('a root repository shows no repository row', () => {
  const view = mount(MoveRequestDetails, { props: { details: newBranch('') } })
  expect(view.text()).toContain('New branch importer')
  expect(view.text()).toContain('main at 1a2b3c4')
  expect(view.text()).not.toContain('Repository')
  expect(view.text()).not.toContain(' in ,')
})
test('a nested repository shows its repository row', () => {
  const view = mount(MoveRequestDetails, { props: { details: newBranch('tools') } })
  expect(view.text()).toContain('Repository')
  expect(view.find('dd:nth-of-type(2) code').text()).toBe('tools')
  expect(view.text()).toContain('New branch importer')
  expect(view.text()).toContain('main at 1a2b3c4')
})
test('shows each handoff section that has content', () => {
  const view = mount(MoveRequestDetails, { props: { details } })
  expect(view.text()).toContain('Goal')
  expect(view.text()).toContain('Decisions')
  expect(view.text()).toContain('Open questions')
  expect(view.text()).not.toContain('Constraints')
})
test('handoff text renders as plain text', () => {
  const goal = '<img src=x onerror=alert(1)> **bold**'
  const view = mount(MoveRequestDetails, {
    props: {
      details: { ...details, handoff: { ...details.handoff, goal, decisions: [goal] } },
    },
  })
  expect(view.text().split(goal)).toHaveLength(3)
  expect(view.find('img').exists()).toBe(false)
  expect(view.find('strong').exists()).toBe(false)
})
test('lists the attachments that the handoff carries', () => {
  const view = mount(MoveRequestDetails, {
    props: { details: { ...details, attachments: [{ id: 'a1', name: 'spec.pdf' }] } },
  })
  expect(view.text()).toContain('Attachments')
  expect(view.text()).toContain('spec.pdf')
  expect(mount(MoveRequestDetails, { props: { details } }).text()).not.toContain('Attachments')
})
