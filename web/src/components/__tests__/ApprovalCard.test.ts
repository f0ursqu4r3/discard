import { expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import ApprovalCard from '../ApprovalCard.vue'
vi.mock('../../bridge/api', () => ({ decideApproval: vi.fn() }))
for (const mode of ['available', 'unavailable']) {
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

test('a saved escape approval shows as a compact row, not a card', async () => {
  const { default: BlockView } = await import('../BlockView.vue')
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
  expect(view.find('[data-ui-id="approval-card"]').exists()).toBe(false)
  expect(view.get('[data-protected="true"]').text()).toContain('file_change')
  expect(view.text()).toContain('/outside')
  expect(view.text()).not.toContain('Always allow')
})

import { decideApproval } from '../../bridge/api'
const commandPrompt = {
  approvalId: 'a',
  tool: 'Bash',
  summary: 'cargo test -p x',
  view: 'pending',
  kind: 'provider_tool',
  alwaysAllow: 'available',
  ruleWords: ['cargo', 'test', '-p', 'x'],
  defaultCut: 2,
  ruleHost: null,
  alwaysAllowNote: null,
}
const commandCard = () => mount(ApprovalCard, { props: { channelId: 'c', prompt: commandPrompt } })
test('Coding keeps its session action without a prefix promise or cut', async () => {
  const view = mount(ApprovalCard, {
    props: {
      channelId: 'c',
      prompt: { ...commandPrompt, ruleWords: null, defaultCut: null },
    },
  })
  expect(view.find('[aria-label="Command prefix"]').exists()).toBe(false)
  expect(view.text()).not.toContain('commands')
  await view
    .findAll('button')
    .find((b) => b.text() === 'Always allow')!
    .trigger('click')
  expect(decideApproval).toHaveBeenLastCalledWith('c', 'a', 'always_allow')
})
test('always allow shows the words with the default cut selected', () => {
  const view = commandCard()
  expect(view.text()).toContain('Always allow `cargo test` commands')
  expect(view.findAll('[aria-pressed="true"]').map((b) => b.text())).toEqual(['cargo', 'test'])
})
test('clicking a word moves the cut', async () => {
  const view = commandCard()
  await view
    .findAll('button')
    .find((b) => b.text() === '-p')!
    .trigger('click')
  expect(view.text()).toContain('Always allow `cargo test -p` commands')
})
test('always allow sends the chosen cut', async () => {
  const view = commandCard()
  await view
    .findAll('button')
    .find((b) => b.text() === '-p')!
    .trigger('click')
  await view
    .findAll('button')
    .find((b) => b.text() === 'Always allow')!
    .trigger('click')
  expect(decideApproval).toHaveBeenLastCalledWith('c', 'a', 'always_allow', 3)
})
test('host prompts preview the exact host', () => {
  const view = mount(ApprovalCard, {
    props: {
      channelId: 'c',
      prompt: {
        ...commandPrompt,
        kind: 'network_domain',
        ruleWords: null,
        ruleHost: 'registry.npmjs.org',
      },
    },
  })
  expect(view.text()).toContain('Always allow network access to `registry.npmjs.org`')
})
test('unavailable shows the note and no Always allow button', () => {
  const view = mount(ApprovalCard, {
    props: {
      channelId: 'c',
      prompt: {
        ...commandPrompt,
        alwaysAllow: 'unavailable',
        alwaysAllowNote: 'This command uses shell syntax that Always allow cannot match.',
      },
    },
  })
  expect(view.text()).toContain('This command uses shell syntax that Always allow cannot match.')
  expect(view.findAll('button').some((b) => b.text() === 'Always allow')).toBe(false)
})
test('no Soon badge remains', () => {
  expect(commandCard().text()).not.toContain('Soon')
})
