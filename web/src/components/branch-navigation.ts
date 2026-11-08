import type { BranchRowDto } from '../bridge/types'
import { planned } from './project-menu'

export const channelRowClass =
  'flex h-8 min-h-8 w-full min-w-0 shrink-0 items-center gap-2 rounded-md px-2 text-left text-sm outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring'

export const branchActions = {
  // TODO: Create a Git branch and its channel with repository selection and validation.
  create: planned('New branch channel'),
  // TODO: Reveal any discovered worktree folder through a validated desktop command.
  reveal: planned('Open worktree folder'),
  // TODO: Implement a worktree manager with create, inspect, and safe removal actions.
  manage: planned('Manage worktrees…'),
}

export function worktreeName(row: BranchRowDto): string {
  return (
    row.worktree
      ?.replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || 'Worktree'
  )
}

/** Group a repository's checkouts under their branch; detached HEADs have no parent branch. */
export function groupBranchRows(rows: BranchRowDto[]) {
  const branches = new Map<string, { row: BranchRowDto; worktrees: BranchRowDto[] }>()
  const detached: BranchRowDto[] = []
  let worktreeCount = 0
  for (const row of rows) {
    const isWorktree = row.kind === 'main' || row.kind === 'worktree'
    if (isWorktree) worktreeCount++
    if (row.detached) {
      if (isWorktree) detached.push(row)
      continue
    }
    let branch = branches.get(row.label)
    if (!branch) {
      branch = { row, worktrees: [] }
      branches.set(row.label, branch)
    } else if (row.current) branch.row = row
    if (isWorktree) branch.worktrees.push(row)
  }
  return { branches: [...branches.values()], detached, worktreeCount }
}

export function treePrefix(isLast: boolean): '├' | '└' {
  return isLast ? '└' : '├'
}
