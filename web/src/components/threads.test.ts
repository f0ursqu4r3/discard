import { expect, test } from 'vite-plus/test'
import type { ChannelDto, MessageDto, ThreadLinkDto } from '../bridge/types'
import { messageCountLabel, threadAction, threadLinks } from './threads'
import { referenceMenuPlanned, threadActions, threadMenuPlanned } from './thread-menu'

const link = (id: string, name = id): ThreadLinkDto => ({
  id,
  parentChannelId: 'channel',
  name,
  unread: false,
  running: false,
  pendingApprovals: 0,
})
const message: MessageDto = { id: 'm', role: 'agent', continuesGroup: false, blocks: [] }

test('live thread links win over stored links', () => {
  const live = [{ id: 'channel', threads: [link('live')] }] as ChannelDto[]
  const stored = [{ id: 'channel', threads: [link('stored')] }]
  expect(threadLinks('channel', live, stored).map((item) => item.id)).toEqual(['live'])
  expect(threadLinks('channel', [{ id: 'channel' }] as ChannelDto[], stored)).toEqual([
    link('stored'),
  ])
  expect(threadLinks('other', live, stored)).toEqual([])
  expect(threadLinks(undefined, live, stored)).toEqual([])
})

test('a message starts a thread, an anchor opens its thread, a reference row opens its source', () => {
  expect(threadAction(message)).toEqual({ kind: 'start', label: 'Start thread', threadId: null })
  const chip = {
    channelId: 't1',
    name: 'Why?',
    messageCount: 2,
    lastMessage: null,
    followed: true,
    unread: false,
  }
  expect(threadAction({ ...message, thread: chip })).toEqual({
    kind: 'open',
    label: 'Open thread',
    threadId: 't1',
  })
  const postedFrom = { sourceMessageId: 's', threadChannelId: 't2', threadName: 'Other' }
  expect(threadAction({ ...message, postedFrom })).toMatchObject({ kind: 'open', threadId: 't2' })
  expect(threadAction({ ...message, postedFrom, thread: chip })).toMatchObject({
    kind: 'open',
    threadId: 't2',
  })
})

test('message counts use the singular for one', () => {
  expect(messageCountLabel(1)).toBe('1 message')
  expect(messageCountLabel(3)).toBe('3 messages')
})

test('every planned thread control uses the shared planned model', () => {
  for (const entry of [
    ...Object.values(threadActions),
    ...threadMenuPlanned,
    ...referenceMenuPlanned,
  ])
    expect(entry.kind).toBe('planned')
  expect(threadMenuPlanned.map((entry) => entry.label)).toEqual(['Rename thread', 'Delete thread'])
  expect(referenceMenuPlanned.map((entry) => entry.label)).toEqual(['Remove from channel'])
})
