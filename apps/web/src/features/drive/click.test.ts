import { expect, test } from 'bun:test'
import { driveClick } from './click'

const keys = (pressed: Partial<Record<'shiftKey' | 'metaKey' | 'ctrlKey', boolean>> = {}) => ({ shiftKey: false, metaKey: false, ctrlKey: false, ...pressed })

test('a plain click opens; Ctrl/Cmd toggles; Shift selects a range from the last pick', () => {
  // A plain click selected the item, and only a double click opened it (2026-11-10).
  expect(driveClick(keys(), false)).toBe('open')
  expect(driveClick(keys(), true)).toBe('open')
  expect(driveClick(keys({ ctrlKey: true }), true)).toBe('toggle')
  expect(driveClick(keys({ metaKey: true }), false)).toBe('toggle')
  expect(driveClick(keys({ shiftKey: true }), true)).toBe('range')
  // Shift with nothing picked yet starts the selection there
  expect(driveClick(keys({ shiftKey: true }), false)).toBe('toggle')
})
