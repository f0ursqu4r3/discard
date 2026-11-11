import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import ChangesCard from './ChangesCard.vue'
import type { BlockDto } from '../bridge/types'
const block: Extract<BlockDto, { type: 'changes' }> = {
  type: 'changes',
  root: '/worktree',
  state: 'completed',
  commits: [{ id: '1234567890', subject: 'Change files' }],
  files: [{ path: 'a.rs', status: 'M', diff: '+new', truncated: false, preexisting: true }],
  truncated: false,
}
test('shows files and commits and expands a file diff', async () => {
  const view = mount(ChangesCard, { props: { block } })
  expect(view.text()).toContain('Changes · 1 file · 1 commit')
  expect(view.text()).toContain('1234567')
  expect(view.text()).toContain('Had uncommitted changes before this response.')
  expect(view.find('[data-diff]').exists()).toBe(false)
  await view.get('button[aria-expanded]').trigger('click')
  expect(view.find('[data-diff]').exists()).toBe(true)
})
for (const state of ['stopped', 'failed'] as const)
  test(`shows ${state} and truncation`, () => {
    const view = mount(ChangesCard, { props: { block: { ...block, state, truncated: true } } })
    expect(view.text()).toContain(state === 'stopped' ? 'Stopped' : 'Failed')
    expect(view.text()).toContain('Some changes are not shown.')
    expect(view.get('[data-ui-id="changes-card"]').attributes('data-protected')).toBe('true')
  })

test('uses singular labels for one and plural labels for two', () => {
  for (const count of [1, 2]) {
    const files = Array.from({ length: count }, (_, i) => ({
      ...block.files[0]!,
      path: `file${i}`,
    }))
    const commits = Array.from({ length: count }, (_, i) => ({
      ...block.commits[0]!,
      id: `commit${i}`,
    }))
    const view = mount(ChangesCard, { props: { block: { ...block, files, commits } } })
    expect(view.text()).toContain(
      count === 1 ? 'Changes · 1 file · 1 commit' : 'Changes · 2 files · 2 commits',
    )
  }
})
