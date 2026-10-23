import { expect, test } from 'bun:test'

import { adminActions, adminBadges, type AdminUser, auditActor, jobLabel, limitText } from './admin-api'

const user = (fields: Partial<AdminUser>): AdminUser => ({
  id: 'u1',
  display_name: 'Ada',
  email: 'ada@example.com',
  admin: false,
  root: false,
  suspended: false,
  two_factor: false,
  workspace_count: 1,
  created_at: '2026-10-01T10:00:00Z',
  ...fields,
})
const root = { id: 'root', root: true }
const admin = { id: 'admin', root: false }

test('nobody manages the root account, their own, or (unless root) another admin', () => {
  expect(adminActions(user({ root: true }), root)).toEqual([])
  expect(adminActions(user({ id: 'admin' }), admin)).toEqual([])
  expect(adminActions(user({ admin: true }), admin)).toEqual([])
})

test('the root user manages admins; a suspended account can only come back', () => {
  expect(adminActions(user({}), root)).toEqual(['make-admin', 'recovery-link', 'suspend'])
  expect(adminActions(user({ admin: true, two_factor: true }), root)).toEqual(['remove-admin', 'recovery-link', 'reset-two-factor', 'suspend'])
  expect(adminActions(user({ suspended: true }), root)).toEqual(['reinstate'])
  expect(adminActions(user({}), admin)).toEqual(['recovery-link', 'suspend'])
})

test('badges, the audit actor and job wording', () => {
  expect(adminBadges(user({ root: true, admin: true, two_factor: true }))).toEqual(['Root', '2FA'])
  expect(adminBadges(user({ admin: true, suspended: true }))).toEqual(['Admin', 'Suspended'])
  expect(auditActor({ actor_id: null, actor_name: null })).toBe('System')
  expect(auditActor({ actor_id: 'u', actor_name: null })).toBe('Unknown account')
  expect(auditActor({ actor_id: 'u', actor_name: 'Ada' })).toBe('Ada')
  expect([jobLabel('notion.scan'), jobLabel('mail.sync'), limitText(null), limitText(4)]).toEqual(['Notion scans', 'mail.sync', 'no limit', '4'])
})
