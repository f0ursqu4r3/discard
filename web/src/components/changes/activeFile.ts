/** The file at the top of the pane: the last section that starts at or above the pane top plus `offset`. Else the first section. */
export function activeFilePath(
  paneTop: number,
  sections: { path: string; top: number }[],
  offset = 16,
): string | null {
  let active = sections[0]?.path ?? null
  for (const section of sections) {
    if (section.top <= paneTop + offset) active = section.path
  }
  return active
}
