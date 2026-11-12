import type { ITheme } from '@xterm/xterm'
import type { ThemeDto } from '@/bridge/types'
import { fallbackTokens } from '@/bridge/theme'

/** Used only before the first theme arrives, or when the backend sends no palette. */
const fallbackPalette = [
  '#1d1f21',
  '#cc6666',
  '#b5bd68',
  '#f0c674',
  '#81a2be',
  '#b294bb',
  '#8abeb7',
  '#c5c8c6',
  '#666666',
  '#d54e53',
  '#b9ca4a',
  '#e7c547',
  '#7aa6da',
  '#c397d8',
  '#70c0b1',
  '#eaeaea',
]
const colorNames = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const

/** The 16 ANSI colors: the Ghostty palette when the backend sends it, else the UI ANSI tokens over the fallback. */
function palette(theme: ThemeDto | null): string[] {
  const sent = theme?.terminal?.palette
  if (sent && sent.length === 16) return [...sent]
  const tokens = theme?.tokens ?? {}
  return fallbackPalette.map(
    (color, index) => (index >= 1 && index <= 6 ? tokens[`ansi_${index}`] : undefined) ?? color,
  )
}

/** xterm colors from the app theme. */
export function terminalTheme(theme: ThemeDto | null): ITheme {
  const tokens = theme?.tokens ?? {}
  const colors = theme?.terminal
  const background =
    colors?.background || tokens.background || fallbackTokens.background || '#101010'
  const foreground =
    colors?.foreground || tokens.foreground || fallbackTokens.foreground || '#e6e6e6'
  const result: ITheme = {
    background,
    foreground,
    cursor: colors?.cursor || foreground,
    cursorAccent: colors?.cursorAccent || background,
    selectionBackground:
      colors?.selectionBackground || tokens.sidebar_accent || tokens.accent || '#3a3f4b',
  }
  if (colors?.selectionForeground) result.selectionForeground = colors.selectionForeground
  palette(theme).forEach((color, index) => {
    result[colorNames[index]!] = color
  })
  return result
}

const BASE_FONT_SIZE = 13

/** The mono font of the app theme and the size that follows the UI font scale. */
export function terminalFont(theme: ThemeDto | null): { fontFamily: string; fontSize: number } {
  const families = ['"SF Mono"', 'ui-monospace', 'Menlo', 'monospace']
  if (theme?.monoFont) families.unshift(`"${theme.monoFont}"`)
  return {
    fontFamily: families.join(', '),
    fontSize: Math.max(8, Math.round(BASE_FONT_SIZE * (theme?.fontScale ?? 1))),
  }
}
