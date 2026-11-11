import { describe, expect, test } from 'bun:test'
import { fromLocalInput, localInputValue, sendLaterPresets } from './times'

describe('sendLaterPresets', () => {
  test('tomorrow and next Monday are at 9:00 local time', () => {
    // Wednesday 2026-11-11 14:20 local
    const [hour, tomorrow, monday] = sendLaterPresets(new Date(2026, 10, 11, 14, 20))
    expect(hour.at.getTime()).toBe(new Date(2026, 10, 11, 15, 20).getTime())
    expect(tomorrow.at.getTime()).toBe(new Date(2026, 10, 12, 9, 0).getTime())
    expect(monday.at.getTime()).toBe(new Date(2026, 10, 16, 9, 0).getTime())
  })

  test('on a Monday, "Monday" is the next week, not today', () => {
    const [, , monday] = sendLaterPresets(new Date(2026, 10, 16, 8, 0))
    expect(monday.at.getTime()).toBe(new Date(2026, 10, 23, 9, 0).getTime())
  })
})

describe('local input values', () => {
  test('round-trip to the minute', () => {
    const date = new Date(2026, 0, 5, 7, 3)
    expect(localInputValue(date)).toBe('2026-01-05T07:03')
    expect(fromLocalInput('2026-01-05T07:03')?.getTime()).toBe(date.getTime())
  })

  test('an empty or broken value is no time', () => {
    expect(fromLocalInput('')).toBeNull()
    expect(fromLocalInput('tomorrow')).toBeNull()
  })
})
