import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { terminalKey } from '../terminal-keys'

const mocks = vi.hoisted(() => ({ webShortcut: false }))
vi.mock('@/bridge/shortcuts', () => ({ isWebShortcut: () => mocks.webShortcut }))
beforeEach(() => (mocks.webShortcut = false))

const key = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init)
const mac = { hasSelection: false, toggleShortcut: 'Ctrl+`', mac: true }
const other = { ...mac, mac: false }

test('typing and shell control keys go to the shell', () => {
  for (const options of [mac, other]) {
    expect(terminalKey(key({ key: 'a' }), options)).toBe('default')
    expect(terminalKey(key({ key: 'c', ctrlKey: true }), options)).toBe('default')
    expect(terminalKey(key({ key: 'r', ctrlKey: true }), options)).toBe('default')
    expect(terminalKey(key({ key: 'Enter' }), options)).toBe('default')
  }
})

test('the toggle shortcut is not sent to the shell', () => {
  expect(terminalKey(key({ key: '`', ctrlKey: true }), mac)).toBe('app')
  expect(terminalKey(key({ key: '`', ctrlKey: true }), other)).toBe('app')
  expect(terminalKey(key({ key: '`' }), mac)).toBe('default')
})

test('a changed toggle shortcut follows the keymap', () => {
  const options = { ...mac, toggleShortcut: 'Cmd+J' }
  expect(terminalKey(key({ key: 'j', metaKey: true }), options)).toBe('app')
  expect(terminalKey(key({ key: '`', ctrlKey: true }), options)).toBe('default')
})

test('a shortcut the webview handles goes to the app', () => {
  mocks.webShortcut = true
  expect(terminalKey(key({ key: 'k', ctrlKey: true }), other)).toBe('app')
})

test('Cmd+C copies a selection and sends no Ctrl+C', () => {
  expect(terminalKey(key({ key: 'c', metaKey: true }), { ...mac, hasSelection: true })).toBe('copy')
  // Without a selection the key stays with the app. It never reaches the shell.
  expect(terminalKey(key({ key: 'c', metaKey: true }), mac)).toBe('app')
  expect(terminalKey(key({ key: 'c', ctrlKey: true }), { ...mac, hasSelection: true })).toBe(
    'default',
  )
})

test('Cmd+V is left to the paste event, and other command keys go to the app', () => {
  expect(terminalKey(key({ key: 'v', metaKey: true }), mac)).toBe('app')
  expect(terminalKey(key({ key: 'k', metaKey: true }), mac)).toBe('app')
})

test('off the Mac, Ctrl+Shift+C copies and Ctrl+Shift+V pastes', () => {
  const selected = { ...other, hasSelection: true }
  expect(terminalKey(key({ key: 'C', ctrlKey: true, shiftKey: true }), selected)).toBe('copy')
  expect(terminalKey(key({ key: 'C', ctrlKey: true, shiftKey: true }), other)).toBe('default')
  expect(terminalKey(key({ key: 'V', ctrlKey: true, shiftKey: true }), other)).toBe('paste')
})

test('a key release is never handled', () => {
  expect(terminalKey(new KeyboardEvent('keyup', { key: '`', ctrlKey: true }), mac)).toBe('default')
})
