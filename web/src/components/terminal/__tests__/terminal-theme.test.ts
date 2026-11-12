import { expect, test } from 'vite-plus/test'
import type { ThemeDto } from '@/bridge/types'
import { terminalFont, terminalTheme } from '../terminal-theme'

const palette = Array.from({ length: 16 }, (_, index) => `#${index.toString(16).repeat(6)}`)
const theme = (overrides: Partial<ThemeDto> = {}): ThemeDto => ({
  mode: 'dark',
  tokens: { background: '#111111', foreground: '#eeeeee', ansi_1: '#aa0000' },
  radius: null,
  fontScale: 1,
  spacingScale: 1,
  protectedBg: '#000000',
  protectedFg: '#ffffff',
  protectedBorder: '#ffffff',
  monoFont: 'Iosevka Term',
  terminal: {
    palette,
    background: '#101010',
    foreground: '#f0f0f0',
    cursor: '#00ff00',
    cursorAccent: '#000000',
    selectionBackground: '#334455',
    selectionForeground: null,
  },
  ...overrides,
})

test('the colors come from the Ghostty palette the backend sends', () => {
  const result = terminalTheme(theme())
  expect(result).toMatchObject({
    background: '#101010',
    foreground: '#f0f0f0',
    cursor: '#00ff00',
    cursorAccent: '#000000',
    selectionBackground: '#334455',
    black: palette[0],
    red: palette[1],
    white: palette[7],
    brightBlack: palette[8],
    brightWhite: palette[15],
  })
  expect(result.selectionForeground).toBeUndefined()
})

test('a selection foreground from the theme is used', () => {
  const base = theme()
  const result = terminalTheme({
    ...base,
    terminal: { ...base.terminal!, selectionForeground: '#ffffff' },
  })
  expect(result.selectionForeground).toBe('#ffffff')
})

test('without a palette the UI tokens and a fallback palette fill every color', () => {
  const result = terminalTheme(theme({ terminal: undefined }))
  expect(result.background).toBe('#111111')
  expect(result.foreground).toBe('#eeeeee')
  expect(result.red).toBe('#aa0000')
  for (const name of ['black', 'green', 'brightRed', 'brightWhite'] as const)
    expect(result[name]).toMatch(/^#[0-9a-f]{6}$/i)
})

test('an empty palette from the first snapshot uses the fallback', () => {
  const base = theme()
  const result = terminalTheme({
    ...base,
    terminal: { ...base.terminal!, palette: [], background: '' },
  })
  expect(result.background).toBe('#111111')
  expect(result.green).toMatch(/^#[0-9a-f]{6}$/i)
})

test('no theme at all still gives a complete fallback', () => {
  const result = terminalTheme(null)
  expect(result.background).toBeTruthy()
  expect(result.foreground).toBeTruthy()
  expect(result.brightCyan).toBeTruthy()
})

test('the font is the theme mono font with fallbacks, and the size follows the font scale', () => {
  expect(terminalFont(theme({ fontScale: 1.5 }))).toEqual({
    fontFamily: '"Iosevka Term", "SF Mono", ui-monospace, Menlo, monospace',
    fontSize: 20,
  })
  expect(terminalFont(theme({ monoFont: null })).fontFamily).toBe(
    '"SF Mono", ui-monospace, Menlo, monospace',
  )
  expect(terminalFont(null).fontSize).toBe(13)
  expect(terminalFont(theme({ fontScale: 0.1 })).fontSize).toBe(8)
})
