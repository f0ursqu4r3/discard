// Replace a leading home directory with `~`. When `home` is not given, infer it
// from the path itself (`/Users/<name>` or `/home/<name>`), so no backend call is needed.
const HOME_PATTERN = /^(?:\/Users|\/home)\/[^/]+(?=\/|$)/

export function shortPath(path: string, home?: string): string {
  const base = home?.replace(/\/+$/, '') || HOME_PATTERN.exec(path)?.[0]
  if (!base) return path
  if (path === base) return '~'
  return path.startsWith(`${base}/`) ? `~${path.slice(base.length)}` : path
}
