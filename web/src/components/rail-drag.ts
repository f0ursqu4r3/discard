import { onBeforeUnmount, shallowRef, type Ref } from 'vue'
import {
  dropChange,
  dropLine,
  dropTarget,
  type DragSource,
  type DropTarget,
  type RailEntry,
  type Rect,
} from './rail-drop'
import type { RailChange, RailViewItem } from './rail-layout'

/** The pointer must move this far before a press becomes a drag. Smaller moves stay clicks. */
export const DRAG_THRESHOLD = 5
const SCROLL_EDGE = 24
const SCROLL_STEP = 8

export interface RailDrag {
  source: DragSource
  /** The pointer moved past the threshold. Before that, the press is still a click. */
  active: boolean
  x: number
  y: number
  target: DropTarget | null
  /** What a drop would do. Null when the drop changes nothing or is not on the rail. */
  change: RailChange | null
  line: { top: number; left: number; width: number } | null
}

const vertical = (element: Element): Rect => {
  const { top, bottom } = element.getBoundingClientRect()
  return { top, bottom }
}

/**
 * Pointer-driven drag and drop of rail tiles. It uses pointer events, not native drag and drop,
 * because the webview can intercept native drag events. Escape cancels a drag.
 */
export function useRailDrag(options: {
  scroller: Ref<HTMLElement | null>
  view: Ref<RailViewItem[]>
  apply: (change: RailChange) => void
}) {
  const drag = shallowRef<RailDrag | null>(null)
  let origin = { x: 0, y: 0 }
  let swallowClick = false
  let swallowTimer: ReturnType<typeof setTimeout> | undefined

  function measure(): RailEntry[] {
    const root = options.scroller.value
    if (!root) return []
    const elements = new Map<string, HTMLElement>()
    root
      .querySelectorAll<HTMLElement>('[data-rail-entry]')
      .forEach((element) => elements.set(element.dataset.railEntry ?? '', element))
    return options.view.value.flatMap((item): RailEntry[] => {
      const element = elements.get(
        item.kind === 'project' ? `project:${item.project.id}` : `group:${item.id}`,
      )
      if (!element) return []
      if (item.kind === 'project')
        return [{ kind: 'project', id: item.project.id, rect: vertical(element) }]
      const members = item.collapsed
        ? []
        : [...element.querySelectorAll<HTMLElement>('[data-rail-member]')].map((member) => ({
            id: member.dataset.railMember ?? '',
            rect: vertical(member),
          }))
      return [
        { kind: 'group', id: item.id, collapsed: item.collapsed, rect: vertical(element), members },
      ]
    })
  }

  function update(current: RailDrag, event: PointerEvent): RailDrag {
    const root = options.scroller.value
    const bounds = root?.getBoundingClientRect()
    if (root && bounds) {
      if (event.clientY < bounds.top + SCROLL_EDGE) root.scrollTop -= SCROLL_STEP
      else if (event.clientY > bounds.bottom - SCROLL_EDGE) root.scrollTop += SCROLL_STEP
    }
    // A pointer outside the rail drops nowhere.
    const inside = !!bounds && event.clientX >= bounds.left && event.clientX <= bounds.right
    const entries = inside ? measure() : []
    const target = inside ? dropTarget(current.source, event.clientY, entries) : null
    const change = target ? dropChange(current.source, target, entries) : null
    const line = change && target && bounds ? dropLine(target, entries) : null
    const inset = line?.groupId ? 10 : 6
    return {
      ...current,
      active: true,
      x: event.clientX,
      y: event.clientY,
      target,
      change,
      line:
        line && bounds
          ? { top: line.y, left: bounds.left + inset, width: bounds.width - inset * 2 }
          : null,
    }
  }

  function onMove(event: PointerEvent) {
    const current = drag.value
    if (!current) return
    if (
      !current.active &&
      Math.hypot(event.clientX - origin.x, event.clientY - origin.y) < DRAG_THRESHOLD
    )
      return
    drag.value = update(current, event)
  }
  function onUp() {
    const current = drag.value
    stop()
    if (current?.active) {
      holdClick()
      if (current.change) options.apply(current.change)
    }
  }
  function onCancel() {
    if (drag.value?.active) holdClick()
    stop()
  }
  function onKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !drag.value) return
    event.preventDefault()
    event.stopPropagation()
    onCancel()
  }
  /** The click that follows a drag must not select a project or toggle a group. */
  function holdClick() {
    swallowClick = true
    clearTimeout(swallowTimer)
    swallowTimer = setTimeout(() => (swallowClick = false), 100)
  }
  function stop() {
    drag.value = null
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onCancel)
    window.removeEventListener('blur', onCancel)
    window.removeEventListener('keydown', onKey, true)
  }

  /** Start tracking a press. The press becomes a drag only after the pointer moves past the threshold. */
  function onPointerDown(event: PointerEvent, source: DragSource) {
    if (event.button !== 0 || drag.value) return
    origin = { x: event.clientX, y: event.clientY }
    drag.value = {
      source,
      active: false,
      x: event.clientX,
      y: event.clientY,
      target: null,
      change: null,
      line: null,
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    window.addEventListener('blur', onCancel)
    window.addEventListener('keydown', onKey, true)
  }
  /** Use as a capture-phase click handler on the rail. */
  function onClickCapture(event: MouseEvent) {
    if (!swallowClick) return
    swallowClick = false
    event.preventDefault()
    event.stopPropagation()
  }
  onBeforeUnmount(() => {
    stop()
    clearTimeout(swallowTimer)
  })
  return { drag, onPointerDown, onClickCapture }
}
