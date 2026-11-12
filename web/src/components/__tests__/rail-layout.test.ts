import { expect, test } from 'vite-plus/test'
import type { ProjectEntry, RailLayoutDto } from '../../bridge/api'
import { groupLabel, groupOf, groupTint, resolveRailView } from '../rail-layout'

const project = (id: string, name = id.toUpperCase()): ProjectEntry => ({
  id,
  name,
  path: `/${id}`,
})
const [a, b, c, d] = ['a', 'b', 'c', 'd'].map((id) => project(id))
const group = (id: string, projectIds: string[], extra = {}) => ({
  kind: 'group' as const,
  id,
  name: '',
  color: null,
  collapsed: false,
  projectIds,
  ...extra,
})

test('without a layout every project is ungrouped in the given order', () => {
  expect(resolveRailView([b!, a!], null)).toEqual([
    { kind: 'project', project: b },
    { kind: 'project', project: a },
  ])
})

test('the layout sets the order and the groups', () => {
  const layout: RailLayoutDto = {
    items: [{ kind: 'project', projectId: 'c' }, group('g', ['b', 'a'], { collapsed: true })],
  }
  const view = resolveRailView([a!, b!, c!], layout)
  expect(view).toEqual([
    { kind: 'project', project: c },
    {
      kind: 'group',
      id: 'g',
      name: '',
      color: null,
      collapsed: true,
      projects: [b, a],
    },
  ])
})

test('unknown projects are skipped, repeated ones appear once, and missing ones go last', () => {
  const layout: RailLayoutDto = {
    items: [
      { kind: 'project', projectId: 'ghost' },
      group('g', ['ghost', 'a']),
      group('empty', ['ghost']),
      { kind: 'project', projectId: 'a' },
    ],
  }
  const view = resolveRailView([a!, b!, c!], layout)
  expect(view.map((item) => (item.kind === 'project' ? item.project.id : item.id))).toEqual([
    'g',
    'b',
    'c',
  ])
  expect(view[0]).toMatchObject({ projects: [a] })
})

test('a group label is its name, or the member names with a count of the rest', () => {
  expect(groupLabel({ name: '  Work ', projects: [a!] })).toBe('Work')
  expect(groupLabel({ name: '', projects: [a!, b!] })).toBe('A, B')
  expect(groupLabel({ name: ' ', projects: [a!, b!, c!, d!] })).toBe('A, B, C +1')
})

test('the group of a project is found, or null', () => {
  const view = resolveRailView([a!, b!, c!], { items: [group('g', ['a', 'b'])] })
  expect(groupOf(view, 'b')?.id).toBe('g')
  expect(groupOf(view, 'c')).toBeNull()
})

test('a tint uses the resolved theme color, and no color keeps the neutral background', () => {
  expect(groupTint(null, {}, 30)).toBeUndefined()
  expect(groupTint('ansi-4', { ansi_4: '#112233' }, 30)).toBe(
    'color-mix(in srgb, #112233 30%, transparent)',
  )
  expect(groupTint('#abcdef', {}, 18)).toBe('color-mix(in srgb, #abcdef 18%, transparent)')
})
