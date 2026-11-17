import { expect, test } from 'bun:test'

import { appFormReady, appIconName, redirectLines, sameRoles } from './apps'

test('redirect URIs: one per line, trimmed, no blank lines', () => {
  expect(redirectLines(' https://a/cb \n\n https://b/cb\n')).toEqual(['https://a/cb', 'https://b/cb'])
  expect(redirectLines('  ')).toEqual([])
})

test('a new app needs a name, an address, a role, and with sign-in a redirect URI', () => {
  const form = { name: 'Reports', url: 'https://reports.example.com', roles: ['owner' as const], signIn: false, redirects: '' }
  expect(appFormReady(form)).toBe(true)
  expect(appFormReady({ ...form, roles: [] })).toBe(false)
  expect(appFormReady({ ...form, signIn: true })).toBe(false)
  expect(appFormReady({ ...form, signIn: true, redirects: 'https://reports.example.com/cb' })).toBe(true)
})

test('roles compare without order, and an unknown icon draws category', () => {
  expect(sameRoles(['owner', 'admin'], ['admin', 'owner'])).toBe(true)
  expect(sameRoles(['owner'], ['owner', 'member'])).toBe(false)
  expect(appIconName('chart')).toBe('chart')
  expect(appIconName('rocket-ship')).toBe('category')
})
