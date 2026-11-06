export interface MemoryRevision {
  version: number
  scope: string
  content: string
  deleted: boolean
  updatedAt: number
  origin: string
}
export interface MemoryItem {
  id: string
  scope: string
  kind: string
  content: string
  version: number
  createdAt: number
  updatedAt: number
  deleted: boolean
  sourceConversation: string | null
  sourceAvailable: boolean
  history: MemoryRevision[]
}
export interface MemoryProposal {
  id: string
  action: string
  scope: string
  content: string
  sourceConversation: string
}
export interface MemoryRecord {
  key: string
  responseId: string
  type: 'automatic' | 'approved' | 'proposal'
  item?: MemoryItem
  proposalId?: string
  pending?: boolean
}
export interface MemoryResolution {
  status: 'approved' | 'rejected'
  committed: boolean
  associationPending: boolean
  warning: string | null
}
export interface MemoryView {
  warnings?: string[]
  agentId?: string | null
  projectId?: string | null
  items: MemoryItem[]
  scopes: { scope: string; title: string }[]
  proposals: MemoryProposal[]
  responses: MemoryRecord[]
  paused: boolean
}
