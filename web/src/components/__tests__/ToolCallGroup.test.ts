import { afterEach, expect, test } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import type { BlockDto } from '../../bridge/types'
import MessageRow from '../MessageRow.vue'
import ToolCallBlock from '../ToolCallBlock.vue'
import ContextMenuHost from '../ContextMenuHost.vue'

const views: ReturnType<typeof mount>[] = []
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
const call = (name: string, output: string | null = 'ok', is_error?: boolean): BlockDto => ({
  type: 'tool_call',
  call_id: name,
  name: `mcp__discard-memory__${name}`,
  input: { query: 'needle' },
  output,
  is_error,
})
function row(blocks: BlockDto[], live = false) {
  const view = mount(MessageRow, {
    props: {
      channelId: 'channel',
      showCursor: live,
      message: { id: live ? 'live' : 'saved', role: 'agent', continuesGroup: false, blocks },
    },
  })
  views.push(view)
  return view
}

test('group starts closed, counts labels, and preserves context indexes when opened', async () => {
  const view = row([
    { type: 'markdown', text: 'Before' },
    call('project_search'),
    call('project_read_file'),
    call('project_search'),
  ])
  const trigger = view.get('[aria-label="Show 3 tool calls"]')
  expect(trigger.text()).toContain('3 tool calls')
  expect(trigger.text()).toContain('project search ×2, project read file')
  expect(trigger.attributes('aria-expanded')).toBe('false')
  expect(view.findComponent(ToolCallBlock).exists()).toBe(false)
  await trigger.trigger('click')
  expect(view.get('[aria-label="Hide 3 tool calls"]').attributes('aria-expanded')).toBe('true')
  expect(view.findAllComponents(ToolCallBlock)).toHaveLength(3)
  const targets = view
    .findAllComponents(ContextMenuHost)
    .map((host) => host.props('target'))
    .filter((target) => target?.type === 'tool_call')
  expect(targets.map((target) => target.block)).toEqual([1, 2, 3])
  await view.findAllComponents(ToolCallBlock)[0]!.get('button').trigger('click')
  expect(view.get('[aria-label="Tool output"]').text()).toBe('ok')
})

test('group shows failures with a running call, and keeps open state during appends', async () => {
  const blocks = [call('project_search', 'failed', true), call('project_read_file', null)]
  const view = row(blocks, true)
  expect(view.get('[aria-label="Show 2 tool calls"]').text()).toContain('project read file needle')
  expect(view.find('[aria-label="Running"]').exists()).toBe(true)
  expect(view.get('[aria-label="1 failed"]').text()).toBe('1 failed')
  await view.get('[aria-label="Show 2 tool calls"]').trigger('click')
  await view.setProps({
    message: {
      id: 'live',
      role: 'agent',
      continuesGroup: false,
      blocks: [...blocks, call('project_search', null)],
    },
  })
  expect(view.get('[aria-label="Hide 3 tool calls"]').attributes('aria-expanded')).toBe('true')
})

test('group distinguishes awaiting, failed, succeeded, and unknown results', () => {
  for (const [blocks, label] of [
    [[call('a', null), call('b')], 'Awaiting result'],
    [[call('a', 'error', true), call('b')], '1 failed'],
    [[call('a', 'ok', false), call('b', 'ok', false)], 'All succeeded'],
  ] as const) {
    const view = row([...blocks])
    expect(view.find(`[aria-label="${label}"]`).exists()).toBe(true)
    expect(view.find('[aria-label="Running"]').exists()).toBe(false)
  }
  const unknown = row([call('a'), call('b')])
  expect(unknown.find('[aria-label="All succeeded"]').exists()).toBe(false)
})

test('a live group does not mark an earlier pending call as awaiting', () => {
  const view = row([call('project_search', null), call('project_read_file', 'ok', false)], true)
  expect(view.find('[aria-label="Running"]').exists()).toBe(false)
  expect(view.find('[aria-label="Awaiting result"]').exists()).toBe(false)
  expect(view.find('[aria-label="All succeeded"]').exists()).toBe(false)
})

const approvalFor = (callId: string): BlockDto => ({
  type: 'approval',
  approval_id: `approval-${callId}`,
  call_id: callId,
  tool: 'Bash',
  summary: 'ls',
})
const prompt = (callId: string, view: string) => ({
  kind: 'provider_tool',
  alwaysAllow: 'unavailable',
  approvalId: `approval-${callId}`,
  tool: 'Bash',
  summary: 'ls',
  view,
})
function approvalRow(blocks: BlockDto[], prompts: ReturnType<typeof prompt>[]) {
  const view = mount(MessageRow, {
    props: {
      channelId: 'channel',
      prompts,
      message: { id: 'saved', role: 'agent', continuesGroup: false, blocks },
    },
  })
  views.push(view)
  return view
}

test('an approval between two calls keeps one group and counts the pending ones', () => {
  const view = approvalRow(
    [call('a'), approvalFor('a'), call('b'), approvalFor('b'), call('c')],
    [prompt('a', 'pending'), prompt('b', 'pending')],
  )
  expect(view.findAll('.tool-call-group')).toHaveLength(1)
  expect(view.get('[data-ui-id="approval-count"]').text()).toBe('2 need approval')
})

test('the group header uses the singular for one pending approval', () => {
  const view = approvalRow(
    [call('a'), approvalFor('a'), call('b'), approvalFor('b')],
    [prompt('a', 'approved'), prompt('b', 'pending')],
  )
  expect(view.get('[data-ui-id="approval-count"]').text()).toBe('1 needs approval')
})

test('the group header shows no count when nothing is pending', () => {
  const view = approvalRow([call('a'), approvalFor('a'), call('b')], [prompt('a', 'approved')])
  expect(view.find('[data-ui-id="approval-count"]').exists()).toBe(false)
})
