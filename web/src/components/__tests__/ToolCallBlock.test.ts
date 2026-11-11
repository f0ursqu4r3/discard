import { afterEach, expect, test } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import ToolCallBlock from './ToolCallBlock.vue'
import BlockView from './BlockView.vue'
import MessageRow from './MessageRow.vue'
const views: ReturnType<typeof mount>[] = []
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
for (const [label, output, running, isError] of [
  ['Running', null, true, undefined],
  ['Awaiting result', null, false, undefined],
  ['Succeeded', 'ok', false, false],
  ['Failed', 'boom', false, true],
  ['Returned', '', false, undefined],
] as const) {
  test(`shows the ${label} mark`, () => {
    const view = mount(ToolCallBlock, {
      props: { name: 'Bash', input: { command: 'ls' }, output, running, isError },
    })
    views.push(view)
    expect(view.find(`[aria-label="${label}"]`).exists()).toBe(true)
    expect(view.get('button').text()).toContain('$')
    expect(view.get('button').attributes('aria-expanded')).toBe('false')
  })
}
test('a stored block without is_error is returned but never succeeded', () => {
  const view = mount(BlockView, {
    props: {
      channelId: 'channel',
      block: { type: 'tool_call', call_id: 'old', name: 'Bash', input: {}, output: 'done' },
    },
  })
  views.push(view)
  expect(view.find('[aria-label="Returned"]').exists()).toBe(true)
  expect(view.find('[aria-label="Succeeded"]').exists()).toBe(false)
})
test('only a live running row animates a pending tool call', async () => {
  const view = mount(MessageRow, {
    props: {
      channelId: 'channel',
      showCursor: true,
      message: {
        id: 'live',
        role: 'agent',
        continuesGroup: false,
        blocks: [{ type: 'tool_call', call_id: 'c', name: 'Bash', input: {}, output: null }],
      },
    },
  })
  views.push(view)
  expect(view.find('[aria-label="Running"]').exists()).toBe(true)
  await view.setProps({ showCursor: false })
  expect(view.find('[aria-label="Running"]').exists()).toBe(false)
  expect(view.find('[aria-label="Awaiting result"]').exists()).toBe(true)
})

test('labels sandboxed tool calls', () => {
  const view = mount(BlockView, {
    props: {
      channelId: 'channel',
      block: {
        type: 'tool_call',
        call_id: 's',
        name: 'Bash',
        input: {},
        output: 'ok',
        sandboxed: true,
      },
    },
  })
  views.push(view)
  expect(view.text()).toContain('Ran in sandbox')
})
test('does not label tool calls that were not sandboxed', () => {
  const view = mount(ToolCallBlock, {
    props: { name: 'Bash', input: {}, output: 'ok', sandboxed: false },
  })
  views.push(view)
  expect(view.text()).not.toContain('Ran in sandbox')
})
