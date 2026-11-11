import type { ChannelDto, MessageDto, ThreadLinkDto } from '../bridge/types'

/** Followed threads under one channel. Live state from the snapshot wins over the stored list. */
export function threadLinks(
  channelId: string | undefined,
  live: ChannelDto[] = [],
  stored: { id: string; threads?: ThreadLinkDto[] }[] = [],
): ThreadLinkDto[] {
  if (!channelId) return []
  return (
    live.find((channel) => channel.id === channelId)?.threads ??
    stored.find((channel) => channel.id === channelId)?.threads ??
    []
  )
}

export type ThreadAction =
  | { kind: 'start'; label: 'Start thread'; threadId: null }
  | { kind: 'open'; label: 'Open thread'; threadId: string }

/**
 * The thread action of a message. An anchor opens its thread. A reference row opens its source thread
 * and never starts one.
 */
export function threadAction(message: MessageDto): ThreadAction {
  const threadId = message.postedFrom?.threadChannelId ?? message.thread?.channelId
  return threadId
    ? { kind: 'open', label: 'Open thread', threadId }
    : { kind: 'start', label: 'Start thread', threadId: null }
}

export function messageCountLabel(count: number): string {
  return `${count} ${count === 1 ? 'message' : 'messages'}`
}
