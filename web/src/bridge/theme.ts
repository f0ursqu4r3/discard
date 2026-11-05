import type { ThemeDto } from './types'
const appliedTokens = new WeakMap<HTMLElement, Set<string>>()

export function applyTheme(theme: ThemeDto, root: HTMLElement) {
  const tokens = new Set(Object.keys(theme.tokens).map((name) => `--${name.replaceAll('_', '-')}`))
  for (const name of appliedTokens.get(root) ?? []) {
    if (!tokens.has(name)) root.style.removeProperty(name)
  }
  for (const [name, value] of Object.entries(theme.tokens))
    root.style.setProperty(`--${name.replaceAll('_', '-')}`, value)
  appliedTokens.set(root, tokens)
  if (theme.radius === null) root.style.removeProperty('--radius')
  else root.style.setProperty('--radius', `${theme.radius}px`)
  root.style.setProperty('--font-scale', String(theme.fontScale))
  root.style.setProperty('--space-scale', String(theme.spacingScale))
  root.style.setProperty('--protected-bg', theme.protectedBg)
  root.style.setProperty('--protected-fg', theme.protectedFg)
  root.style.setProperty('--protected-border', theme.protectedBorder)
  const diffColors = theme.diffColors
  for (const [name, value] of [
    ['--diff-added', diffColors?.added],
    ['--diff-removed', diffColors?.removed],
    ['--diff-added-inline', diffColors?.inlineAdded],
    ['--diff-removed-inline', diffColors?.inlineRemoved],
  ] as const) {
    if (value) root.style.setProperty(name, value)
    else root.style.removeProperty(name)
  }
  root.classList.toggle('dark', theme.mode === 'dark')
}
