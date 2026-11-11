import { afterEach, expect, mock, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render } from '@testing-library/react'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router'
import { MovedTo } from '@/app/MovedTo'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { AgenticAdmin, AgenticTabs, AgenticWorkspace, AgenticYou } from './AgenticPage'

const originalFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

function Where() {
  const { pathname, search, hash } = useLocation()
  return <p data-testid="where">{pathname}{search}{hash}</p>
}

type Role = 'owner' | 'admin' | 'member'

function show(role: Role, instanceAdmin: boolean, enabled: boolean, path: string) {
  const asked: string[] = []
  globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(new Request(input, init).url).pathname
    asked.push(url)
    if (url === '/api/v1/agent') return Response.json(enabled ? { enabled, online: true, can_connect: true, bot_user_id: 'bot', name: 'Ada', chatgpt: { connected: false, invalid: false, account_id: null, connected_at: null }, login: null } : { enabled })
    if (url === '/api/v1/auth/me') return Response.json({ id: 'u', email: 'a@b.c', display_name: 'A', installation_admin: instanceAdmin, root: instanceAdmin })
    if (url === '/api/v1/admin/agent/model') return Response.json({ model: null, thinking: null, available: null, thinking_levels: [], updated_by_name: null })
    if (url.endsWith('/agent-integrations')) return Response.json({ items: [], can_keep_secrets: true })
    if (url.endsWith('/agent')) return Response.json({ instructions: '', can_edit: role !== 'member', max_chars: 4000, updated_at: null, updated_by_name: null })
    if (url === '/api/v1/oauth/grants') return Response.json({ items: [] })
    return Response.json({ items: [], next_cursor: null, lines: [], can_delete: false })
  }) as unknown as typeof fetch
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const workspace = testWorkspace(role)
  const view = render(
    <QueryClientProvider client={client}>
      <WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>
        <MemoryRouter initialEntries={[path]}>
          <Where />
          <Routes>
            <Route path="settings/agentic" element={<AgenticTabs />}>
              <Route index element={<AgenticYou />} />
              <Route path="workspace" element={<AgenticWorkspace />} />
              <Route path="admin" element={<AgenticAdmin />} />
            </Route>
            <Route path="settings/agent-tools" element={<MovedTo path="/settings/agentic/workspace" />} />
          </Routes>
        </MemoryRouter>
      </WorkspaceContext.Provider>
    </QueryClientProvider>,
  )
  return { view, asked }
}

test('You shows MCP setup to every role, and agent sections only when the agent runs', async () => {
  for (const role of ['owner', 'admin', 'member'] as const) {
    const on = show(role, false, true, '/settings/agentic')
    expect(await on.view.findByText('MCP server')).toBeTruthy()
    expect(await on.view.findByText('ChatGPT connection')).toBeTruthy()
    expect(await on.view.findByText('Memories')).toBeTruthy()
    cleanup()
    const off = show(role, false, false, '/settings/agentic')
    expect(await off.view.findByText('MCP server')).toBeTruthy()
    expect(off.view.queryAllByText('ChatGPT connection')).toHaveLength(0)
    // Bots need no agent: with the agent off, only You and Bots are tabs.
    expect(off.view.queryAllByRole('tab').map((tab) => tab.textContent)).toEqual(['You', 'Bots'])
    cleanup()
  }
})

test('Workspace shows Tools to owners and admins; members read instructions and never call the tools API', async () => {
  for (const role of ['owner', 'admin'] as const) {
    const { view } = show(role, false, true, '/settings/agentic/workspace')
    expect(await view.findByText('Connected services')).toBeTruthy()
    cleanup()
  }
  const { view, asked } = show('member', false, true, '/settings/agentic/workspace')
  expect(await view.findByText('Team instructions')).toBeTruthy()
  expect(view.queryAllByText('Connected services')).toHaveLength(0)
  expect(asked.some((url) => url.endsWith('/agent-integrations'))).toBe(false)
})

test('Admin opens only for an instance admin, whatever the workspace role', async () => {
  const admin = show('member', true, true, '/settings/agentic/admin')
  await admin.view.findByRole('heading', { name: 'Model' })
  expect(admin.view.getByTestId('where').textContent).toBe('/settings/agentic/admin')
  cleanup()
  const other = show('owner', false, true, '/settings/agentic/admin')
  await other.view.findByText('MCP server')
  expect(other.view.getByTestId('where').textContent).toBe('/settings/agentic')
  expect(other.view.queryAllByRole('tab', { name: 'Admin' })).toHaveLength(0)
})

test('an old address keeps its query and hash', async () => {
  const { view } = show('owner', false, true, '/settings/agent-tools?from=app#add')
  await view.findByText('Connected services')
  expect(view.getByTestId('where').textContent).toBe('/settings/agentic/workspace?from=app#add')
})
