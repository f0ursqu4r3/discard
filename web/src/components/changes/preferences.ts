/** Keys of the small preferences of the Changes tab. They stay in the webview. */
export const preferenceKeys = {
  scope: 'discard:changes-scope',
  layout: 'discard:changes-layout',
  files: 'discard:changes-files',
} as const

/** Read a saved choice. Anything else than an allowed value, or no storage, gives the fallback. */
export function readChoice<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const saved = localStorage.getItem(key)
    return allowed.find((value) => value === saved) ?? fallback
  } catch {
    return fallback
  }
}

export function saveChoice(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // The choice still holds for this session.
  }
}
