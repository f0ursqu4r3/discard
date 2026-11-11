import { expect, test } from 'vite-plus/test'
import { normalizeTextChannelName } from './text-channel'

test('the preview follows the saved name rules', () => {
  expect(normalizeTextChannelName('  Release Notes  ')).toBe('release-notes')
  expect(normalizeTextChannelName('a   b\tc')).toBe('a-b-c')
  expect(normalizeTextChannelName('a -- b')).toBe('a-b')
  expect(normalizeTextChannelName('--Plan--')).toBe('plan')
  expect(normalizeTextChannelName('keep_this-1')).toBe('keep_this-1')
  expect(normalizeTextChannelName('   ')).toBe('')
})
