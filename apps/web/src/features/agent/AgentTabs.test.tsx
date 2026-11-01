import { afterEach, expect, mock, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render } from '@testing-library/react'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router'
import { MovedTo } from '@/app/MovedTo'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { AgentTabs } from './AgentTabs'

const originalFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

function Where({ name }: { name: string }) {
  const { pathname, search, hash } = useLocation()
  return <p data-testid="where">{name} {pathname}{search}{hash}</p>
}

function show(role: 'owner' | 'member', enabled: boolean, path: string) {
  globalThis.fetch = mock(async () => Response.json({ enabled })) as unknown as typeof fetch
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const workspace = testWorkspace(role)
  return render(
    <QueryClientProvider client={client}>
      <WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="settings/agent" element={<AgentTabs area="settings" />}>
              <Route index element={<Where name="general" />} />
              <Route path="tools" element={<Where name="tools" />} />
            </Route>
            <Route path="settings/agent-tools" element={<MovedTo path="/settings/agent/tools" />} />
          </Routes>
        </MemoryRouter>
      </WorkspaceContext.Provider>
    </QueryClientProvider>,
  )
}

test('an owner sees both tabs', async () => {
  const view = show('owner', true, '/settings/agent/tools')
  expect((await view.findByTestId('where')).textContent).toBe('tools /settings/agent/tools')
  expect(view.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['General', 'Tools'])
})

test('an old address keeps its query and hash and shows the new tab', async () => {
  const view = show('owner', true, '/settings/agent-tools?from=app#add')
  expect((await view.findByTestId('where')).textContent).toBe('tools /settings/agent/tools?from=app#add')
})

test('a member at the Tools address lands on General with no tab bar', async () => {
  const view = show('member', true, '/settings/agent-tools')
  expect((await view.findByTestId('where')).textContent).toBe('general /settings/agent')
  expect(view.queryAllByRole('tab')).toHaveLength(0)
})

test('with the agent off a direct link still opens its page, with no tab bar', async () => {
  const view = show('owner', false, '/settings/agent/tools')
  expect((await view.findByTestId('where')).textContent).toBe('tools /settings/agent/tools')
  expect(view.queryAllByRole('tab')).toHaveLength(0)
})
