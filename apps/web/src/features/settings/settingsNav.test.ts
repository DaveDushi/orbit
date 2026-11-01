import { expect, test } from 'bun:test'
import type { Permission } from '@orbit/api/generated/types.gen'
import { ACCOUNT_AGENT_TABS, WORKSPACE_AGENT_TABS, agentTabsView, navItemVisible, type SettingsNavItem } from './settingsNav'

const manager: Permission[] = ['integrations.manage']
const member: Permission[] = []
const labels = (view: ReturnType<typeof agentTabsView>) => view.tabs.map((tab) => tab.label)

test('a manager gets General and Tools, and stays on the tab of the address', () => {
  for (const [path, label] of [['/settings/agent', 'General'], ['/settings/agent/', 'General'], ['/settings/agent/tools', 'Tools']]) {
    const view = agentTabsView(WORKSPACE_AGENT_TABS, manager, true, path)
    expect(labels(view)).toEqual(['General', 'Tools'])
    expect(view.current?.label).toBe(label)
    expect(view.redirect).toBeNull()
  }
})

test('a member gets only General, and the Tools address goes there', () => {
  const view = agentTabsView(WORKSPACE_AGENT_TABS, member, true, '/settings/agent/tools')
  expect(labels(view)).toEqual(['General'])
  expect(view.redirect).toBe('/settings/agent')
  expect(agentTabsView(WORKSPACE_AGENT_TABS, member, true, '/settings/agent').redirect).toBeNull()
})

test('a tab that needs a permission never is the default of someone without it, and the first open tab is', () => {
  const toolsOnly = [{ to: '/a', label: 'A', permission: 'workspace.update' as const }, { to: '/a/b', label: 'B', permission: 'integrations.manage' as const }]
  const view = agentTabsView(toolsOnly, manager, true, '/a')
  expect(labels(view)).toEqual(['B'])
  expect(view.redirect).toBe('/a/b')
})

test('everyone gets both account tabs', () => {
  for (const permissions of [manager, member]) {
    const view = agentTabsView(ACCOUNT_AGENT_TABS, permissions, true, '/profile/agent/memory')
    expect(labels(view)).toEqual(['Settings', 'Memory'])
    expect(view.redirect).toBeNull()
  }
})

test('with the agent off there are no tabs and no redirect: direct links keep answering as before', () => {
  for (const permissions of [manager, member]) {
    for (const path of ['/settings/agent', '/settings/agent/tools']) {
      const view = agentTabsView(WORKSPACE_AGENT_TABS, permissions, false, path)
      expect(view.tabs).toEqual([])
      expect(view.redirect).toBeNull()
    }
  }
})

test('the Agent link shows when the agent runs and at least one tab can be opened', () => {
  const item: SettingsNavItem = { to: '/settings/agent', label: 'Agent', icon: () => null, agent: true, tabs: WORKSPACE_AGENT_TABS }
  expect(navItemVisible(item, { permissions: manager, cloud: false, agent: true })).toBe(true)
  expect(navItemVisible(item, { permissions: member, cloud: false, agent: true })).toBe(true)
  expect(navItemVisible(item, { permissions: manager, cloud: false, agent: false })).toBe(false)
  expect(navItemVisible(item, { permissions: member, cloud: false, agent: false })).toBe(false)
  const locked = { ...item, tabs: [{ to: '/x', label: 'X', permission: 'workspace.delete' as const }] }
  expect(navItemVisible(locked, { permissions: manager, cloud: false, agent: true })).toBe(false)
})
