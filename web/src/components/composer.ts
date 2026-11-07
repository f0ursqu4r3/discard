/** An open channel accepts text, attachments, or both. */
export function maySend(archived: boolean, text: string, attachmentCount = 0): boolean {
  return !archived && (text.trim().length > 0 || attachmentCount > 0)
}
/** A running turn can be stopped. */
export function mayStop(running: boolean): boolean {
  return running
}
