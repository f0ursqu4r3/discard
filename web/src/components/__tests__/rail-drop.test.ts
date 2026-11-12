import { expect, test } from 'vite-plus/test'
import {
  dropChange,
  dropLine,
  dropTarget,
  type DragSource,
  type DropTarget,
  type RailEntry,
} from '../rail-drop'

const tile = (top: number) => ({ top, bottom: top + 48 })
// Top-level list: a, b, collapsed group g1, expanded group g2 holding m1 and m2, e.
const entries: RailEntry[] = [
  { kind: 'project', id: 'a', rect: tile(0) },
  { kind: 'project', id: 'b', rect: tile(56) },
  { kind: 'group', id: 'g1', collapsed: true, rect: tile(112), members: [] },
  {
    kind: 'group',
    id: 'g2',
    collapsed: false,
    rect: { top: 168, bottom: 308 },
    members: [
      { id: 'm1', rect: tile(200) },
      { id: 'm2', rect: tile(256) },
    ],
  },
  { kind: 'project', id: 'e', rect: tile(316) },
]
const project = (id: string): DragSource => ({ kind: 'project', id })
const group = (id: string): DragSource => ({ kind: 'group', id })
const reorder = (groupId: string | null, index: number): DropTarget => ({
  type: 'reorder',
  groupId,
  index,
})

test('the middle of an ungrouped project means "make a group"', () => {
  expect(dropTarget(project('e'), 80, entries)).toEqual({ type: 'into-project', projectId: 'b' })
})

test('the top and bottom quarter of a tile mean "between items"', () => {
  expect(dropTarget(project('e'), 60, entries)).toEqual(reorder(null, 1))
  expect(dropTarget(project('e'), 100, entries)).toEqual(reorder(null, 2))
  expect(dropTarget(project('e'), 2, entries)).toEqual(reorder(null, 0))
})

test('the gaps between tiles and the space around the list reorder at that boundary', () => {
  expect(dropTarget(project('e'), 52, entries)).toEqual(reorder(null, 1))
  expect(dropTarget(project('e'), -20, entries)).toEqual(reorder(null, 0))
  expect(dropTarget(project('a'), 500, entries)).toEqual(reorder(null, 5))
})

test('the middle of a collapsed group means "add to the group"', () => {
  expect(dropTarget(project('a'), 136, entries)).toEqual({ type: 'into-group', groupId: 'g1' })
  expect(dropTarget(project('a'), 114, entries)).toEqual(reorder(null, 2))
})

test('a drop inside an expanded group picks the member position', () => {
  expect(dropTarget(project('a'), 180, entries)).toEqual(reorder('g2', 0))
  expect(dropTarget(project('a'), 210, entries)).toEqual(reorder('g2', 0))
  expect(dropTarget(project('a'), 240, entries)).toEqual(reorder('g2', 1))
  expect(dropTarget(project('a'), 290, entries)).toEqual(reorder('g2', 2))
})

test('the outer band of an expanded group means "outside the group"', () => {
  expect(dropTarget(project('m1'), 170, entries)).toEqual(reorder(null, 3))
  expect(dropTarget(project('m1'), 306, entries)).toEqual(reorder(null, 4))
})

test('a project over its own ungrouped tile is not a target', () => {
  expect(dropTarget(project('b'), 80, entries)).toBeNull()
})

test('a group only reorders, by the middle of the item under it', () => {
  expect(dropTarget(group('g1'), 80, entries)).toEqual(reorder(null, 2))
  expect(dropTarget(group('g1'), 60, entries)).toEqual(reorder(null, 1))
  expect(dropTarget(group('g1'), 200, entries)).toEqual(reorder(null, 3))
  expect(dropTarget(group('g1'), 300, entries)).toEqual(reorder(null, 4))
})

test('reordering an ungrouped project counts the list without the project', () => {
  expect(dropChange(project('a'), reorder(null, 3), entries)).toEqual({
    type: 'move-project',
    projectId: 'a',
    groupId: null,
    index: 2,
  })
  expect(dropChange(project('e'), reorder(null, 1), entries)).toEqual({
    type: 'move-project',
    projectId: 'e',
    groupId: null,
    index: 1,
  })
  expect(dropChange(project('b'), reorder(null, 1), entries)).toBeNull()
  expect(dropChange(project('b'), reorder(null, 2), entries)).toBeNull()
})

test('a member dropped between top-level items leaves its group at the displayed index', () => {
  expect(dropChange(project('m1'), reorder(null, 1), entries)).toEqual({
    type: 'move-project',
    projectId: 'm1',
    groupId: null,
    index: 1,
  })
  // Index 4 is the boundary after the expanded group. The group stays in the list while the index applies.
  expect(dropChange(project('m2'), reorder(null, 4), entries)).toEqual({
    type: 'move-project',
    projectId: 'm2',
    groupId: null,
    index: 4,
  })
})

test('reordering inside a group counts the members without the moved one', () => {
  expect(dropChange(project('m1'), reorder('g2', 2), entries)).toEqual({
    type: 'move-project',
    projectId: 'm1',
    groupId: 'g2',
    index: 1,
  })
  expect(dropChange(project('m2'), reorder('g2', 0), entries)).toEqual({
    type: 'move-project',
    projectId: 'm2',
    groupId: 'g2',
    index: 0,
  })
  expect(dropChange(project('m1'), reorder('g2', 0), entries)).toBeNull()
  expect(dropChange(project('m1'), reorder('g2', 1), entries)).toBeNull()
  expect(dropChange(project('m2'), reorder('g2', 2), entries)).toBeNull()
})

test('an outside project dropped into an expanded group keeps the displayed index', () => {
  expect(dropChange(project('a'), reorder('g2', 1), entries)).toEqual({
    type: 'move-project',
    projectId: 'a',
    groupId: 'g2',
    index: 1,
  })
})

test('dropping onto a project makes a group, and never onto itself', () => {
  expect(dropChange(project('a'), { type: 'into-project', projectId: 'b' }, entries)).toEqual({
    type: 'group-projects',
    draggedId: 'a',
    targetId: 'b',
  })
  expect(dropChange(project('a'), { type: 'into-project', projectId: 'a' }, entries)).toBeNull()
})

test('dropping onto a collapsed group appends the project, unless it is a member', () => {
  expect(dropChange(project('a'), { type: 'into-group', groupId: 'g1' }, entries)).toEqual({
    type: 'move-project',
    projectId: 'a',
    groupId: 'g1',
    index: 0,
  })
  expect(dropChange(project('m1'), { type: 'into-group', groupId: 'g1' }, entries)).toMatchObject({
    groupId: 'g1',
  })
  expect(dropChange(project('a'), { type: 'into-group', groupId: 'missing' }, entries)).toBeNull()
})

test('a group moves in the top-level list and ignores project targets', () => {
  expect(dropChange(group('g1'), reorder(null, 0), entries)).toEqual({
    type: 'move-group',
    groupId: 'g1',
    index: 0,
  })
  expect(dropChange(group('g1'), reorder(null, 5), entries)).toEqual({
    type: 'move-group',
    groupId: 'g1',
    index: 4,
  })
  expect(dropChange(group('g1'), reorder(null, 2), entries)).toBeNull()
  expect(dropChange(group('g1'), reorder(null, 3), entries)).toBeNull()
  expect(dropChange(group('g1'), reorder('g2', 0), entries)).toBeNull()
  expect(dropChange(group('g1'), { type: 'into-project', projectId: 'a' }, entries)).toBeNull()
})

test('the drop line sits between the items it separates', () => {
  expect(dropLine(reorder(null, 0), entries)).toEqual({ y: -4, groupId: null })
  expect(dropLine(reorder(null, 1), entries)).toEqual({ y: 52, groupId: null })
  expect(dropLine(reorder(null, 5), entries)).toEqual({ y: 368, groupId: null })
  expect(dropLine(reorder('g2', 0), entries)).toEqual({ y: 198, groupId: 'g2' })
  expect(dropLine(reorder('g2', 1), entries)).toEqual({ y: 252, groupId: 'g2' })
  expect(dropLine(reorder('g2', 2), entries)).toEqual({ y: 306, groupId: 'g2' })
  expect(dropLine({ type: 'into-project', projectId: 'a' }, entries)).toBeNull()
  expect(dropLine(reorder(null, 0), [])).toBeNull()
})
