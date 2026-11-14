import { afterEach, expect, mock, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { AgentSkills } from '@orbit/api/agentSkills'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { testWorkspace } from '@/test/workspace'
import { SkillsPage } from './SkillsPage'

const originalFetch = globalThis.fetch
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

const listed: AgentSkills = {
  skills: [{ id: 's1', name: 'pdf-forms', description: 'Fill PDF forms.', source: 'zip', source_url: null, agents: ['default'], files: [{ path: 'scripts/fill.sh', size: 7 }], has_scripts: true, size_bytes: 100, created_at: 1, updated_at: 1 }],
  agents: [{ key: 'default', name: 'Donna' }, { key: 'bot-2', name: 'Writer' }],
  sandbox: false,
  max_skills: 100,
}

function serve() {
  const requests: { method: string; path: string; body?: unknown }[] = []
  globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    const path = new URL(request.url).pathname
    const body = request.method === 'PATCH' ? await request.json() : undefined
    requests.push({ method: request.method, path, body })
    if (request.method === 'PATCH') return Response.json({ ...listed.skills[0], ...(body as object) })
    return Response.json(listed)
  }) as unknown as typeof fetch
  return requests
}

function show(role: 'owner' | 'admin' | 'member') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  const workspace = testWorkspace(role)
  return render(<QueryClientProvider client={client}><WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}><SkillsPage /></WorkspaceContext.Provider></QueryClientProvider>)
}

test('members get a note, no admin controls and no request for the skills', async () => {
  const requests = serve()
  const view = show('member')
  expect(view.getByText(/Only owners and admins can see and manage/)).toBeTruthy()
  expect(view.queryAllByRole('button')).toHaveLength(0)
  expect(view.queryAllByRole('checkbox')).toHaveLength(0)
  expect(requests).toHaveLength(0)
})

test('an admin turns a skill on for one more agent, and sees that scripts cannot run without a sandbox', async () => {
  const requests = serve()
  const view = show('admin')
  expect(await view.findByText('pdf-forms')).toBeTruthy()
  expect(view.getByText('Scripts cannot run')).toBeTruthy()
  expect(view.getByText(/no agent sandbox/)).toBeTruthy()
  const group = view.getByRole('group', { name: 'Agents that use pdf-forms' })
  const boxes = group.querySelectorAll('[role="checkbox"]')
  expect(boxes).toHaveLength(2)
  await userEvent.click(boxes[1])
  await waitFor(() => expect(requests.filter((request) => request.method === 'PATCH')).toEqual([{ method: 'PATCH', path: '/api/v1/workspaces/' + testWorkspace('admin').id + '/agent-skills/s1', body: { agents: ['default', 'bot-2'] } }]))
})
