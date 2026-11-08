import { expect, test } from 'vite-plus/test'
import { treePrefix } from './branch-navigation'
test('tree prefixes distinguish middle and last worktree children', () => {
  expect(treePrefix(false)).toBe('├')
  expect(treePrefix(true)).toBe('└')
})
