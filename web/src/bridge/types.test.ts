import { expect, test } from 'vitest'
import snapshot from './fixtures/snapshot.json'
import type { SnapshotDto } from './types'

test('the golden snapshot follows the contract', () => {
  const value = snapshot as SnapshotDto satisfies SnapshotDto
  expect(value.liveBlocks[0]?.type).toBe('markdown')
})
