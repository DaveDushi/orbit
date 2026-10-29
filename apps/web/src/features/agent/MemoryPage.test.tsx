import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router'
import type { AgentMemoryPage } from '@orbit/api/agent'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { MemoryPage } from './MemoryPage'

const realFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = realFetch
})

const memory = {
  position: 3,
  body: 'Deploys happen on Thursday',
  author_id: 'u1',
  author_name: 'Ann',
  created_at: new Date().toISOString(),
  source: { conversation_id: 'c1', root_message_id: 'm1' },
}

/** The memory routes; `canDelete` is the server's answer for the team memory. */
function serve(canDelete: boolean) {
  const calls: string[] = []
  globalThis.fetch = (async (input: Request | string) => {
    const request = input instanceof Request ? input : new Request(input)
    const url = new URL(request.url)
    calls.push(`${request.method} ${url.pathname}`)
    if (url.pathname === '/api/v1/agent') return Response.json({ enabled: true })
    if (request.method === 'DELETE') return new Response(null, { status: 204 })
    if (url.pathname.endsWith('/wake')) return Response.json({ total: 1, lines: [], text: '' })
    const page: AgentMemoryPage = { items: [memory], next_before: null, can_delete: canDelete }
    return Response.json(page)
  }) as typeof fetch
  return calls
}

function show(role: 'owner' | 'member') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const workspace = testWorkspace(role)
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>
          <MemoryPage />
        </WorkspaceContext.Provider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

test('a member reads team memory with its author and source thread, and cannot delete it', async () => {
  serve(false)
  const view = show('member')
  expect(await view.findByText('Deploys happen on Thursday')).toBeTruthy()
  expect(view.getByRole('link', { name: 'Source thread' }).getAttribute('href')).toBe('/chat/c1/thread/m1')
  expect(view.getByText(/Ann/)).toBeTruthy()
  expect(view.queryAllByRole('button', { name: /Delete memory/ }).length).toBe(0)
})

test('deleting a memory asks first, then calls the server', async () => {
  const calls = serve(true)
  const view = show('owner')
  fireEvent.click(await view.findByRole('button', { name: 'Delete memory #3' }))
  await userEvent.click(await view.findByRole('button', { name: 'Delete' }))
  expect(calls).toContain('DELETE /api/v1/workspaces/workspace-1/agent-memory/team/memories/3')
})
