import { Ban, CircleCheck, CircleX, Timer } from '@lucide/vue'
import type {
  ChannelDto,
  MessageDto,
  SnapshotDto,
  ThreadKind,
  ThreadLinkDto,
  ThreadState,
} from '../bridge/types'

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

/** How each run state of a sub-agent thread shows. `running` uses the app spinner, so it has no icon. */
export const threadStates: Record<
  ThreadState,
  { label: string; icon: typeof CircleCheck | null; class: string }
> = {
  running: { label: 'Running', icon: null, class: 'text-muted-foreground' },
  done: { label: 'Done', icon: CircleCheck, class: 'text-success' },
  failed: { label: 'Failed', icon: CircleX, class: 'text-destructive' },
  cancelled: { label: 'Cancelled', icon: Ban, class: 'text-muted-foreground' },
  timed_out: { label: 'Timed out', icon: Timer, class: 'text-warning' },
}

export function threadKindLabel(kind: ThreadKind): string {
  return kind === 'sub_agent' ? 'Sub-agent thread' : 'Thread'
}

export function pendingApprovalsLabel(count: number): string {
  return `${count} pending ${count === 1 ? 'approval' : 'approvals'}`
}

/** Approvals that wait in a channel and in its threads, followed or not. */
export function channelApprovals(
  channel: Pick<ChannelDto, 'pendingApprovals' | 'threadPendingApprovals'>,
): number {
  return (channel.pendingApprovals ?? 0) + (channel.threadPendingApprovals ?? 0)
}

/** Approvals that wait in the selected channel and in its threads. */
export function selectedChannelApprovals(
  snapshot: Pick<SnapshotDto, 'channelId' | 'channels' | 'pendingApprovals'>,
): number {
  const channel = snapshot.channels.find((entry) => entry.id === snapshot.channelId)
  return (snapshot.pendingApprovals ?? 0) + (channel?.threadPendingApprovals ?? 0)
}
