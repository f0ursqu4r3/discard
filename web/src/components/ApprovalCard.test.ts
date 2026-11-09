import { expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import ApprovalCard from './ApprovalCard.vue'
vi.mock('../bridge/api', () => ({ decideApproval: vi.fn() }))
for (const mode of ['available', 'planned', 'unavailable']) {
  test(`Always allow is ${mode}`, () => {
    const view = mount(ApprovalCard, {
      props: {
        channelId: 'c',
        prompt: {
          approvalId: 'a',
          tool: 'Bash',
          summary: 'test',
          view: 'pending',
          kind: 'provider_tool',
          alwaysAllow: mode,
        },
      },
    })
    const button = view.findAll('button').find((b) => b.text().includes('Always allow'))
    expect(!!button).toBe(mode !== 'unavailable')
    if (button) expect(button.attributes('disabled') !== undefined).toBe(mode === 'planned')
    expect(view.text().includes('Soon')).toBe(mode === 'planned')
  })
}
test('escape describes full access and has no Always allow', () => {
  const view = mount(ApprovalCard, {
    props: {
      channelId: 'c',
      prompt: {
        approvalId: 'a',
        tool: 'command',
        summary: 'test',
        view: 'pending',
        kind: 'sandbox_escape',
        alwaysAllow: 'unavailable',
      },
    },
  })
  expect(view.text()).toContain('Runs outside the sandbox: full access as you')
  expect(view.text()).toContain(
    'This command can read and change any file you can, and use the network.',
  )
  expect(view.text()).not.toContain('Always allow')
})

test('saved escape cards retain the full-access warning after live prompts clear', async () => {
  const { default: BlockView } = await import('./BlockView.vue')
  const view = mount(BlockView, {
    props: {
      channelId: 'c',
      block: {
        type: 'approval',
        kind: 'sandbox_escape',
        approval_id: 'saved',
        tool: 'file_change',
        summary: '/outside',
      },
    },
  })
  expect(view.text()).toContain('Runs outside the sandbox: full access as you')
  expect(view.text()).not.toContain('Always allow')
})
