import { describe, expect, test } from 'bun:test'
import type { Permission } from '@orbit/api/generated/types.gen'

import { ADMIN_NAV, allowedNav, WORKSPACE_NAV } from './settings-nav'

// the server's `Policy::can` (crates/orbit/src/policy.rs), for the permissions these links use
const ADMIN: Permission[] = ['workspace.update', 'members.manage', 'api_tokens.manage', 'integrations.manage', 'chat.manage', 'apps.manage', 'workspace.export']
const OWNER: Permission[] = [...ADMIN, 'workspace.delete', 'workspace.transfer', 'billing.manage']
const labels = (items: readonly { label: string }[]) => items.map((item) => item.label)

describe('workspace settings links', () => {
  test('an owner sees every page, Billing only in Orbit Cloud', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, OWNER, { cloud: true }))).toEqual(['General', 'Members', 'Billing', 'Emoji', 'Stickers', 'File storage', 'GitHub', 'Agentic', 'API tokens', 'Apps', 'Export', 'Danger zone'])
    expect(labels(allowedNav(WORKSPACE_NAV, OWNER))).not.toContain('Billing')
  })

  test('an admin has no Danger zone', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, ADMIN))).toEqual(['General', 'Members', 'Emoji', 'Stickers', 'File storage', 'GitHub', 'Agentic', 'API tokens', 'Apps', 'Export'])
  })

  test('a member sees no admin pages', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, [], { cloud: true }))).toEqual(['General', 'Members', 'Billing', 'File storage', 'GitHub', 'Agentic'])
  })

  test('a guest sees only the Settings page itself', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, [], { cloud: true, agent: true }, true))).toEqual(['General'])
  })
})

test('Feedback & fixes shows only while Suggest a change is on', () => {
  expect(labels(allowedNav(ADMIN_NAV, []))).not.toContain('Feedback & fixes')
  expect(labels(allowedNav(ADMIN_NAV, [], { improvements: true }))).toContain('Feedback & fixes')
})
