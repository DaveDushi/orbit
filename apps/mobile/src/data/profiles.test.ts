import { expect, test } from 'bun:test'

import { localTimeLabel, noteToSave, phoneHref } from './profiles'

test('local time shows in the member zone, and nothing for a missing or unknown zone', () => {
  const noon = Date.UTC(2026, 10, 17, 12, 0)
  expect(localTimeLabel('Asia/Jerusalem', noon, 'en-US')).toBe('2:00 PM local time')
  expect(localTimeLabel(null, noon)).toBe(null)
  expect(localTimeLabel('Mars/Base', noon)).toBe(null)
})

test('a phone link keeps the digits and a leading plus', () => {
  expect(phoneHref(' +972 (54) 123-4567 ')).toBe('tel:+972541234567')
  expect(phoneHref('054.123')).toBe('tel:054123')
})

test('a note saves only when it changed; an emptied note removes it', () => {
  expect(noteToSave('Likes tea', ' Likes tea ')).toBe(null)
  expect(noteToSave('Likes tea', 'Likes coffee')).toBe('Likes coffee')
  expect(noteToSave('Likes tea', '  ')).toBe('')
  expect(noteToSave(null, '')).toBe(null)
})
