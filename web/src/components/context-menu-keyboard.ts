/** Open a context menu from the keyboard: the Menu key or Shift+F10 acts like a right click on the element. */
export function openContextMenuFromKeyboard(event: KeyboardEvent) {
  if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return
  event.preventDefault()
  const element = event.currentTarget as HTMLElement
  const bounds = element.getBoundingClientRect()
  element.dispatchEvent(
    new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: bounds.right,
      clientY: bounds.top,
    }),
  )
}
