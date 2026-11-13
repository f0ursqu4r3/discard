import type { ProjectEntry, RailLayoutDto } from '../bridge/api'
import { resolveColor } from './agents/agent-icon'

/** A rail group with its member projects resolved. */
export interface RailGroupView {
  id: string
  name: string
  color: string | null
  collapsed: boolean
  projects: ProjectEntry[]
}
export type RailViewItem =
  | { kind: 'project'; project: ProjectEntry }
  | ({ kind: 'group' } & RailGroupView)

/** One change to the saved rail. `index` counts the target list without the moved item. */
export type RailChange =
  | { type: 'move-project'; projectId: string; groupId: string | null; index: number }
  | { type: 'group-projects'; draggedId: string; targetId: string }
  /** `created` runs with the new group ID after the group exists. */
  | { type: 'create-group'; projectId: string; created?: (groupId: string) => void }
  | { type: 'move-group'; groupId: string; index: number }
  | { type: 'rename-group'; groupId: string; name: string }
  | { type: 'color-group'; groupId: string; color: string | null }
  | { type: 'collapse-group'; groupId: string; collapsed: boolean }
  | { type: 'ungroup'; groupId: string }

/**
 * Resolve the saved layout against the visible projects. Unknown project IDs are skipped.
 * A project missing from the layout goes last. Without a layout, every project is ungrouped.
 */
export function resolveRailView(
  projects: ProjectEntry[],
  layout?: RailLayoutDto | null,
): RailViewItem[] {
  const byId = new Map(projects.map((project) => [project.id, project]))
  const placed = new Set<string>()
  const view: RailViewItem[] = []
  for (const item of layout?.items ?? []) {
    if (item.kind === 'project') {
      const project = byId.get(item.projectId)
      if (!project || placed.has(project.id)) continue
      placed.add(project.id)
      view.push({ kind: 'project', project })
      continue
    }
    const members = item.projectIds.flatMap((id) => {
      const project = byId.get(id)
      if (!project || placed.has(id)) return []
      placed.add(id)
      return [project]
    })
    if (members.length)
      view.push({
        kind: 'group',
        id: item.id,
        name: item.name,
        color: item.color,
        collapsed: item.collapsed,
        projects: members,
      })
  }
  for (const project of projects)
    if (!placed.has(project.id)) view.push({ kind: 'project', project })
  return view
}

/** The saved name, or a label built from the member names. */
export function groupLabel(group: Pick<RailGroupView, 'name' | 'projects'>): string {
  const name = group.name.trim()
  if (name) return name
  const names = group.projects.map((project) => project.name)
  if (names.length <= 3) return names.join(', ')
  return `${names.slice(0, 3).join(', ')} +${names.length - 3}`
}

/** The group that holds the project, if any. */
export function groupOf(view: RailViewItem[], projectId: string): RailGroupView | null {
  for (const item of view)
    if (item.kind === 'group' && item.projects.some((project) => project.id === projectId))
      return item
  return null
}

export interface RailGroupOption {
  id: string
  label: string
}

export type RailSize = 'mini' | 'compact' | 'default'
/**
 * Classes per rail size, and the pill height of a selected row in spacing units.
 * An expanded group pads its tiles by one spacing unit on all four sides: `group` is the tile width
 * plus that padding, `groupTile` is a collapsed group as wide as an expanded one, `groupRow` is a row as high as the tile, and `groupPill` moves the selection
 * pill from the row to the rail edge.
 */
export const railSizes: Record<
  RailSize,
  {
    tile: string
    row: string
    pillUnits: number
    group: string
    groupTile: string
    groupRow: string
    groupPill: string
    icon: string
  }
> = {
  mini: {
    tile: 'size-8',
    row: 'h-8',
    pillUnits: 8,
    group: 'w-10',
    groupTile: 'size-10',
    groupRow: 'h-8',
    groupPill: '-left-2',
    icon: 'size-4',
  },
  compact: {
    tile: 'size-9',
    row: 'h-10',
    pillUnits: 10,
    group: 'w-11',
    groupTile: 'size-11',
    groupRow: 'h-9',
    groupPill: '-left-2',
    icon: 'size-4',
  },
  default: {
    tile: 'size-12',
    row: 'h-12',
    pillUnits: 12,
    group: 'w-14',
    groupTile: 'size-14',
    groupRow: 'h-12',
    groupPill: '-left-3',
    icon: 'size-5',
  },
}

/** A translucent tint of a group color for backgrounds. Undefined keeps the neutral theme background. */
export function groupTint(
  color: string | null,
  tokens: Record<string, string>,
  percent: number,
): string | undefined {
  if (!color) return undefined
  return `color-mix(in srgb, ${resolveColor(color, tokens)} ${percent}%, transparent)`
}
