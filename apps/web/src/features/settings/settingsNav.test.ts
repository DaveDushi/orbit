import { expect, test } from 'bun:test'
import { AGENTIC_TABS, agenticTabsView } from './settingsNav'

const labels = (view: ReturnType<typeof agenticTabsView>) => view.tabs.map((tab) => tab.label)

test('every section shows and stays on its address', () => {
  for (const [path, label] of [['/settings/agentic', 'You'], ['/settings/agentic/', 'You'], ['/settings/agentic/workspace', 'Workspace'], ['/settings/agentic/agents', 'Agents'], ['/settings/agentic/routines', 'Routines'], ['/settings/agentic/skills', 'Skills']]) {
    const view = agenticTabsView(AGENTIC_TABS, true, path)
    expect(labels(view)).toEqual(['You', 'Workspace', 'Agents', 'Routines', 'Skills'])
    expect(view.current?.label).toBe(label)
    expect(view.redirect).toBeNull()
  }
})

test('the old Admin tab goes to Agents and the old Bots tab to You; another unknown address to the first section', () => {
  for (const agent of [true, false]) {
    expect(agenticTabsView(AGENTIC_TABS, agent, '/settings/agentic/admin').redirect).toBe('/settings/agentic/agents')
    expect(agenticTabsView(AGENTIC_TABS, agent, '/settings/agentic/admin/').redirect).toBe('/settings/agentic/agents')
    expect(agenticTabsView(AGENTIC_TABS, agent, '/settings/agentic/bots').redirect).toBe('/settings/agentic')
    expect(agenticTabsView(AGENTIC_TABS, agent, '/settings/agentic/nothing').redirect).toBe('/settings/agentic')
  }
})

test('with the agent off only You shows, and direct links keep their access rule', () => {
  expect(labels(agenticTabsView(AGENTIC_TABS, false, '/settings/agentic'))).toEqual(['You'])
  const tools = agenticTabsView(AGENTIC_TABS, false, '/settings/agentic/workspace')
  expect(labels(tools)).toEqual(['You'])
  expect(tools.redirect).toBeNull()
  expect(tools.current?.label).toBe('Workspace')
})
