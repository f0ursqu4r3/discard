import { expect, test } from 'vite-plus/test'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto, ProjectChannelDto } from '@/bridge/types'
import { channelBranch } from '../channel-branch'

const snapshot: SnapshotDto = {
  ...(fixture as SnapshotDto),
  branches: [
    {
      key: 'main',
      label: 'main',
      kind: 'branch',
      depth: 0,
      current: true,
      collapsed: false,
      hasChildren: false,
    },
  ],
}
test('the metadata branch wins over the current row', () => {
  const channel = { id: snapshot.channelId, branch: 'feat/x' } as ProjectChannelDto
  expect(channelBranch(snapshot, [channel])).toBe('feat/x')
})
test('falls back to the current branch row', () => {
  expect(channelBranch(snapshot, undefined)).toBe('main')
})
test('with no data, the result is null', () => {
  expect(channelBranch({ ...snapshot, branches: [] }, undefined)).toBeNull()
})
