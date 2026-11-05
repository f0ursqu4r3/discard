/** A message can be sent when the channel is open and it has text. */
export function maySend(archived: boolean, text: string): boolean {
  return !archived && text.trim().length > 0
}
/** A running turn can be stopped. */
export function mayStop(running: boolean): boolean {
  return running
}
