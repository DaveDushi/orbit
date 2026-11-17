import { expect, test } from 'bun:test'

import { type DriveNode, moveTargets, shareExpiry, shareState } from './drive'

test('a link is off once revoked, expired once past its date, else active', () => {
  const now = Date.parse('2026-11-17T12:00:00Z')
  expect(shareState({ revoked_at: '2026-11-01T00:00:00Z', expires_at: null }, now)).toEqual({ label: 'Off', active: false })
  expect(shareState({ revoked_at: null, expires_at: '2026-11-17T11:59:59Z' }, now)).toEqual({ label: 'Expired', active: false })
  expect(shareState({ revoked_at: null, expires_at: '2026-11-18T00:00:00Z' }, now).active).toBe(true)
  expect(shareState({ revoked_at: null, expires_at: null }, now).label).toBe('Active')
})

test('a new link expires never or whole days from now', () => {
  const now = Date.parse('2026-11-17T12:00:00Z')
  expect(shareExpiry(null, now)).toBeNull()
  expect(shareExpiry(7, now)).toBe('2026-11-24T12:00:00.000Z')
})

test('a folder never offers itself as the place to move to, and files are no target', () => {
  const nodes = [{ id: 'a', kind: 'folder' }, { id: 'b', kind: 'folder' }, { id: 'f', kind: 'file' }] as DriveNode[]
  expect(moveTargets(nodes, { id: 'a' }).map((node) => node.id)).toEqual(['b'])
})
