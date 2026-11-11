import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { invoke } from '@tauri-apps/api/core'
import {
  closeThread,
  followThread,
  getThreadHistory,
  leaveThread,
  openThread,
  postThreadMessage,
  selectThread,
  sendMessage,
  startThread,
} from './api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
beforeEach(() => vi.mocked(invoke).mockReset())

test('thread commands use exact IDs and send optional fields only when set', async () => {
  await startThread('channel-1', 'message-1', 'Why?')
  await startThread('channel-1', 'message-1', 'Why?', ['file-1'], true)
  await openThread('thread-1')
  await closeThread()
  await selectThread('thread-1')
  await postThreadMessage('message-2')
  await followThread('thread-1')
  await leaveThread('thread-1')
  await getThreadHistory()

  const calls = vi.mocked(invoke).mock.calls
  expect(calls[0]).toEqual([
    'start_thread',
    { parentChannelId: 'channel-1', anchorMessageId: 'message-1', text: 'Why?' },
  ])
  expect(calls[1]).toEqual([
    'start_thread',
    {
      parentChannelId: 'channel-1',
      anchorMessageId: 'message-1',
      text: 'Why?',
      attachmentIds: ['file-1'],
      alsoPost: true,
    },
  ])
  expect(calls.slice(2).map(([name, args]) => [name, args])).toEqual([
    ['open_thread', { threadId: 'thread-1' }],
    ['close_thread', undefined],
    ['select_thread', { threadId: 'thread-1' }],
    ['post_thread_message', { messageId: 'message-2' }],
    ['follow_thread', { threadId: 'thread-1' }],
    ['leave_thread', { threadId: 'thread-1' }],
    ['get_thread_history', undefined],
  ])
})

test('sendMessage adds alsoPost only when asked', async () => {
  await sendMessage('thread-1', 'hello')
  await sendMessage('thread-1', 'hello', [], true)
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['send_message', { channelId: 'thread-1', text: 'hello' }],
    ['send_message', { channelId: 'thread-1', text: 'hello', alsoPost: true }],
  ])
})
