/** Properties that plugin elements may use. */
export const PROP_WHITELIST = new Set([
  'gap',
  'padding',
  'padding_x',
  'padding_y',
  'width',
  'grow',
  'height_full',
  'align',
  'border_bottom',
  'border_top',
  'border_left',
  'border_right',
  'bg',
  'color',
  'size',
  'weight',
  'on_click',
  'variant',
  'name',
  'status',
  'count',
  'agent',
  'label',
])
const colors = new Set([
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
  'ansi_1',
  'ansi_2',
  'ansi_3',
  'ansi_4',
  'ansi_5',
  'ansi_6',
])
/** Clamp a numeric property before using it in a style. */
export function clampNumber(name: string, value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.max(0, Math.min(name === 'count' ? 9999 : 400, value))
}
/** Map a known color token to a CSS variable. */
export function colorToken(value: unknown, agent?: 'text' | 'mark'): string | undefined {
  if (value === 'agent' && agent)
    return agent === 'mark'
      ? 'var(--agent-color, var(--primary))'
      : 'var(--agent-text, var(--primary))'
  return typeof value === 'string' && colors.has(value)
    ? `var(--${value.replaceAll('_', '-')})`
    : undefined
}
