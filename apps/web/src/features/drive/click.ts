/**
 * What a click on a Drive item does: a plain click (or tap) opens it, Ctrl/Cmd+click adds it to the selection or takes
 * it out, Shift+click selects the range from the last item picked. Only the checkbox selects without a key.
 */
export function driveClick(event: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }, hasAnchor: boolean): 'open' | 'toggle' | 'range' {
  if (event.shiftKey && hasAnchor) return 'range'
  if (event.shiftKey || event.metaKey || event.ctrlKey) return 'toggle'
  return 'open'
}
