import { expect, test } from 'bun:test'

import { exportPollInterval, invitationForOther, invitationToken } from './workspaces'

const SERVER = 'https://orbit.example.com'

test('invitationToken takes the token of a link to this server', () => {
  expect(invitationToken(' https://orbit.example.com/accept-invitation?token=abc_DEF-123 ', SERVER)).toEqual({ token: 'abc_DEF-123' })
  expect(invitationToken('https://ORBIT.example.com/accept-invitation/?token=t0k3n', SERVER)).toEqual({ token: 't0k3n' })
})

test('invitationToken refuses a link to another server, another page or no token', () => {
  expect(invitationToken('https://other.example.com/accept-invitation?token=abc', SERVER)).toEqual({ problem: 'This invitation is for other.example.com. Sign in to that server to accept it.' })
  expect(invitationToken('https://orbit.example.com/register?token=abc', SERVER)).toEqual({ problem: 'This is not an invitation link.' })
  expect(invitationToken('https://orbit.example.com/accept-invitation', SERVER)).toEqual({ problem: 'This is not an invitation link.' })
  expect(invitationToken('   ', SERVER)).toEqual({ problem: 'Paste the invitation link.' })
  expect(invitationToken('hello there', SERVER)).toEqual({ problem: 'This is not an invitation link.' })
})

test('invitationToken takes a bare token as it is', () => {
  expect(invitationToken('abcdefghijklmnop_123', SERVER)).toEqual({ token: 'abcdefghijklmnop_123' })
})

test('invitationForOther compares emails without case or spaces', () => {
  expect(invitationForOther({ email: ' Ana@Example.com' }, 'ana@example.com')).toBe(false)
  expect(invitationForOther({ email: 'bo@example.com' }, 'ana@example.com')).toBe(true)
})

test('exportPollInterval polls only while an export is queued or running', () => {
  expect(exportPollInterval(undefined)).toBe(false)
  expect(exportPollInterval([{ status: 'ready' }, { status: 'failed' }])).toBe(false)
  expect(exportPollInterval([{ status: 'ready' }, { status: 'running' }])).toBe(3000)
  expect(exportPollInterval([{ status: 'queued' }])).toBe(3000)
})
