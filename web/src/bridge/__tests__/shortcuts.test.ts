import { expect, test } from 'vite-plus/test'
import { matchesChord } from '../shortcuts'
test('matches a command chord without accepting extra modifiers', () => {
  const key = new KeyboardEvent('keydown', { key: 'k', metaKey: true })
  expect(matchesChord('cmd-k', key, true)).toBe(true)
  expect(
    matchesChord(
      'cmd-k',
      new KeyboardEvent('keydown', { key: 'k', metaKey: true, shiftKey: true }),
      true,
    ),
  ).toBe(false)
  expect(
    matchesChord('cmd-k', new KeyboardEvent('keydown', { key: 'k', ctrlKey: true }), false),
  ).toBe(true)
})
