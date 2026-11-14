import { expect, test } from 'vite-plus/test'
import { parseDiff } from '../../messages'
import { inlineRows, splitRows } from '../diffRows'

const rows = (text: string) => splitRows(parseDiff(text).lines)
const HEAD = 'diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n'

test('a context line fills both sides with its old and new number', () => {
  expect(rows(`${HEAD}@@ -5,2 +7,2 @@\n one\n two`)).toEqual([
    { type: 'hunk', text: '@@ -5,2 +7,2 @@' },
    {
      type: 'line',
      left: { number: 5, text: 'one', kind: 'context' },
      right: { number: 7, text: 'one', kind: 'context' },
    },
    {
      type: 'line',
      left: { number: 6, text: 'two', kind: 'context' },
      right: { number: 8, text: 'two', kind: 'context' },
    },
  ])
})

test('a removed run and an added run of the same size pair row by row', () => {
  expect(rows(`${HEAD}@@ -1,2 +1,2 @@\n-a\n-b\n+A\n+B`)).toEqual([
    { type: 'hunk', text: '@@ -1,2 +1,2 @@' },
    {
      type: 'line',
      left: { number: 1, text: 'a', kind: 'removed' },
      right: { number: 1, text: 'A', kind: 'added' },
    },
    {
      type: 'line',
      left: { number: 2, text: 'b', kind: 'removed' },
      right: { number: 2, text: 'B', kind: 'added' },
    },
  ])
})

test('the longer run leaves empty cells on the other side', () => {
  const more = rows(`${HEAD}@@ -1 +1,3 @@\n-a\n+A\n+B\n+C`).slice(1)
  expect(
    more.map((row) => row.type === 'line' && [row.left?.text ?? null, row.right?.text]),
  ).toEqual([
    ['a', 'A'],
    [null, 'B'],
    [null, 'C'],
  ])
  const less = rows(`${HEAD}@@ -1,3 +1 @@\n-a\n-b\n-c\n+A`).slice(1)
  expect(
    less.map((row) => row.type === 'line' && [row.left?.text, row.right?.text ?? null]),
  ).toEqual([
    ['a', 'A'],
    ['b', null],
    ['c', null],
  ])
})

test('pure additions and pure removals have one empty side', () => {
  const [, add] = rows(`${HEAD}@@ -0,0 +1 @@\n+x`)
  expect(add).toEqual({
    type: 'line',
    left: null,
    right: { number: 1, text: 'x', kind: 'added' },
  })
  const [, gone] = rows(`${HEAD}@@ -4 +3,0 @@\n-y`)
  expect(gone).toEqual({
    type: 'line',
    left: { number: 4, text: 'y', kind: 'removed' },
    right: null,
  })
})

test('a context line ends a run, so later changes pair on their own', () => {
  const list = rows(`${HEAD}@@ -1,4 +1,4 @@\n-a\n+A\n same\n-b\n+B`)
  expect(list.map((row) => row.type)).toEqual(['hunk', 'line', 'line', 'line'])
  expect(list[2]).toMatchObject({ left: { text: 'same' }, right: { text: 'same' } })
  expect(list[3]).toMatchObject({ left: { text: 'b', number: 3 }, right: { text: 'B', number: 3 } })
})

test('a removed line after an added line starts a new pair run', () => {
  const list = rows(`${HEAD}@@ -1,2 +1,2 @@\n-a\n+A\n-b\n+B`)
  expect(
    list.slice(1).map((row) => row.type === 'line' && [row.left?.text, row.right?.text]),
  ).toEqual([
    ['a', 'A'],
    ['b', 'B'],
  ])
})

test('each hunk header is one row for both sides, and headers before the first hunk are dropped', () => {
  const text = `diff --git a/f b/f\nnew file mode 100644\n--- /dev/null\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n@@ -20 +20 @@\n-c\n+d`
  const list = rows(text)
  expect(list.map((row) => row.type)).toEqual(['hunk', 'line', 'hunk', 'line'])
  expect(list[2]).toEqual({ type: 'hunk', text: '@@ -20 +20 @@' })
})

test('a diff without a hunk has no rows', () => {
  expect(rows('diff --git a/a b/b\nsimilarity index 100%\nrename from a\nrename to b')).toEqual([])
  expect(rows('')).toEqual([])
})

test('a no-newline marker is a note row, not a line of code', () => {
  const list = rows(`${HEAD}@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b`)
  expect(list.map((row) => row.type)).toEqual(['hunk', 'line', 'note', 'line'])
  expect(list[2]).toEqual({ type: 'note', text: '\\ No newline at end of file' })
})

test('inline rows carry both line numbers and no prefix character', () => {
  const list = inlineRows(parseDiff(`${HEAD}@@ -3,2 +3,2 @@\n keep\n-old\n+new`).lines)
  expect(list).toEqual([
    { type: 'hunk', text: '@@ -3,2 +3,2 @@' },
    { type: 'line', kind: 'context', oldNumber: 3, newNumber: 3, text: 'keep' },
    { type: 'line', kind: 'removed', oldNumber: 4, newNumber: null, text: 'old' },
    { type: 'line', kind: 'added', oldNumber: null, newNumber: 4, text: 'new' },
  ])
})

test('an added line whose text starts with dashes keeps its text', () => {
  const list = rows(`${HEAD}@@ -1 +1,2 @@\n a\n+-- not a header`)
  expect(list[2]).toMatchObject({ right: { text: '-- not a header', kind: 'added' } })
})
