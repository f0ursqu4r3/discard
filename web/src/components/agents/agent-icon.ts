import { defineAsyncComponent, type Component } from 'vue'
import { Bot } from '@lucide/vue'
import { fallbackTokens } from '@/bridge/theme'

// Keep the full catalog searchable, but load only the icons that are displayed.
const modules = import.meta.glob<{ default: Component }>([
  '/node_modules/@lucide/vue/dist/esm/icons/*.mjs',
  '!/node_modules/@lucide/vue/dist/esm/icons/index.mjs',
])
const loaders = new Map(
  Object.entries(modules).map(([path, load]) => [path.slice(path.lastIndexOf('/') + 1, -4), load]),
)
export const iconNames = [...loaders.keys()].sort()
export const defaultIconColor = 'ansi-4'
const components = new Map<string, Component>()
export function iconComponent(name: string): Component {
  const load = loaders.get(name)
  if (!load) return Bot
  if (!components.has(name)) components.set(name, defineAsyncComponent(load))
  return components.get(name)!
}
export function iconLabel(name: string) {
  return name.replaceAll('-', ' ')
}
export function searchIcons(query: string) {
  const normalized = query.toLowerCase().replace(/[^a-z0-9]/g, '')
  return iconNames.filter((name) => name.replaceAll('-', '').includes(normalized))
}
export function validIconColor(color: string) {
  return /^(#[0-9a-f]{6}|ansi-[1-6])$/i.test(color)
}
// A tagged value uses the existing persisted avatar field and preserves legacy URLs.
export function parseAgentIcon(
  avatar?: string | null,
): { icon: string | null; color: string } | null {
  const match = /^lucide:([a-z0-9-]*):(#[0-9a-f]{6}|ansi-[1-6])$/i.exec(avatar ?? '')
  return match && (!match[1] || loaders.has(match[1]))
    ? { icon: match[1] || null, color: match[2]!.toLowerCase() }
    : null
}
export function formatAgentIcon(icon: string | null, color: string) {
  if ((icon !== null && !loaders.has(icon)) || !validIconColor(color))
    throw new Error('Choose a valid icon and color.')
  return `lucide:${icon ?? ''}:${color.toLowerCase()}`
}
export type PaletteSlot = 1 | 2 | 3 | 4 | 5 | 6
export const paletteSlots: readonly { slot: PaletteSlot; label: string }[] = [
  { slot: 1, label: 'Red' },
  { slot: 2, label: 'Green' },
  { slot: 3, label: 'Yellow' },
  { slot: 4, label: 'Blue' },
  { slot: 5, label: 'Magenta' },
  { slot: 6, label: 'Cyan' },
]

export function hashedSlot(agentId: string): PaletteSlot {
  let hash = 0x811c9dc5
  for (let i = 0; i < agentId.length; i++) {
    hash ^= agentId.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193) >>> 0
  }
  return ((hash % 6) + 1) as PaletteSlot
}

export function resolveColor(color: string, tokens: Record<string, string>): string {
  if (!/^ansi-[1-6]$/i.test(color)) return color
  const token = color.toLowerCase().replace('-', '_')
  return tokens[token] ?? fallbackTokens[token]!
}

function channels(color: string): number[] {
  return [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16))
}

function luminance(color: string): number {
  const rgb = channels(color).map((channel) => {
    const value = channel / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  return rgb[0]! * 0.2126 + rgb[1]! * 0.7152 + rgb[2]! * 0.0722
}

export function contrastRatio(a: string, b: string): number {
  const first = luminance(a),
    second = luminance(b)
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05)
}

export function iconForeground(color: string): string {
  return luminance(color) > 0.179 ? fallbackTokens.icon_dark! : fallbackTokens.icon_light!
}

function mixColor(start: string, end: string, fraction: number): string {
  const target = channels(end)
  return (
    '#' +
    channels(start)
      .map((channel, i) =>
        Math.round(channel * (1 - fraction) + target[i]! * fraction)
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')
  )
}

export function readableOn(
  color: string,
  background: string,
  foreground: string,
  min: number,
): string {
  if (contrastRatio(color, background) >= min) return color
  for (let step = 1; step <= 10; step++) {
    const mixed = mixColor(color, foreground, step / 10)
    if (contrastRatio(mixed, background) >= min) return mixed
  }
  return foreground
}

export function agentIdentity(
  avatar: string | null | undefined,
  agentId: string,
  tokens: Record<string, string>,
) {
  const stored = parseAgentIcon(avatar)?.color ?? `ansi-${hashedSlot(agentId)}`
  const color = resolveColor(stored, tokens)
  const bg = tokens.background ?? fallbackTokens.background!
  const fg = tokens.foreground ?? fallbackTokens.foreground!
  const mark = readableOn(color, bg, fg, 3)
  return {
    color,
    text: readableOn(color, bg, fg, 4.5),
    mark,
    avatarMark: readableOn(mark, mixColor(bg, mark, 0.15), fg, 3),
  }
}
