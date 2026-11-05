export type BlockDto =
  | { type: 'markdown' | 'diff'; text: string }
  | { type: 'code'; language: string | null; text: string }
  | { type: 'tool_call'; call_id: string; name: string; input: unknown; output: string | null }
  | { type: 'approval'; approval_id: string; tool: string; summary: string }
export interface MessageDto { id: string; role: string; blocks: BlockDto[]; continuesGroup: boolean }
export interface PromptDto { approvalId: string; tool: string; summary: string; view: string }
export interface BranchRowDto { key: string; label: string; kind: string; depth: number; current: boolean; collapsed: boolean }
export interface ThemeDto { mode: string; tokens: Record<string, string>; radius: number | null; fontScale: number; spacingScale: number; protectedBg: string; protectedFg: string; protectedBorder: string }
export interface ElementDto { tag: string; props: Record<string, boolean | number | string>; children: unknown[] }
export interface SlotDto { plugin: string; element: ElementDto }
export interface BannersDto { reviewCount: number; problemLines: string[]; confirm: string | null; archive: { message: string } | null; safeMode: boolean }
export interface SnapshotDto {
  rev: number; historyRev: number; project: string; channel: string; agent: string; provider: string;
  agentStatus: string; turnRunning: boolean; notice: string | null; liveBlocks: BlockDto[]; prompts: PromptDto[];
  pendingApprovals: number; unread: boolean; archived: boolean; archiveBusy: boolean; memberListVisible: boolean;
  panels: { channelList: number; memberList: number }; branches: BranchRowDto[]; slots: Record<string, SlotDto>;
  banners: BannersDto; theme: ThemeDto
}
