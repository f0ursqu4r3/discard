/** Height of the server indicator, in spacing units. */
export function serverPillHeight(selected: boolean, hovered: boolean, unread: boolean): number {
  return selected ? 8 : hovered ? 4 : unread ? 2 : 0
}

/** Up to two initials from the project name. */
export function serverInitials(name: string): string {
  const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
  if (words.length === 0) return ''
  if (words.length === 1) return Array.from(words[0]!).slice(0, 2).join('').toUpperCase()
  return (Array.from(words[0]!)[0]! + Array.from(words[1]!)[0]!).toUpperCase()
}
/** Save widths only while all three panels are visible. */
export function savePanelWidths(visible: boolean, count: number): boolean { return visible && count === 3 }
/** Select the settings action and label for the review count. */
export function settingsTarget(count: number): [string, string] {
  return count > 0 ? ['review_plugins', `Review plugins (${count})`] : ['open_settings', 'Settings']
}
/** Side panels report their widths in pixels. */
export function panelWidths(sizes: number[]): [number, number] { return [sizes[0]!, sizes[2]!] }
