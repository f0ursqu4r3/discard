import { expect, test } from 'vite-plus/test'
import { activeFilePath } from '../activeFile'

const sections = [
  { path: 'a.ts', top: -300 },
  { path: 'b.ts', top: 10 },
  { path: 'c.ts', top: 400 },
]

test('an empty list has no active file', () => {
  expect(activeFilePath(0, [])).toBeNull()
})

test('the first section is active when none starts above the line', () => {
  expect(
    activeFilePath(0, [
      { path: 'a.ts', top: 200 },
      { path: 'b.ts', top: 500 },
    ]),
  ).toBe('a.ts')
})

test('the last section at or above the pane top plus the offset is active', () => {
  expect(activeFilePath(0, sections)).toBe('b.ts')
  expect(activeFilePath(0, sections, 5)).toBe('a.ts')
  expect(activeFilePath(0, sections, 10)).toBe('b.ts')
  expect(activeFilePath(390, sections)).toBe('c.ts')
})
