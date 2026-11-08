import type { ProjectChannelDto, SnapshotDto } from '@/bridge/types'

export function channelBranch(
  snapshot: SnapshotDto,
  projectChannels: ProjectChannelDto[] | undefined,
): string | null {
  return (
    projectChannels?.find((channel) => channel.id === snapshot.channelId)?.branch ??
    snapshot.branches.find((row) => row.current)?.label ??
    null
  )
}
