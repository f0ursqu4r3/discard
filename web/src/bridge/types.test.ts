import { expect, test } from 'vite-plus/test'
import snapshot from './fixtures/snapshot.json'
import thread from './fixtures/thread.json'
import type { HistoryDto, SnapshotDto, ThreadHistoryDto } from './types'

test('the golden snapshot follows the contract', () => {
  const value = snapshot as SnapshotDto satisfies SnapshotDto
  expect(value.liveBlocks[0]?.type).toBe('markdown')
})

test('the thread golden payloads follow the contract', () => {
  const {
    snapshot: payload,
    history,
    threadHistory,
  } = thread as {
    snapshot: SnapshotDto
    history: HistoryDto
    threadHistory: ThreadHistoryDto
  }
  const open = payload.openThread
  expect(open?.channelId).toBe('thread')
  expect(open?.historyRev).toBe(threadHistory.rev)
  expect(payload.channels[0]?.threads?.[0]?.id).toBe(open?.channelId)
  expect(history.messages[0]?.thread?.channelId).toBe(open?.channelId)
  expect(history.messages[1]?.postedFrom?.threadChannelId).toBe(open?.channelId)
  expect(threadHistory.anchor?.id).toBe(open?.anchorMessageId)
  expect(threadHistory.messages.filter((message) => message.postedToChannel)).toHaveLength(1)
})
