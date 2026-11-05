/** A viewport position, kept by channel while the transcript is not mounted. */
export interface TranscriptPosition {
  messageId: string | null
  offset: number
  top: number
  following: boolean
}
