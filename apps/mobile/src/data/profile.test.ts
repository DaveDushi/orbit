import { expect, test } from 'bun:test'

import { phoneValid, profileChanges, profileFormOf } from './profile'

const saved = { display_name: 'Ada', title: 'Engineer', phone: null, timezone: 'Europe/London', bio: 'Hello' }

test('the form starts from the saved profile, empty text for what is not set', () => {
  expect(profileFormOf(saved)).toEqual({ name: 'Ada', title: 'Engineer', phone: '', timezone: 'Europe/London', bio: 'Hello' })
})

test('a save has the name and only the parts that changed; an emptied part goes as empty text', () => {
  const form = profileFormOf(saved)
  expect(profileChanges(saved, form)).toBeNull()
  expect(profileChanges(saved, { ...form, title: ' Engineer ' })).toBeNull()
  expect(profileChanges(saved, { ...form, title: 'Designer' })).toEqual({ display_name: 'Ada', title: 'Designer' })
  expect(profileChanges(saved, { ...form, name: 'Ada L', bio: '', timezone: '' })).toEqual({ display_name: 'Ada L', bio: '', timezone: '' })
  expect(profileChanges(saved, { ...form, name: '  ', title: 'CTO' })).toBeNull()
})

test('a phone number has a digit and only digits, spaces and + - ( ) .', () => {
  for (const phone of ['+36 (30) 123-4567', '555 0100', '020.7946.0958']) expect(phoneValid(phone)).toBe(true)
  for (const phone of ['call me', '+-()', '555 0100 ext 2', '']) expect(phoneValid(phone)).toBe(false)
})
