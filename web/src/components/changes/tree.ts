import type { ChangeFileDto } from '../../bridge/types'

export interface FolderNode {
  type: 'folder'
  /** The shown name. A chain of single-child folders becomes one name, such as `discard-core/src`. */
  name: string
  /** The full path of the deepest folder of the chain. */
  path: string
  children: TreeNode[]
  fileCount: number
  additions: number
  deletions: number
}
export interface FileNode {
  type: 'file'
  name: string
  path: string
  file: ChangeFileDto
}
export type TreeNode = FolderNode | FileNode
export interface TreeRow {
  node: TreeNode
  depth: number
}

/** Keep the files whose path contains `query`, ignoring case. A blank query keeps every file. */
export function filterFiles(files: ChangeFileDto[], query: string): ChangeFileDto[] {
  const needle = query.trim().toLowerCase()
  return needle ? files.filter((file) => file.path.toLowerCase().includes(needle)) : files
}

const byName = (a: TreeNode, b: TreeNode) => a.name.localeCompare(b.name, 'en')

function sort(nodes: TreeNode[]) {
  const folders = nodes.filter((node) => node.type === 'folder').sort(byName)
  const files = nodes.filter((node) => node.type === 'file').sort(byName)
  return [...folders, ...files]
}

/** Build the folder tree of the changed files. Folders come first, then files, each by name. */
export function buildTree(files: ChangeFileDto[]): TreeNode[] {
  interface Draft {
    folders: Map<string, Draft>
    files: FileNode[]
  }
  const root: Draft = { folders: new Map(), files: [] }
  for (const file of files) {
    const parts = file.path.split('/')
    const name = parts.pop() ?? file.path
    let at = root
    for (const part of parts) {
      let next = at.folders.get(part)
      if (!next) at.folders.set(part, (next = { folders: new Map(), files: [] }))
      at = next
    }
    at.files.push({ type: 'file', name, path: file.path, file })
  }
  function finish(draft: Draft, prefix: string): TreeNode[] {
    const nodes: TreeNode[] = [...draft.files]
    for (const [name, child] of draft.folders) {
      let shown = name
      let path = prefix ? `${prefix}/${name}` : name
      let current = child
      // A folder that holds exactly one folder and nothing else joins that folder.
      while (current.files.length === 0 && current.folders.size === 1) {
        const [[nextName, next]] = [...current.folders]
        shown += `/${nextName}`
        path += `/${nextName}`
        current = next!
      }
      const children = finish(current, path)
      const totals = children.reduce(
        (sum, node) => {
          if (node.type === 'folder') {
            sum.files += node.fileCount
            sum.additions += node.additions
            sum.deletions += node.deletions
          } else {
            sum.files += 1
            sum.additions += node.file.additions
            sum.deletions += node.file.deletions
          }
          return sum
        },
        { files: 0, additions: 0, deletions: 0 },
      )
      nodes.push({
        type: 'folder',
        name: shown,
        path,
        children,
        fileCount: totals.files,
        additions: totals.additions,
        deletions: totals.deletions,
      })
    }
    return sort(nodes)
  }
  return finish(root, '')
}

/** The rows to draw, top to bottom. The children of a folder in `collapsed` are left out. */
export function flattenTree(nodes: TreeNode[], collapsed: ReadonlySet<string>): TreeRow[] {
  const rows: TreeRow[] = []
  const walk = (list: TreeNode[], depth: number) => {
    for (const node of list) {
      rows.push({ node, depth })
      if (node.type === 'folder' && !collapsed.has(node.path)) walk(node.children, depth + 1)
    }
  }
  walk(nodes, 0)
  return rows
}

/** The text color class of a status letter. */
export function statusTone(status: string): string {
  if (status === 'A' || status === '?') return 'text-success'
  if (status === 'D') return 'text-destructive'
  if (status === 'M') return 'text-warning'
  return 'text-muted-foreground'
}

/** The order that the tree shows the files in. */
export function treeOrder(files: ChangeFileDto[]): ChangeFileDto[] {
  const out: ChangeFileDto[] = []
  const walk = (nodes: TreeNode[]) => {
    for (const node of nodes) {
      if (node.type === 'file') out.push(node.file)
      else walk(node.children)
    }
  }
  walk(buildTree(files))
  return out
}
