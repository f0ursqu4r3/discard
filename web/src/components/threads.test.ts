import { expect, test } from 'vite-plus/test'
import type { ChannelDto, MessageDto, ThreadChipDto, ThreadLinkDto } from '../bridge/types'
import {
  THREAD_NAME_MAX,
  agentFollows,
  channelApprovals,
  messageCountLabel,
  normalizeThreadName,
  selectedChannelApprovals,
  threadAction,
  threadLinks,
  threadStates,
} from './threads'
import { threadActionLabels, threadDeleteBlock } from './thread-menu'

const link = (id: string, name = id): ThreadLinkDto => ({
  id,
  parentChannelId: 'channel',
  name,
  unread: false,
  running: false,
  pendingApprovals: 0,
  kind: 'question',
  state: null,
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
  const chip: ThreadChipDto = {
    channelId: 't1',
    name: 'Why?',
    messageCount: 2,
    lastMessage: null,
    followed: true,
    unread: false,
    kind: 'question',
    state: null,
    agentName: 'Quinn',
    pendingApprovals: 0,
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

test('the thread actions are implemented: no planned entry is left in the thread menu model', () => {
  expect(threadActionLabels).toEqual({
    list: 'Threads',
    rename: 'Rename thread',
    remove: 'Delete thread',
    removeFromChannel: 'Remove from channel',
  })
})

test('a running turn, a running task or a pending approval blocks Delete', () => {
  expect(threadDeleteBlock({})).toBeNull()
  expect(threadDeleteBlock({ running: false, state: 'done', pendingApprovals: 0 })).toBeNull()
  expect(threadDeleteBlock({ running: true })).toBe('A turn runs in this thread.')
  expect(threadDeleteBlock({ state: 'running' })).toBe('A sub-agent task runs in this thread.')
  expect(threadDeleteBlock({ pendingApprovals: 3 })).toBe('An approval waits in this thread.')
})

test('a thread name loses line breaks and edge spaces and has 1 to 80 characters', () => {
  expect(normalizeThreadName('  Latency\nplan\r\nnow  ')).toBe('Latency plan now')
  expect(normalizeThreadName('')).toBeNull()
  expect(normalizeThreadName(' \n ')).toBeNull()
  expect(normalizeThreadName('é'.repeat(THREAD_NAME_MAX))).toBe('é'.repeat(THREAD_NAME_MAX))
  expect(normalizeThreadName('a'.repeat(THREAD_NAME_MAX + 1))).toBeNull()
})

test('an agent message that follows a reference row says the agent read it', () => {
  const at = (id: string, role: MessageDto['role'], postedFrom?: MessageDto['postedFrom']) =>
    ({ id, role, continuesGroup: false, blocks: [], postedFrom }) as MessageDto
  const from = { sourceMessageId: 's', threadChannelId: 't', threadName: 'T' }
  const reference = at('ref', 'agent', from)
  expect(agentFollows([at('q', 'user'), reference], 'ref')).toBe(false)
  expect(agentFollows([reference, at('u', 'user')], 'ref')).toBe(false)
  expect(agentFollows([reference, at('a', 'agent')], 'ref')).toBe(true)
  expect(agentFollows([at('a', 'agent'), reference], 'ref')).toBe(false)
  expect(agentFollows([reference, at('other', 'agent', from)], 'ref')).toBe(false)
  expect(agentFollows([at('a', 'agent')], 'missing')).toBe(false)
})

test('approval counts add the threads of a channel to the channel itself', () => {
  expect(channelApprovals({ pendingApprovals: 1, threadPendingApprovals: 2 })).toBe(3)
  expect(channelApprovals({ pendingApprovals: 1 })).toBe(1)
  const channels = [
    { id: 'a', pendingApprovals: 0, threadPendingApprovals: 2 },
    { id: 'b', pendingApprovals: 4, threadPendingApprovals: 0 },
  ] as ChannelDto[]
  expect(selectedChannelApprovals({ channelId: 'a', channels, pendingApprovals: 0 })).toBe(2)
  expect(selectedChannelApprovals({ channelId: 'b', channels, pendingApprovals: 4 })).toBe(4)
})

test('every run state has a label', () => {
  expect(Object.values(threadStates).map((state) => state.label)).toEqual([
    'Running',
    'Done',
    'Failed',
    'Cancelled',
    'Timed out',
  ])
})
