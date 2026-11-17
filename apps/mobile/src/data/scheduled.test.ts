import { describe, expect, test } from 'bun:test'

import { canScheduleFrom, initialSendTime, type ScheduledMessage, scheduledHere, scheduledWhere, sendLaterPresets, whenLabel } from './scheduled'

describe('sendLaterPresets', () => {
  test('an hour on, tomorrow 9:00 and the next Monday 9:00, in local time', () => {
    const wednesday = new Date(2026, 10, 18, 14, 30) // Wed 18 Nov 2026
    const [hour, tomorrow, monday] = sendLaterPresets(wednesday)
    expect(hour!.at.getTime() - wednesday.getTime()).toBe(3_600_000)
    expect([tomorrow!.at.getDate(), tomorrow!.at.getHours(), tomorrow!.at.getMinutes()]).toEqual([19, 9, 0])
    expect([monday!.at.getDay(), monday!.at.getDate(), monday!.at.getHours()]).toEqual([1, 23, 9])
  })

  test('on a Monday, "Monday" is the next week, not today', () => {
    const monday = new Date(2026, 10, 16, 8, 0)
    expect(sendLaterPresets(monday)[2]!.at.getDate()).toBe(23)
  })

  test('across a month end and a daylight saving change the hour stays 9:00', () => {
    // 28 March 2026 is the Saturday before the EU change; the calendar day moves, not 24 hours
    const [, tomorrow, monday] = sendLaterPresets(new Date(2026, 2, 28, 12, 0))
    expect([tomorrow!.at.getMonth(), tomorrow!.at.getDate(), tomorrow!.at.getHours()]).toEqual([2, 29, 9])
    expect([monday!.at.getMonth(), monday!.at.getDate(), monday!.at.getHours()]).toEqual([2, 30, 9])
    const [, newYear] = sendLaterPresets(new Date(2026, 11, 31, 22, 0))
    expect([newYear!.at.getFullYear(), newYear!.at.getMonth(), newYear!.at.getDate()]).toEqual([2027, 0, 1])
  })
})

test('whenLabel says Today and Tomorrow by calendar day, not by 24 hours', () => {
  const now = new Date(2026, 10, 17, 23, 50)
  expect(whenLabel(new Date(2026, 10, 17, 23, 55), now)).toStartWith('Today ')
  expect(whenLabel(new Date(2026, 10, 18, 0, 5), now)).toStartWith('Tomorrow ')
  expect(whenLabel(new Date(2026, 10, 20, 9, 0), now)).not.toMatch(/^(Today|Tomorrow)/)
})

test('a failed message in the past starts an hour from now; a future one keeps its time', () => {
  const now = new Date(2026, 10, 17, 10, 0)
  expect(initialSendTime(now, new Date(2026, 10, 17, 9, 0)).getTime()).toBe(now.getTime() + 3_600_000)
  expect(initialSendTime(now, new Date(2026, 10, 18, 9, 0)).getDate()).toBe(18)
})

const scheduled = (id: string, extra: Partial<ScheduledMessage>): ScheduledMessage => ({
  id,
  workspace_id: 'w',
  kind: 'message',
  conversation_id: 'c1',
  conversation_name: 'general',
  conversation_kind: 'public',
  body: 'hi',
  send_at: 2,
  status: 'scheduled',
  error: null,
  created_at: 0,
  updated_at: 0,
  ...extra,
})

test('the bar of a conversation counts its own messages, soonest first, and the failed ones', () => {
  const list = [scheduled('a', { send_at: 5 }), scheduled('b', { send_at: 3, status: 'failed' }), scheduled('r', { kind: 'reminder', conversation_id: 'c1' }), scheduled('o', { conversation_id: 'c2' })]
  const here = scheduledHere(list, 'c1')
  expect(here.items.map((item) => item.id)).toEqual(['b', 'a'])
  expect(here.failed).toBe(1)
  expect(scheduledWhere(list[2]!)).toBe('Reminder')
  expect(scheduledWhere(scheduled('d', { conversation_kind: 'dm' }))).toBe('Direct message')
  expect(scheduledWhere(scheduled('x', { conversation_name: null }))).toBe('Deleted conversation')
})

test('only new text in a conversation can be scheduled, never by a guest', () => {
  const base = { text: 'hello', files: 0, editing: false, inThread: false, askDonna: false, guest: false }
  expect(canScheduleFrom(base)).toBe(true)
  expect(canScheduleFrom({ ...base, text: '  ' })).toBe(false)
  expect(canScheduleFrom({ ...base, files: 1 })).toBe(false)
  expect(canScheduleFrom({ ...base, editing: true })).toBe(false)
  expect(canScheduleFrom({ ...base, inThread: true })).toBe(false)
  expect(canScheduleFrom({ ...base, askDonna: true })).toBe(false)
  expect(canScheduleFrom({ ...base, guest: true })).toBe(false)
})
