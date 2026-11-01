import { expect, test } from 'bun:test'
import { AGENTIC_TABS, agenticTabsView } from './settingsNav'

const labels = (view: ReturnType<typeof agenticTabsView>) => view.tabs.map((tab) => tab.label)

test('an instance admin gets every section and stays on the address', () => {
  for (const [path, label] of [['/agentic', 'You'], ['/agentic/', 'You'], ['/agentic/workspace', 'Workspace'], ['/agentic/admin', 'Admin']]) {
    const view = agenticTabsView(AGENTIC_TABS, true, true, path)
    expect(labels(view)).toEqual(['You', 'Workspace', 'Admin'])
    expect(view.current?.label).toBe(label)
    expect(view.redirect).toBeNull()
  }
})

test('anyone else has no Admin tab, and its address goes to the first section', () => {
  const view = agenticTabsView(AGENTIC_TABS, false, true, '/agentic/admin')
  expect(labels(view)).toEqual(['You', 'Workspace'])
  expect(view.redirect).toBe('/agentic')
  expect(agenticTabsView(AGENTIC_TABS, false, true, '/agentic/workspace').redirect).toBeNull()
})

test('with the agent off only You shows, and direct links keep their access rule', () => {
  expect(labels(agenticTabsView(AGENTIC_TABS, true, false, '/agentic'))).toEqual(['You'])
  const tools = agenticTabsView(AGENTIC_TABS, false, false, '/agentic/workspace')
  expect(labels(tools)).toEqual(['You'])
  expect(tools.redirect).toBeNull()
  expect(tools.current?.label).toBe('Workspace')
  expect(agenticTabsView(AGENTIC_TABS, true, false, '/agentic/admin').redirect).toBeNull()
  expect(agenticTabsView(AGENTIC_TABS, false, false, '/agentic/admin').redirect).toBe('/agentic')
})
