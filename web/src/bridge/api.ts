import { invoke } from '@tauri-apps/api/core'
import type { MessageDto, SnapshotDto } from './types'
export const getSnapshot = () => invoke<SnapshotDto>('get_snapshot')
export const getHistory = () => invoke<MessageDto[]>('get_history')
export const sendMessage = (text: string) => invoke<void>('send_message', { text })
export const decideApproval = (approvalId: string, decision: string) => invoke<void>('decide_approval', { approvalId, decision })
export const cancelTurn = () => invoke<void>('cancel_turn')
