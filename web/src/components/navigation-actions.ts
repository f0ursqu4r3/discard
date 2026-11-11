import type { BranchRowDto, ChannelDto, ProjectChannelDto } from '../bridge/types'

/**
 * A channel is busy while a turn runs in it, while an approval waits in it or in one of its threads, while a
 * sub-agent task runs in one of its threads, and while an archive runs. This mirrors the app rule for the menus. The
 * app checks again when an action runs, so a stale view cannot start an action on a busy channel.
 */
export function channelBusy(channel: ChannelDto | undefined, archiveBusy = false): boolean {
  if (archiveBusy) return true
  if (!channel) return false
  return (
    channel.running ||
    channel.pendingApprovals > 0 ||
    (channel.threadPendingApprovals ?? 0) > 0 ||
    (channel.threads ?? []).some(
      (thread) => thread.running || thread.pendingApprovals > 0 || thread.state === 'running',
    )
  )
}

/** The branch of a row is the default branch when the main checkout of its repository holds it. */
export function isDefaultBranch(rows: BranchRowDto[], branch: BranchRowDto): boolean {
  return rows.some(
    (row) =>
      (row.repository ?? '') === (branch.repository ?? '') &&
      row.kind === 'main' &&
      !row.detached &&
      row.label === branch.label,
  )
}

export interface ChannelMenuState {
  muted: boolean
  archiveDisabled: boolean
}
/** Archive Channel… is blocked for `#general`, for an archived channel and for a busy channel. */
export function channelMenuState(
  channel: ProjectChannelDto | undefined,
  busy: boolean,
): ChannelMenuState {
  return {
    muted: !!channel?.muted,
    archiveDisabled: !channel || channel.kind === 'general' || channel.archived || busy,
  }
}

export interface BranchMenuState {
  createWorktreeDisabled: boolean
  renameBranchDisabled: boolean
  deleteBranchDisabled: boolean
}
/**
 * `checkedOut`: some working tree holds the branch. `channel`: the branch channel, when one exists.
 * Create Worktree… needs a free branch whose channel has no worktree. Rename Branch… needs an idle channel.
 * Delete Branch… needs a free, non-default branch and an idle channel.
 */
export function branchMenuState(input: {
  checkedOut: boolean
  isDefault: boolean
  channel: ProjectChannelDto | undefined
  busy: boolean
}): BranchMenuState {
  return {
    createWorktreeDisabled: input.checkedOut || !!input.channel?.worktree,
    renameBranchDisabled: input.busy,
    deleteBranchDisabled: input.checkedOut || input.isDefault || input.busy,
  }
}

export interface WorktreeMenuState {
  createBranchDisabled: boolean
  removeWorktreeDisabled: boolean
}
/** Create Branch… and Remove Worktree… need idle channels. The main checkout cannot be removed. */
export function worktreeMenuState(row: BranchRowDto, busy: boolean): WorktreeMenuState {
  return { createBranchDisabled: busy, removeWorktreeDisabled: row.kind === 'main' || busy }
}
