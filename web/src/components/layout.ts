/** Height of the server indicator, in spacing units. */
export function serverPillHeight(selected: boolean, hovered: boolean, unread: boolean, itemHeight = 12): number {
  return selected ? itemHeight : hovered ? itemHeight / 2 : unread ? 2 : 0
}

/** Up to two initials from the project name. */
export function serverInitials(name: string): string {
  const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
  if (words.length === 0) return ''
  if (words.length === 1) return Array.from(words[0]!).slice(0, 2).join('').toUpperCase()
  return (Array.from(words[0]!)[0]! + Array.from(words[1]!)[0]!).toUpperCase()
}
/** Only persist a complete layout, never the intermediate layout during a mount. */
export function savePanelWidths(visible: boolean, count: number): boolean { return count === (visible ? 3 : 2) }
/** Select the settings action and label for the review count. */
export function settingsTarget(count: number): [string, string] {
  return count > 0 ? ['review_plugins', `Review plugins (${count})`] : ['open_settings', 'Settings']
}
const clampPanel = (value: number) => Math.min(400, Math.max(180, value))
/** The native store still owns channel-only widths; the displayed panel includes the rail. */
export function navigationWidth(channelWidth: number, railWidth = 72): number { return clampPanel(channelWidth) + railWidth }
export function panelWidths(sizes: number[], railWidth = 72, memberWidth = 240): [number, number] {
  return [clampPanel(sizes[0]! - railWidth), clampPanel(sizes[2] ?? memberWidth)]
}
