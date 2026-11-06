import { defineAsyncComponent, type Component } from 'vue'
import { Bot } from '@lucide/vue'

// Keep the full catalog searchable, but load only the icons that are displayed.
const modules = import.meta.glob<{ default: Component }>([
  '/node_modules/@lucide/vue/dist/esm/icons/*.mjs',
  '!/node_modules/@lucide/vue/dist/esm/icons/index.mjs',
])
const loaders = new Map(
  Object.entries(modules).map(([path, load]) => [path.slice(path.lastIndexOf('/') + 1, -4), load]),
)
export const iconNames = [...loaders.keys()].sort()
export const defaultIconColor = '#789cf5'
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
  return /^#[0-9a-f]{6}$/i.test(color)
}
// A tagged value uses the existing persisted avatar field and preserves legacy URLs.
export function parseAgentIcon(
  avatar?: string | null,
): { icon: string | null; color: string } | null {
  const match = /^lucide:([a-z0-9-]*):(#[0-9a-f]{6})$/i.exec(avatar ?? '')
  return match && (!match[1] || loaders.has(match[1]))
    ? { icon: match[1] || null, color: match[2]!.toLowerCase() }
    : null
}
export function formatAgentIcon(icon: string | null, color: string) {
  if ((icon !== null && !loaders.has(icon)) || !validIconColor(color))
    throw new Error('Choose a valid icon and color.')
  return `lucide:${icon ?? ''}:${color.toLowerCase()}`
}
export function iconForeground(color: string) {
  const rgb = [1, 3, 5].map((offset) => {
    const value = parseInt(color.slice(offset, offset + 2), 16) / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  const luminance = rgb[0]! * 0.2126 + rgb[1]! * 0.7152 + rgb[2]! * 0.0722
  return luminance > 0.179 ? '#000000' : '#ffffff'
}
