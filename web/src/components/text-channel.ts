/** Preview of the name that the app saves for a text channel. The app checks the final name. */
export function normalizeTextChannelName(input: string): string {
  return input
    .trim()
    .toLowerCase()
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-+|-+$/g, '')
}
