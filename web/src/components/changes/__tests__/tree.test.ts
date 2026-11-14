import { expect, test } from 'vite-plus/test'
import type { ChangeFileDto } from '../../../bridge/types'
import { buildTree, filterFiles, flattenTree, statusTone, treeOrder, type TreeNode } from '../tree'

const file = (path: string, patch: Partial<ChangeFileDto> = {}): ChangeFileDto => ({
  path,
  oldPath: null,
  status: 'M',
  additions: 1,
  deletions: 2,
  binary: false,
  diff: '',
  truncated: false,
  ...patch,
})
const names = (nodes: TreeNode[]): string[] =>
  nodes.map((node) => (node.type === 'folder' ? `${node.name}/` : node.name))

test('a chain of single-child folders becomes one row', () => {
  const tree = buildTree([
    file('crates/discard-core/src/a.rs'),
    file('crates/discard-core/src/b.rs'),
  ])
  expect(names(tree)).toEqual(['crates/discard-core/src/'])
  const folder = tree[0]!
  expect(folder.type === 'folder' && folder.path).toBe('crates/discard-core/src')
  expect(folder.type === 'folder' && names(folder.children)).toEqual(['a.rs', 'b.rs'])
})

test('a folder with two children keeps its own row, and a folder with a file does not join its child', () => {
  const tree = buildTree([
    file('web/src/a.ts'),
    file('web/test/b.ts'),
    file('docs/x/y/z.md'),
    file('docs/readme.md'),
  ])
  expect(names(tree)).toEqual(['docs/', 'web/'])
  const [docs, web] = tree
  expect(docs?.type === 'folder' && names(docs.children)).toEqual(['x/y/', 'readme.md'])
  expect(web?.type === 'folder' && names(web.children)).toEqual(['src/', 'test/'])
})

test('folders come before files and each group is sorted by name', () => {
  const tree = buildTree([
    file('z.txt'),
    file('b/one.txt'),
    file('b/two.txt'),
    file('a.txt'),
    file('a/one.txt'),
    file('a/two.txt'),
  ])
  expect(names(tree)).toEqual(['a/', 'b/', 'a.txt', 'z.txt'])
})

test('a folder counts its files and totals', () => {
  const [folder] = buildTree([
    file('d/a', { additions: 3, deletions: 1 }),
    file('d/e/f', { additions: 4, deletions: 5 }),
    file('d/e/g'),
  ])
  expect(folder).toMatchObject({ fileCount: 3, additions: 8, deletions: 8 })
})

test('flattening hides the children of a collapsed folder', () => {
  const tree = buildTree([file('a/x/1'), file('a/x/2'), file('a/y/3'), file('top')])
  const open = flattenTree(tree, new Set())
  expect(open.map((row) => [row.depth, row.node.name])).toEqual([
    [0, 'a'],
    [1, 'x'],
    [2, '1'],
    [2, '2'],
    [1, 'y'],
    [2, '3'],
    [0, 'top'],
  ])
  expect(flattenTree(tree, new Set(['a/x'])).map((row) => row.node.name)).toEqual([
    'a',
    'x',
    'y',
    '3',
    'top',
  ])
  expect(flattenTree(tree, new Set(['a'])).map((row) => row.node.name)).toEqual(['a', 'top'])
})

test('the filter keeps paths that contain the text, ignoring case and outer spaces', () => {
  const all = [file('src/App.vue'), file('src/main.ts'), file('docs/app.md')]
  expect(filterFiles(all, 'app').map((f) => f.path)).toEqual(['src/App.vue', 'docs/app.md'])
  expect(filterFiles(all, ' SRC/m ').map((f) => f.path)).toEqual(['src/main.ts'])
  expect(filterFiles(all, '   ')).toBe(all)
  expect(filterFiles(all, 'nothing')).toEqual([])
})

test('a filtered tree drops the folders that have no match', () => {
  const tree = buildTree(filterFiles([file('a/one.ts'), file('b/two.ts')], 'two'))
  expect(names(tree)).toEqual(['b/'])
})

test('tree order lists files the way the tree shows them', () => {
  expect(treeOrder([file('z'), file('a/b'), file('a/a'), file('b/c')]).map((f) => f.path)).toEqual([
    'a/a',
    'a/b',
    'b/c',
    'z',
  ])
})

test('status letters use the success, destructive, and quiet tokens', () => {
  expect(statusTone('A')).toBe('text-success')
  expect(statusTone('?')).toBe('text-success')
  expect(statusTone('D')).toBe('text-destructive')
  expect(statusTone('M')).toBe('text-warning')
  expect(statusTone('R')).toBe('text-muted-foreground')
})
