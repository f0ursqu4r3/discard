/** Convert a keyboard event to the core keymap chord format. */
export function captureChord(event: KeyboardEvent): string | null {
  const key = event.key.toLowerCase()
  if (['meta', 'control', 'alt', 'shift'].includes(key)) return null
  const names: Record<string, string> = { ' ': 'space', arrowup: 'up', arrowdown: 'down', arrowleft: 'left', arrowright: 'right' }
  const parts = [event.ctrlKey && 'ctrl', event.altKey && 'alt', event.metaKey && 'cmd', event.shiftKey && 'shift', names[key] ?? key].filter(Boolean)
  return parts.join('-')
}
