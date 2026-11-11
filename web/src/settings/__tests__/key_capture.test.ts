import { expect, it } from 'vite-plus/test'
import { captureChord } from './key_capture'
it('captures a chord with normalized modifiers and keys', () => {
  expect(
    captureChord(new KeyboardEvent('keydown', { key: 'k', metaKey: true, shiftKey: true })),
  ).toBe('cmd-shift-k')
  expect(captureChord(new KeyboardEvent('keydown', { key: 'ArrowDown', ctrlKey: true }))).toBe(
    'ctrl-down',
  )
})
