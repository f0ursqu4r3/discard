import { expect, test } from 'vite-plus/test'
import snapshot from '../fixtures/snapshot.json'
import thread from '../fixtures/thread.json'
import subAgent from '../fixtures/subagent.json'
import type { AgentRequest } from '../api'
import type { HistoryDto, SnapshotDto, ThreadHistoryDto } from '../types'

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

test('the sub-agent golden payloads follow the contract', () => {
  const {
    snapshot: payload,
    history,
    threadHistory,
    agentRequests,
  } = subAgent as unknown as {
    snapshot: SnapshotDto
    history: HistoryDto
    threadHistory: ThreadHistoryDto
    agentRequests: AgentRequest[]
  }
  // The live chips are on the snapshot, in task order, with no anchor.
  expect(payload.liveSubAgentThreads?.map((chip) => [chip.name, chip.kind, chip.state])).toEqual([
    ['Find callers', 'sub_agent', 'running'],
    ['Read the docs', 'sub_agent', 'done'],
  ])
  const open = payload.openThread
  expect(open?.kind).toBe('sub_agent')
  expect(open?.anchorMessageId).toBeNull()
  expect(open?.composerDisabled).toBe(false)
  expect(open?.composerDisabledReason).toBeNull()
  expect(threadHistory.anchor).toBeNull()
  // After the turn the same chips hang under the saved response.
  expect(history.messages[1]?.subAgentThreads?.map((chip) => chip.state)).toEqual([
    'done',
    'failed',
  ])
  expect(history.messages[0]?.subAgentThreads).toEqual([])
  expect(payload.threadApprovals?.[0]?.threadId).toBe(open?.channelId)
  expect(payload.channels[0]?.threadPendingApprovals).toBe(1)
  expect(payload.channels[0]?.threads?.[0]?.state).toBe('running')
  expect(agentRequests[0]?.state).toBe('pending')
})
