export type WindowMode = { kind: 'main' } | { kind: 'project'; projectId: string }

const projectPrefix = '#/project/'

/**
 * The mode of this window from its URL hash. A project window loads `#/project/<project ID>`. The core encodes the ID,
 * so it is decoded here. Any other hash, an empty ID, or an ID that cannot be decoded gives the main window.
 */
export function windowMode(hash: string): WindowMode {
  if (!hash.startsWith(projectPrefix)) return { kind: 'main' }
  try {
    const projectId = decodeURIComponent(hash.slice(projectPrefix.length))
    return projectId ? { kind: 'project', projectId } : { kind: 'main' }
  } catch {
    return { kind: 'main' }
  }
}
