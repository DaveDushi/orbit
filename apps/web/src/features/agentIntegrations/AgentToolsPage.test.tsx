import { afterEach, expect, mock, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import type { AgentIntegration } from '@orbit/api/agentIntegrations'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { AgentToolsPage } from './AgentToolsPage'

const originalFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

const http: AgentIntegration = {
  id: 'i1', slug: 'shop', name: 'Shop', kind: 'http', url: 'https://shop.example.com', auth_header: null, auth_type: 'none', oauth_token_url: null,
  test_path: 'me', graphql_read_path: null, graphql_reads_enabled: false, secret_set: false, allowed_roles: ['owner'], allowed_methods: ['GET'], tools: null, tools_checked_at: null, created_at: 1, updated_at: 1,
}

function serve(items: AgentIntegration[]) {
  const posts: unknown[] = []
  const patches: unknown[] = []
  globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    if (request.method === 'POST' && new URL(request.url).pathname.endsWith('/test')) return Response.json({ ok: true, detail: 'Connected.', integration: http })
    if (request.method === 'POST') { posts.push(await request.json()); return Response.json({ ...http, id: 'new', kind: 'http' }, { status: 201 }) }
    if (request.method === 'PATCH') { patches.push(await request.json()); return Response.json(http) }
    return Response.json({ items, can_keep_secrets: true })
  }) as unknown as typeof fetch
  return { posts, patches }
}

function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  const workspace = testWorkspace('owner')
  return render(<QueryClientProvider client={client}><WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}><AgentToolsPage /></WorkspaceContext.Provider></QueryClientProvider>)
}

const type = (element: Element, value: string) => fireEvent.change(element, { target: { value } })

test('the form opens on demand and sends a value from the collapsed Advanced section', async () => {
  const { posts } = serve([])
  const view = show()
  await view.findByText('No tools yet.')
  expect(view.queryAllByLabelText(/Server URL/)).toHaveLength(0)
  fireEvent.click(view.getByRole('button', { name: 'Add tool' }))
  const details = view.container.querySelector('details') as HTMLDetailsElement
  expect(details.open).toBe(false)
  type(view.getByLabelText(/Name/), ' Cloudflare ')
  type(view.getByLabelText(/Server URL/), 'https://mcp.example.com/mcp')
  type(view.getByLabelText('Secret'), 'token')
  type(details.querySelector('#agent-tool-header') as Element, 'X-Key')
  expect(details.open).toBe(false)
  fireEvent.click(view.getByRole('button', { name: 'Add' }))
  await waitFor(() => expect(posts).toHaveLength(1))
  expect(posts[0]).toEqual({ kind: 'mcp', name: 'Cloudflare', url: 'https://mcp.example.com/mcp', allowed_roles: ['owner', 'admin', 'member'], auth_header: 'X-Key', secret: 'token' })
})

test('the list is compact; Edit shows the GraphQL switch with its notice and keeps Advanced collapsed', async () => {
  const { patches } = serve([{ ...http, graphql_read_path: '/graphql' }])
  const view = show()
  await view.findByText('Shop')
  expect(view.queryAllByRole('switch')).toHaveLength(0)
  fireEvent.click(view.getByRole('button', { name: 'Edit Shop' }))
  expect(view.getByText(/Reads can return customer data to the agent and the thread/)).toBeTruthy()
  expect(view.getByRole('switch', { name: 'Run GraphQL reads without approval' })).toBeTruthy()
  const details = view.container.querySelector('details') as HTMLDetailsElement
  expect(details.open).toBe(false)
  const testPath = details.querySelector('input') as HTMLInputElement
  expect(testPath.value).toBe('me')
  type(testPath, 'shop.json')
  fireEvent.click(view.getByRole('button', { name: 'Save test path' }))
  await waitFor(() => expect(patches).toEqual([{ test_path: 'shop.json' }]))
})

test('the switch stays off and says why when there is no GraphQL path', async () => {
  serve([http])
  const view = show()
  await view.findByText('Shop')
  fireEvent.click(view.getByRole('button', { name: 'Edit Shop' }))
  expect(view.getByText(/Set a GraphQL path under Advanced first/)).toBeTruthy()
  expect(view.getByRole('switch', { name: 'Run GraphQL reads without approval' }).hasAttribute('data-disabled')).toBe(true)
})
