import { describe, expect, test } from 'bun:test'
import type { Permission } from '@orbit/api/generated/types.gen'

import { ADMIN_NAV, allowedNav, settingsSections, WORKSPACE_NAV } from './settings-nav'

// the server's `Policy::can` (crates/orbit/src/policy.rs), for the permissions these links use
const ADMIN: Permission[] = ['workspace.update', 'members.manage', 'api_tokens.manage', 'integrations.manage', 'chat.manage', 'apps.manage', 'workspace.export']
const OWNER: Permission[] = [...ADMIN, 'workspace.delete', 'workspace.transfer', 'billing.manage']
const labels = (items: readonly { label: string }[]) => items.map((item) => item.label)

describe('workspace settings links', () => {
  test('an owner sees every page, Billing only in Orbit Cloud', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, OWNER, { cloud: true }))).toEqual(['General', 'Members', 'Billing', 'Emoji', 'Stickers', 'Reports', 'File storage', 'GitHub', 'Agentic', 'API tokens', 'Apps', 'Export', 'Danger zone'])
    expect(labels(allowedNav(WORKSPACE_NAV, OWNER))).not.toContain('Billing')
  })

  test('an admin has no Danger zone', () => {
    expect(labels(allowedNav(WORKSPACE_NAV, ADMIN))).toEqual(['General', 'Members', 'Emoji', 'Stickers', 'Reports', 'File storage', 'GitHub', 'Agentic', 'API tokens', 'Apps', 'Export'])
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

describe('the Settings page sections', () => {
  const view = (sections: ReturnType<typeof settingsSections>) => Object.fromEntries(sections.map((section) => [section.title, labels(section.items)]))
  const ACCOUNT = ['Profile', 'Security', 'Sessions', 'Blocked people', 'Appearance', 'Notifications']

  test('an instance admin who owns the workspace sees Workspace, Account and Admin, without General in Workspace', () => {
    expect(view(settingsSections(OWNER, { agent: true }, false, true))).toEqual({
      Workspace: ['Members', 'Emoji', 'Stickers', 'Reports', 'File storage', 'GitHub', 'Agentic', 'API tokens', 'Apps', 'Export', 'Danger zone'],
      Account: ACCOUNT,
      Admin: ['General', 'Users', 'Email', 'Limits', 'Storage', 'Backups', 'Audit log', 'Background jobs'],
    })
  })

  test('a member sees Workspace without the admin pages, and no Admin section', () => {
    expect(view(settingsSections([], { cloud: true }, false, false))).toEqual({ Workspace: ['Members', 'Billing', 'File storage', 'GitHub', 'Agentic'], Account: ACCOUNT })
  })

  test('a guest sees only Account', () => {
    expect(view(settingsSections([], { cloud: true, agent: true }, true, false))).toEqual({ Account: ACCOUNT })
  })

  test('Admin follows the instance admin flag, not the workspace role', () => {
    expect(view(settingsSections([], {}, true, true)).Admin).toContain('Users')
  })

  test('every page has one row only: Agentic is one page', () => {
    const hrefs = settingsSections(OWNER, { agent: true, cloud: true, improvements: true }, false, true).flatMap((section) => section.items.map((item) => item.href))
    expect(new Set(hrefs).size).toBe(hrefs.length)
    expect(hrefs.filter((href) => href.includes('agentic'))).toEqual(['/settings/agentic'])
  })
})
