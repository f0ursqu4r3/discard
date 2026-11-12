import type { RailChange } from './rail-layout'

/** Vertical extent of an element, in viewport pixels. */
export interface Rect {
  top: number
  bottom: number
}
/** The measured top-level rail, in display order. */
export type RailEntry =
  | { kind: 'project'; id: string; rect: Rect }
  | {
      kind: 'group'
      id: string
      collapsed: boolean
      /** The collapsed tile, or the whole expanded container. */
      rect: Rect
      /** The member tiles of an expanded group, in order. Empty when collapsed. */
      members: { id: string; rect: Rect }[]
    }
export type DragSource = { kind: 'project'; id: string } | { kind: 'group'; id: string }
export type DropTarget =
  /** Drop between items. `index` counts the displayed list, including the dragged item. */
  | { type: 'reorder'; groupId: string | null; index: number }
  /** Drop onto an ungrouped project to make a group. */
  | { type: 'into-project'; projectId: string }
  /** Drop onto a collapsed group to add the project. */
  | { type: 'into-group'; groupId: string }

/** Share of a single tile at its top and bottom that means "between", not "onto". */
const EDGE_FRACTION = 0.25
/** Height of the top and bottom band of an expanded container that means "outside the group". */
const CONTAINER_EDGE = 6
/** Distance of the drop line from the neighboring item. */
const LINE_OFFSET = 4

const middle = (rect: Rect) => (rect.top + rect.bottom) / 2
const reorder = (groupId: string | null, index: number): DropTarget => ({
  type: 'reorder',
  groupId,
  index,
})

/** Where a drop at vertical position `y` would land. Null means "not a drop target". */
export function dropTarget(source: DragSource, y: number, entries: RailEntry[]): DropTarget | null {
  for (const [index, entry] of entries.entries()) {
    if (y < entry.rect.top) return reorder(null, index)
    if (y > entry.rect.bottom) continue
    if (source.kind === 'group') return reorder(null, y < middle(entry.rect) ? index : index + 1)
    if (entry.kind === 'group' && !entry.collapsed) {
      if (y < entry.rect.top + CONTAINER_EDGE) return reorder(null, index)
      if (y > entry.rect.bottom - CONTAINER_EDGE) return reorder(null, index + 1)
      let at = 0
      for (const [position, member] of entry.members.entries())
        if (y >= middle(member.rect)) at = position + 1
      return reorder(entry.id, at)
    }
    if (entry.kind === 'project' && entry.id === source.id) return null
    const edge = (entry.rect.bottom - entry.rect.top) * EDGE_FRACTION
    if (y < entry.rect.top + edge) return reorder(null, index)
    if (y > entry.rect.bottom - edge) return reorder(null, index + 1)
    return entry.kind === 'project'
      ? { type: 'into-project', projectId: entry.id }
      : { type: 'into-group', groupId: entry.id }
  }
  return reorder(null, entries.length)
}

/** Convert a displayed index to the index of the list without the moved item. Null when nothing moves. */
function withoutSelf(position: number, index: number): number | null {
  if (position < 0) return index
  if (index === position || index === position + 1) return null
  return position < index ? index - 1 : index
}

/** The change a drop makes, or null when the drop leaves the rail as it is. */
export function dropChange(
  source: DragSource,
  target: DropTarget,
  entries: RailEntry[],
): RailChange | null {
  if (source.kind === 'group') {
    if (target.type !== 'reorder' || target.groupId !== null) return null
    const index = withoutSelf(
      entries.findIndex((entry) => entry.kind === 'group' && entry.id === source.id),
      target.index,
    )
    return index === null ? null : { type: 'move-group', groupId: source.id, index }
  }
  if (target.type === 'into-project')
    return target.projectId === source.id
      ? null
      : { type: 'group-projects', draggedId: source.id, targetId: target.projectId }
  if (target.type === 'into-group') {
    const group = entries.find((entry) => entry.kind === 'group' && entry.id === target.groupId)
    if (group?.kind !== 'group' || group.members.some((member) => member.id === source.id))
      return null
    return {
      type: 'move-project',
      projectId: source.id,
      groupId: target.groupId,
      index: group.members.length,
    }
  }
  if (target.groupId === null) {
    const index = withoutSelf(
      entries.findIndex((entry) => entry.kind === 'project' && entry.id === source.id),
      target.index,
    )
    return index === null
      ? null
      : { type: 'move-project', projectId: source.id, groupId: null, index }
  }
  const group = entries.find((entry) => entry.kind === 'group' && entry.id === target.groupId)
  if (group?.kind !== 'group') return null
  const index = withoutSelf(
    group.members.findIndex((member) => member.id === source.id),
    target.index,
  )
  return index === null
    ? null
    : { type: 'move-project', projectId: source.id, groupId: target.groupId, index }
}

/** The vertical position of the drop line, and whether it sits inside an expanded group. */
export function dropLine(
  target: DropTarget,
  entries: RailEntry[],
): { y: number; groupId: string | null } | null {
  if (target.type !== 'reorder') return null
  const items =
    target.groupId === null
      ? entries.map((entry) => entry.rect)
      : (() => {
          const group = entries.find(
            (entry) => entry.kind === 'group' && entry.id === target.groupId,
          )
          return group?.kind === 'group' ? group.members.map((member) => member.rect) : []
        })()
  if (!items.length) return null
  const offset = target.groupId === null ? LINE_OFFSET : LINE_OFFSET / 2
  const y =
    target.index <= 0
      ? items[0]!.top - offset
      : target.index >= items.length
        ? items[items.length - 1]!.bottom + offset
        : (items[target.index - 1]!.bottom + items[target.index]!.top) / 2
  return { y, groupId: target.groupId }
}
