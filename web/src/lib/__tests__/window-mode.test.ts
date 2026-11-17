import { expect, test } from 'vite-plus/test'
import { windowMode } from '../window-mode'

test('an empty hash or another hash is the main window', () => {
  expect(windowMode('')).toEqual({ kind: 'main' })
  expect(windowMode('#/settings/general')).toEqual({ kind: 'main' })
})

test('the project hash names the project of the window', () => {
  expect(windowMode('#/project/abc-123')).toEqual({ kind: 'project', projectId: 'abc-123' })
})

test('a project hash with no ID is the main window', () => {
  expect(windowMode('#/project/')).toEqual({ kind: 'main' })
})

test('the project ID is decoded from its percent-encoded form', () => {
  expect(windowMode('#/project/a%20b%23c')).toEqual({ kind: 'project', projectId: 'a b#c' })
})

test('a malformed escape is the main window', () => {
  expect(windowMode('#/project/%E0%A4%A')).toEqual({ kind: 'main' })
})
