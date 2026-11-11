import { expect, test } from 'vite-plus/test'
import { activeTheme, applyTheme } from '../theme'
import type { ThemeDto } from '../types'
test('applyTheme sets every token and toggles dark', () => {
  const tokens = Object.fromEntries(
    [
      'background',
      'foreground',
      'sidebar',
      'sidebar_foreground',
      'sidebar_accent',
      'primary',
      'primary_foreground',
      'muted',
      'muted_foreground',
      'border',
      'accent',
      'success',
      'warning',
      'danger',
    ].map((name) => [name, '#123456']),
  )
  const theme: ThemeDto = {
    mode: 'dark',
    tokens,
    radius: 4,
    fontScale: 1.2,
    spacingScale: 1.1,
    protectedBg: '#000000',
    protectedFg: '#ffffff',
    protectedBorder: '#aaaaaa',
  }
  const root = document.createElement('div')
  applyTheme(theme, root)
  for (const name of Object.keys(tokens))
    expect(root.style.getPropertyValue(`--${name.replaceAll('_', '-')}`)).toBe('#123456')
  expect(root.style.getPropertyValue('--radius')).toBe('4px')
  expect(root.style.getPropertyValue('--font-scale')).toBe('1.2')
  expect(root.style.getPropertyValue('--space-scale')).toBe('1.1')
  expect(root.style.getPropertyValue('--protected-bg')).toBe('#000000')
  expect(root.classList.contains('dark')).toBe(true)
  applyTheme({ ...theme, mode: 'light' }, root)
  expect(root.classList.contains('dark')).toBe(false)
})

test('removes tokens that are absent from the next theme', () => {
  const root = document.createElement('div')
  const theme: ThemeDto = {
    mode: 'dark',
    tokens: { warning: '#ffaa00' },
    radius: null,
    fontScale: 1,
    spacingScale: 1,
    protectedBg: '#000000',
    protectedFg: '#ffffff',
    protectedBorder: '#ffffff',
  }
  applyTheme(theme, root)
  applyTheme({ ...theme, tokens: {} }, root)
  expect(root.style.getPropertyValue('--warning')).toBe('')
})

test('applies the resolved diff colors', () => {
  const root = document.createElement('div')
  const theme: ThemeDto = {
    mode: 'dark',
    tokens: {},
    radius: null,
    fontScale: 1,
    spacingScale: 1,
    protectedBg: '#000000',
    protectedFg: '#ffffff',
    protectedBorder: '#aaaaaa',
    diffColors: {
      added: '#123456',
      removed: '#abcdef',
      inlineAdded: '#654321',
      inlineRemoved: '#fedcba',
    },
  }
  applyTheme(theme, root)
  expect(root.style.getPropertyValue('--diff-added')).toBe('#123456')
  expect(root.style.getPropertyValue('--diff-removed-inline')).toBe('#fedcba')
})

const monoTheme: ThemeDto = {
  mode: 'dark',
  tokens: { ansi_4: '#61afef' },
  monoFont: 'SF Mono',
  radius: null,
  fontScale: 1,
  spacingScale: 1,
  protectedBg: '#000000',
  protectedFg: '#ffffff',
  protectedBorder: '#aaaaaa',
}
test('applyTheme sets the mono family and the ansi tokens', () => {
  const root = document.createElement('div')
  applyTheme(monoTheme, root)
  expect(root.style.getPropertyValue('--ansi-4')).toBe('#61afef')
  expect(root.style.getPropertyValue('--mono-family')).toBe('"SF Mono"')
})
test('applyTheme removes the mono family when the theme has none', () => {
  const root = document.createElement('div')
  applyTheme(monoTheme, root)
  root.style.setProperty('--mono-family', '"SF Mono"')
  applyTheme({ ...monoTheme, monoFont: null }, root)
  expect(root.style.getPropertyValue('--mono-family')).toBe('')
})
test('applyTheme publishes the active theme', () => {
  applyTheme(monoTheme, document.createElement('div'))
  expect(activeTheme.value).toBe(monoTheme)
})
